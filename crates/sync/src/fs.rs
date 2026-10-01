//! Staging the working tree's current contents into the git index,
//! skipping unchanged files by comparing cached mtime/size rather than
//! re-reading and re-hashing everything on every sync.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use git2::{IndexEntry, IndexTime, Oid, Repository};

/// Walks every non-ignored file under `dir` (relative to `root`), calling
/// `on_file(abs_path, posix_rel)` for each. Skips `.git` entries and
/// anything the repository's `.gitignore` rules would ignore. Used by
/// [`stage_dir`] to avoid repeating the `read_dir` + gitignore-filter
/// skeleton; `remove_unwanted` and the vault's `.tmt` walk keep their own
/// loops because they need to handle directory-level operations or lack a
/// `Repository` reference.
fn walk_working_tree<F>(
    repo: &Repository,
    root: &Path,
    dir: &Path,
    on_file: &mut F,
) -> anyhow::Result<()>
where
    F: FnMut(&Path, &str) -> anyhow::Result<()>,
{
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_name().to_string_lossy() == ".git" {
            continue;
        }
        let rel_path = path.strip_prefix(root)?;
        if repo.status_should_ignore(rel_path)? {
            continue;
        }
        if path.is_dir() {
            walk_working_tree(repo, root, &path, on_file)?;
        } else {
            let rel = relative_posix_path(root, &path)?;
            on_file(&path, &rel)?;
        }
    }
    Ok(())
}

pub(crate) fn stage_dir(
    repo: &Repository,
    index: &mut git2::Index,
    root: &Path,
    dir: &Path,
) -> anyhow::Result<()> {
    let mut seen_paths = BTreeSet::new();

    walk_working_tree(repo, root, dir, &mut |path, rel| {
        seen_paths.insert(rel.to_string());

        let meta = match std::fs::symlink_metadata(path) {
            Ok(m) => m,
            Err(_) => return Ok(()),
        };
        let file_size = meta.len() as u32;
        let mtime = meta
            .modified()
            .map(system_time_to_index_time)
            .unwrap_or((0, 0));

        // Check if existing index entry matches mtime and size
        if let Some(entry) = index.get_path(Path::new(rel), 0) {
            if entry.file_size == file_size
                && entry.mtime.seconds() == mtime.0
                && entry.mtime.nanoseconds() == mtime.1
                && entry.id != Oid::zero()
            {
                // Unchanged: stat cache matches, no need to read file contents or re-hash
                return Ok(());
            }
        }

        let data = std::fs::read(path)?;
        stage_bytes(index, rel, &data, mtime)
    })?;

    // Remove any entries from index that no longer exist in working tree
    let mut paths_to_remove = Vec::new();
    for i in 0..index.len() {
        if let Some(entry) = index.get(i) {
            if let Ok(path_str) = std::str::from_utf8(&entry.path) {
                if !seen_paths.contains(path_str) {
                    paths_to_remove.push(PathBuf::from(path_str));
                }
            }
        }
    }
    for p in paths_to_remove {
        let _ = index.remove_path(&p);
    }

    Ok(())
}

pub(crate) fn stage_bytes(
    index: &mut git2::Index,
    path: &str,
    data: &[u8],
    mtime: (i32, u32),
) -> anyhow::Result<()> {
    let entry = IndexEntry {
        ctime: IndexTime::new(mtime.0, mtime.1),
        mtime: IndexTime::new(mtime.0, mtime.1),
        dev: 0,
        ino: 0,
        mode: 0o100644,
        uid: 0,
        gid: 0,
        file_size: data.len() as u32,
        id: Oid::zero(),
        flags: 0,
        flags_extended: 0,
        path: path.as_bytes().to_vec(),
    };
    index.add_frombuffer(&entry, data)?;
    Ok(())
}

fn system_time_to_index_time(t: std::time::SystemTime) -> (i32, u32) {
    match t.duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => (d.as_secs() as i32, d.subsec_nanos()),
        Err(e) => (
            -(e.duration().as_secs() as i32),
            e.duration().subsec_nanos(),
        ),
    }
}

fn relative_posix_path(root: &Path, path: &Path) -> anyhow::Result<String> {
    Ok(path
        .strip_prefix(root)?
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/"))
}
