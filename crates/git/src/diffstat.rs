//! `git diff --numstat` parsing: per-path added/deleted line counts.
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

/// Added and deleted line counts for one path.
pub type Stat = (u32, u32);

/// Parse NUL-delimited `--numstat -z` output. Rename/copy records carry the
/// new path as the following NUL field, and binary files report `-`.
pub fn parse_numstat(root: &Path, raw: &[u8]) -> HashMap<PathBuf, Stat> {
    let mut stats = HashMap::new();
    let mut parts = raw.split(|b| *b == 0);
    while let Some(part) = parts.next() {
        if part.is_empty() {
            continue;
        }
        let text = String::from_utf8_lossy(part);
        let mut fields = text.splitn(3, '\t');
        let added = fields.next().unwrap_or("");
        let deleted = fields.next().unwrap_or("");
        let rest = fields.next().unwrap_or("");
        let path = if rest.is_empty() {
            // Rename/copy output is `add\tdel\t\0old\0new\0`; keep the
            // destination (the second following field).
            let Some(_old) = parts.next() else {
                continue;
            };
            let Some(new) = parts.next() else {
                continue;
            };
            root.join(terminator_core::os_name(new))
        } else {
            root.join(terminator_core::os_name(rest.as_bytes()))
        };
        let added = added.parse::<u32>().ok();
        let deleted = deleted.parse::<u32>().ok();
        if let (Some(added), Some(deleted)) = (added, deleted) {
            stats.insert(path, (added, deleted));
        }
    }
    stats
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_added_deleted_and_skips_binary() {
        let root = Path::new("/repo");
        let raw = b"4\t2\tsrc/a.rs\0-\t-\tlogo.png\0";
        let stats = parse_numstat(root, raw);
        assert_eq!(stats.get(&root.join("src/a.rs")), Some(&(4, 2)));
        assert!(!stats.contains_key(&root.join("logo.png")));
    }

    #[test]
    fn rename_uses_the_destination_path() {
        let root = Path::new("/repo");
        let raw = b"1\t0\t\0old/name.rs\0new/name.rs\0";
        let stats = parse_numstat(root, raw);
        assert_eq!(stats.get(&root.join("new/name.rs")), Some(&(1, 0)));
        assert!(!stats.contains_key(&root.join("old/name.rs")));
    }
}
