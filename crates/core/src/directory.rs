//! Builds the Library "Directory" page's folder-at-a-time browsing out of
//! the same flat, slash-separated display titles the old flat list already
//! used (e.g. `work/todo`) -- no new path parsing or sorting of the vault
//! itself. List and Grid are just two renderings of the same
//! [`build_folder_entries`] output (rows vs. tiles).
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BreadcrumbSegment {
    pub key: String,
    pub display: String,
}

pub fn build_breadcrumb(current_folder: &str) -> Vec<BreadcrumbSegment> {
    if current_folder.is_empty() {
        return Vec::new();
    }
    let mut segments = vec![BreadcrumbSegment {
        key: String::new(),
        display: String::new(),
    }];
    let mut accum = String::new();
    for part in current_folder.split('/') {
        if part.is_empty() {
            continue;
        }
        if !accum.is_empty() {
            accum.push('/');
        }
        accum.push_str(part);
        segments.push(BreadcrumbSegment {
            key: accum.clone(),
            display: part.to_string(),
        });
    }
    segments
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_folder_lists_subfolders_and_direct_notes() {
        let notes: Vec<TitledNote> = vec![
            ("inbox/first", 0, false),
            ("inbox/second", 1, false),
            ("root_note", 2, true),
            ("work/project/todo", 3, false),
        ];
        let entries = build_folder_entries(&notes, "");
        assert_eq!(
            entries,
            vec![
                DirectoryEntry {
                    is_folder: true,
                    key: "inbox".into(),
                    display: "inbox".into(),
                    note_index: -1,
                    has_conflict: false,
                },
                DirectoryEntry {
                    is_folder: true,
                    key: "work".into(),
                    display: "work".into(),
                    note_index: -1,
                    has_conflict: false,
                },
                DirectoryEntry {
                    is_folder: false,
                    key: String::new(),
                    display: "root_note".into(),
                    note_index: 2,
                    has_conflict: true,
                },
            ]
        );
    }

    #[test]
    fn nested_folder_lists_only_its_direct_children() {
        let notes: Vec<TitledNote> = vec![
            ("work/a", 0, false),
            ("work/project/alpha", 1, false),
            ("work/project/beta", 2, false),
        ];
        let entries = build_folder_entries(&notes, "work");
        assert_eq!(
            entries,
            vec![
                DirectoryEntry {
                    is_folder: true,
                    key: "work/project".into(),
                    display: "project".into(),
                    note_index: -1,
                    has_conflict: false,
                },
                DirectoryEntry {
                    is_folder: false,
                    key: String::new(),
                    display: "a".into(),
                    note_index: 0,
                    has_conflict: false,
                },
            ]
        );
    }

    #[test]
    fn breadcrumb_builds_root_and_chain() {
        assert_eq!(build_breadcrumb(""), Vec::<BreadcrumbSegment>::new());
        assert_eq!(
            build_breadcrumb("work"),
            vec![
                BreadcrumbSegment {
                    key: String::new(),
                    display: String::new(),
                },
                BreadcrumbSegment {
                    key: "work".into(),
                    display: "work".into(),
                },
            ]
        );
        assert_eq!(
            build_breadcrumb("work/project/sub"),
            vec![
                BreadcrumbSegment {
                    key: String::new(),
                    display: String::new(),
                },
                BreadcrumbSegment {
                    key: "work".into(),
                    display: "work".into(),
                },
                BreadcrumbSegment {
                    key: "work/project".into(),
                    display: "project".into(),
                },
                BreadcrumbSegment {
                    key: "work/project/sub".into(),
                    display: "sub".into(),
                },
            ]
        );
    }
}
