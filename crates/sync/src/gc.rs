//! Packing loose objects into a packfile, and the cheap shard-sampling
//! heuristic that decides when it's worth doing.

use std::io::Write;
use std::path::Path;

use git2::{Buf, Indexer};

use crate::Vault;

/// Shard directory inside `.git/objects/` sampled to decide whether auto-GC is needed.
pub(crate) const AUTO_GC_SAMPLE_SHARD: &str = "17";
/// If the sampled shard has at least this many files (~4 * 256 ≈ 1,000 loose objects total),
/// `should_auto_gc` returns true.
pub(crate) const AUTO_GC_SHARD_THRESHOLD: usize = 4;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GcReport {
    /// Number of objects packed into the new packfile.
    pub packed_objects: usize,
    /// Number of loose object files pruned from disk.
    pub pruned_objects: usize,
}

impl Vault {
    /// Checks whether loose objects in the repository have accumulated beyond
    /// the threshold where packing is recommended.
    ///
    /// Uses Git's standard heuristic of sampling a single 2-hex shard directory
    /// (`objects/17/`) rather than scanning all 256 shards, keeping this check
    /// sub-millisecond during regular sync cycles.
    pub fn should_auto_gc(&self) -> bool {
        let sample_dir = self.repo.path().join("objects").join(AUTO_GC_SAMPLE_SHARD);
        match std::fs::read_dir(sample_dir) {
            Ok(entries) => {
                let count = entries.flatten().filter(|e| e.path().is_file()).count();
                count >= AUTO_GC_SHARD_THRESHOLD
            }
            Err(_) => false,
        }
    }

    /// Packs all reachable objects into a single packfile in `objects/pack/`
    /// and prunes loose object files from disk.
    ///
    /// Safe to call at any time; if the repository has no objects or refs,
    /// returns an empty [`GcReport`] without writing files.
    pub fn gc(&self) -> anyhow::Result<GcReport> {
        let mut pb = self.repo.packbuilder()?;
        let mut revwalk = self.repo.revwalk()?;
        let _ = revwalk.push_glob("refs/*");
        pb.insert_walk(&mut revwalk)?;

        let object_count = pb.object_count();
        if object_count == 0 {
            return Ok(GcReport::default());
        }

        let mut buf = Buf::new();
        pb.write_buf(&mut buf)?;

        let odb = self.repo.odb()?;
        let pack_dir = self.repo.path().join("objects").join("pack");
        std::fs::create_dir_all(&pack_dir)?;
        let mut indexer = Indexer::new(Some(&odb), &pack_dir, 0o644, true)?;
        indexer.write_all(&buf)?;
        let _pack_name = indexer.commit()?;

        let pruned_objects = prune_loose_objects(&self.repo.path().join("objects"))?;

        Ok(GcReport {
            packed_objects: object_count,
            pruned_objects,
        })
    }
}

fn prune_loose_objects(objects_dir: &Path) -> anyhow::Result<usize> {
    let mut pruned = 0;
    if !objects_dir.exists() {
        return Ok(0);
    }
    for entry in std::fs::read_dir(objects_dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.len() == 2
            && name_str.chars().all(|c| c.is_ascii_hexdigit())
            && entry.path().is_dir()
        {
            let shard_dir = entry.path();
            if let Ok(shard_entries) = std::fs::read_dir(&shard_dir) {
                for file_entry in shard_entries.flatten() {
                    let file_path = file_entry.path();
                    if file_path.is_file() {
                        if std::fs::remove_file(&file_path).is_ok() {
                            pruned += 1;
                        }
                    }
                }
            }
            let _ = std::fs::remove_dir(&shard_dir);
        }
    }
    Ok(pruned)
}
