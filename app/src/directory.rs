//! Builds the Library "Directory" page's folder-at-a-time browsing out of
//! the same flat, slash-separated display titles the old flat list already
//! used (e.g. `work/todo`) -- no new path parsing or sorting of the vault
//! itself. List and Grid are just two renderings of the same
//! [`build_folder_entries`] output (rows vs. tiles); a first cut also had a
//! Tree view, dropped as unworkable on a phone-sized screen.
//!
//! Entries still only ever hand back a `note_index` into
//! `Session.notes`/`Session.conflicted` -- the index contract every other
//! note action (`select`, `note-menu-requested`, rename, delete) already
//! relies on. Folders have no index of their own: they're derived purely
//! from splitting titles on `/`, so they can't be renamed or deleted as
//! their own entity.

/// One folder or note directly inside whichever folder is being browsed.
/// `note_index` is `-1` for a folder entry; `key` is the folder's full
/// slash-joined title prefix (e.g. `work/sub`) and is empty for a note
/// entry, since a note is identified by `note_index` instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryEntry {
    pub is_folder: bool,
    pub key: String,
    pub display: String,
    pub note_index: i32,
    pub has_conflict: bool,
}

/// `(display title, index into Session.notes/conflicted, has_conflict)`,
/// expected sorted alphabetically by title -- callers building this from
/// search hits (ranked by relevance, not path) need to sort it themselves
/// first, since [`build_folder_entries`] assumes a folder's notes all
/// appear as a contiguous run.
pub type TitledNote<'a> = (&'a str, i32, bool);

fn join_key(parent: &str, child: &str) -> String {
    if parent.is_empty() {
        child.to_string()
    } else {
        format!("{parent}/{child}")
    }
}

/// Builds the direct children of `current_folder` only (its subfolders,
/// deduplicated, then its own notes), both sorted alphabetically.
/// `current_folder` is `""` at the vault root.
pub fn build_folder_entries(notes: &[TitledNote], current_folder: &str) -> Vec<DirectoryEntry> {
    let prefix = if current_folder.is_empty() {
        String::new()
    } else {
        format!("{current_folder}/")
    };

    let mut folder_names: Vec<String> = Vec::new();
    let mut note_rows: Vec<DirectoryEntry> = Vec::new();

    for &(title, note_index, has_conflict) in notes {
        let Some(remainder) = title.strip_prefix(prefix.as_str()) else {
            continue;
        };
        match remainder.split_once('/') {
            Some((child, _rest)) => {
                if !folder_names.iter().any(|f| f == child) {
                    folder_names.push(child.to_string());
                }
            }
            None => note_rows.push(DirectoryEntry {
                is_folder: false,
                key: String::new(),
                display: remainder.to_string(),
                note_index,
                has_conflict,
            }),
        }
    }

    folder_names.sort();
    let mut entries: Vec<DirectoryEntry> = folder_names
        .into_iter()
        .map(|name| {
            let key = join_key(current_folder, &name);
            DirectoryEntry {
                is_folder: true,
                key,
                display: name,
                note_index: -1,
                has_conflict: false,
            }
        })
        .collect();
    entries.extend(note_rows);
    entries
}

/// One tappable segment of the breadcrumb trail above the browsed folder's
/// contents -- `key` is what a tap navigates to (the same shape
/// [`build_folder_entries`]'s `current_folder` takes), `display` is just
/// that segment's own name. The first entry is always the vault root
/// itself (`key`/`display` both `""` -- the only way back to it, now that
/// there's no separate always-present root button) followed by each real
/// ancestor folder in turn. Empty at the vault root (nothing to show; you
/// are already there).
pub fn build_breadcrumb(current_folder: &str) -> Vec<(String, String)> {
    if current_folder.is_empty() {
        return Vec::new();
    }
    let mut crumbs = vec![(String::new(), String::new())];
    let mut key = String::new();
    for segment in current_folder.split('/') {
        key = join_key(&key, segment);
        crumbs.push((key.clone(), segment.to_string()));
    }
    crumbs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(
        is_folder: bool,
        key: &str,
        display: &str,
        note_index: i32,
        has_conflict: bool,
    ) -> DirectoryEntry {
        DirectoryEntry {
            is_folder,
            key: key.to_string(),
            display: display.to_string(),
            note_index,
            has_conflict,
        }
    }

    #[test]
    fn folder_entries_at_root_list_top_level_folders_then_notes() {
        let notes: Vec<TitledNote> = vec![
            ("grocery", 0, false),
            ("work/sub/deep", 1, false),
            ("work/todo", 2, true),
            ("zz-last", 3, false),
        ];
        let rows = build_folder_entries(&notes, "");
        assert_eq!(
            rows,
            vec![
                entry(true, "work", "work", -1, false),
                entry(false, "", "grocery", 0, false),
                entry(false, "", "zz-last", 3, false),
            ]
        );
    }

    #[test]
    fn folder_entries_descend_into_a_folder() {
        let notes: Vec<TitledNote> = vec![
            ("grocery", 0, false),
            ("work/sub/deep", 1, false),
            ("work/todo", 2, true),
        ];
        let rows = build_folder_entries(&notes, "work");
        assert_eq!(
            rows,
            vec![
                entry(true, "work/sub", "sub", -1, false),
                entry(false, "", "todo", 2, true),
            ]
        );
    }

    #[test]
    fn folder_entries_deduplicate_a_folder_with_multiple_notes() {
        let notes: Vec<TitledNote> = vec![("work/a", 0, false), ("work/b", 1, false)];
        let rows = build_folder_entries(&notes, "");
        assert_eq!(rows, vec![entry(true, "work", "work", -1, false)]);
    }

    #[test]
    fn breadcrumb_is_empty_at_root() {
        assert_eq!(build_breadcrumb(""), Vec::<(String, String)>::new());
    }

    #[test]
    fn breadcrumb_lists_the_root_then_each_ancestor_with_its_cumulative_key() {
        assert_eq!(
            build_breadcrumb("work/sub/deep"),
            vec![
                ("".to_string(), "".to_string()),
                ("work".to_string(), "work".to_string()),
                ("work/sub".to_string(), "sub".to_string()),
                ("work/sub/deep".to_string(), "deep".to_string()),
            ]
        );
    }
}
