//! Porcelain status: change records, grouping, and decorations.
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug)]
pub struct Change {
    pub path: PathBuf,
    pub status: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitGroup {
    Conflicts,
    Staged,
    Changes,
    Untracked,
}

impl GitGroup {
    pub const ALL: [Self; 4] = [
        Self::Conflicts,
        Self::Staged,
        Self::Changes,
        Self::Untracked,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Conflicts => "Conflicts",
            Self::Staged => "Staged Changes",
            Self::Changes => "Changes",
            Self::Untracked => "Untracked",
        }
    }
}

impl Change {
    pub fn conflict(&self) -> bool {
        matches!(
            self.status.as_str(),
            "DD" | "AU" | "UD" | "UA" | "DU" | "AA" | "UU"
        )
    }

    /// Explorer precedence: conflicts, working tree, then index.
    pub fn decoration(&self) -> char {
        if self.conflict() {
            return '!';
        }
        if self.status == "??" {
            return 'U';
        }
        let bytes = self.status.as_bytes();
        if bytes.len() != 2 {
            return ' ';
        }
        let (Some(&index), Some(&worktree)) = (bytes.first(), bytes.get(1)) else {
            return ' ';
        };
        if worktree == b' ' {
            index as char
        } else {
            worktree as char
        }
    }

    pub fn in_group(&self, group: GitGroup) -> bool {
        let bytes = self.status.as_bytes();
        if bytes.len() != 2 {
            return false;
        }
        let (Some(&index), Some(&worktree)) = (bytes.first(), bytes.get(1)) else {
            return false;
        };
        match group {
            GitGroup::Conflicts => self.conflict(),
            GitGroup::Staged => !self.conflict() && !matches!(index, b' ' | b'?' | b'!'),
            GitGroup::Changes => !self.conflict() && !matches!(worktree, b' ' | b'?' | b'!'),
            GitGroup::Untracked => self.status == "??",
        }
    }

    pub fn letter(&self, group: GitGroup) -> char {
        if self.conflict() {
            '!'
        } else if self.status == "??" {
            'U'
        } else if group == GitGroup::Staged {
            self.status.chars().next().unwrap_or(' ')
        } else {
            self.status.chars().nth(1).unwrap_or(' ')
        }
    }

    /// Working-tree side wins when both exist. Conflicts are not auto-reviewed.
    #[cfg(test)]
    pub fn default_staged(&self) -> Option<bool> {
        if self.conflict() {
            None
        } else if self.in_group(GitGroup::Changes) || self.in_group(GitGroup::Untracked) {
            Some(false)
        } else if self.in_group(GitGroup::Staged) {
            Some(true)
        } else {
            None
        }
    }
}

/// Parse `status --porcelain=v1 -z` output. Rename and copy records carry
/// their origin path in the following field; the target path wins.
pub fn parse_porcelain(root: &Path, raw: &[u8]) -> Vec<Change> {
    let mut changes = Vec::new();
    let mut parts = raw.split(|b| *b == 0);
    while let Some(part) = parts.next() {
        if part.len() < 4 {
            continue;
        }
        let (Some(status_bytes), Some(path_bytes)) = (part.get(..2), part.get(3..)) else {
            continue;
        };
        let status = String::from_utf8_lossy(status_bytes).into_owned();
        let path = root.join(terminator_core::os_name(path_bytes));
        if status.contains('R') || status.contains('C') {
            let _ = parts.next();
        }
        changes.push(Change { path, status });
    }
    changes
}

/// Build folder and file lookups once on the refresh worker. The total ordering
/// makes folder badges independent of Git output order.
pub fn decorations(root: &Path, changes: &[Change]) -> HashMap<PathBuf, char> {
    let mut result = HashMap::new();
    for change in changes {
        let status = change.decoration();
        for path in change.path.ancestors().take_while(|p| p.starts_with(root)) {
            let entry = result.entry(path.to_path_buf()).or_insert(status);
            if decoration_priority(status) > decoration_priority(*entry) {
                *entry = status;
            }
        }
    }
    result
}

fn decoration_priority(status: char) -> u8 {
    match status {
        '!' => 8,
        'D' => 7,
        'M' => 6,
        'R' => 5,
        'C' => 4,
        'A' => 3,
        'T' => 2,
        'U' => 1,
        _ => 0,
    }
}

pub fn status_description(status: char) -> &'static str {
    match status {
        '!' => "Conflict",
        'D' => "Deleted",
        'M' => "Modified",
        'R' => "Renamed",
        'C' => "Copied",
        'A' => "Added",
        'T' => "Type changed",
        'U' => "Untracked",
        _ => "Unchanged",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explorer_status_precedence_and_folder_aggregation_are_deterministic() {
        for (status, expected) in [
            ("??", 'U'),
            ("AM", 'M'),
            ("MD", 'D'),
            ("M ", 'M'),
            ("R ", 'R'),
            ("UU", '!'),
            ("AA", '!'),
            ("DU", '!'),
        ] {
            assert_eq!(
                Change {
                    path: "/repo/a".into(),
                    status: status.into()
                }
                .decoration(),
                expected
            );
        }
        let mut changes = vec![
            Change {
                path: "/repo/new/file".into(),
                status: "??".into(),
            },
            Change {
                path: "/repo/new/deleted".into(),
                status: " D".into(),
            },
            Change {
                path: "/repo/conflict".into(),
                status: "UU".into(),
            },
        ];
        let first = decorations(Path::new("/repo"), &changes);
        changes.reverse();
        assert_eq!(first, decorations(Path::new("/repo"), &changes));
        assert_eq!(first[Path::new("/repo")], '!');
        assert_eq!(first[Path::new("/repo/new")], 'D');
        assert_eq!(first[Path::new("/repo/new/file")], 'U');
        assert!(!first.contains_key(Path::new("/")));
    }

    #[test]
    fn partially_staged_and_conflicts_are_grouped_correctly() {
        let change = Change {
            path: "file".into(),
            status: "MM".into(),
        };
        assert!(change.in_group(GitGroup::Staged));
        assert!(change.in_group(GitGroup::Changes));
        assert!(!change.in_group(GitGroup::Conflicts));
        for status in ["DD", "AU", "UD", "UA", "DU", "AA", "UU"] {
            let change = Change {
                path: "file".into(),
                status: status.into(),
            };
            assert!(change.in_group(GitGroup::Conflicts));
            assert!(!change.in_group(GitGroup::Staged));
            assert!(!change.in_group(GitGroup::Changes));
        }
        assert!(
            Change {
                path: "file".into(),
                status: "??".into()
            }
            .in_group(GitGroup::Untracked)
        );
    }

    #[test]
    fn default_staged_prefers_the_working_tree() {
        let working = Change {
            path: "a.rs".into(),
            status: "MM".into(),
        };
        assert_eq!(working.default_staged(), Some(false));
        let staged_only = Change {
            path: "a.rs".into(),
            status: "M ".into(),
        };
        assert_eq!(staged_only.default_staged(), Some(true));
        let untracked = Change {
            path: "a.rs".into(),
            status: "??".into(),
        };
        assert_eq!(untracked.default_staged(), Some(false));
        let conflict = Change {
            path: "a.rs".into(),
            status: "UU".into(),
        };
        assert_eq!(conflict.default_staged(), None);
    }

    #[test]
    fn porcelain_renames_keep_the_target_path_and_skip_the_origin() {
        let root = Path::new("/repo");
        let raw = b"M  kept.rs\0R  new.rs\0old.rs\0?? fresh.rs\0";
        let changes = parse_porcelain(root, raw);
        assert_eq!(
            changes
                .iter()
                .map(|c| (c.path.clone(), c.status.clone()))
                .collect::<Vec<_>>(),
            vec![
                (PathBuf::from("/repo/kept.rs"), "M ".to_string()),
                (PathBuf::from("/repo/new.rs"), "R ".to_string()),
                (PathBuf::from("/repo/fresh.rs"), "??".to_string()),
            ]
        );
        assert!(changes[1].in_group(GitGroup::Staged));
    }
}
