//! Explorer file nesting (`IntelliJ` "File Nesting"): generated or lock
//! files show under the file they come from, for example `Cargo.lock`
//! under `Cargo.toml` and `app.js` under `app.ts`.
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use terminator_git::Entry;

/// Whole-name rules: parent name and the names nested under it.
const NAME_RULES: &[(&str, &[&str])] = &[
    ("Cargo.toml", &["Cargo.lock"]),
    (
        "package.json",
        &[
            "package-lock.json",
            "npm-shrinkwrap.json",
            "yarn.lock",
            "pnpm-lock.yaml",
            "bun.lockb",
        ],
    ),
    ("pyproject.toml", &["poetry.lock", "uv.lock", "pdm.lock"]),
    ("Pipfile", &["Pipfile.lock"]),
    ("go.mod", &["go.sum"]),
    ("Gemfile", &["Gemfile.lock"]),
    ("composer.json", &["composer.lock"]),
];

/// Suffix rules: a file `<stem><child>` nests under `<stem><parent>`.
const SUFFIX_RULES: &[(&str, &[&str])] = &[
    (".ts", &[".js", ".d.ts", ".js.map", ".d.ts.map"]),
    (".tsx", &[".js", ".jsx", ".js.map"]),
    (".js", &[".js.map", ".min.js", ".min.js.map"]),
    (".scss", &[".css", ".css.map", ".min.css"]),
    (".sass", &[".css", ".css.map"]),
    (".less", &[".css", ".css.map"]),
    (".css", &[".css.map", ".min.css"]),
];

/// The sibling name `name` nests under, if that sibling exists.
fn direct_parent(name: &str, siblings: &HashSet<&str>) -> Option<String> {
    for (parent, children) in NAME_RULES {
        if children.contains(&name) && siblings.contains(parent) {
            return Some((*parent).to_owned());
        }
    }
    for (parent, children) in SUFFIX_RULES {
        for child in *children {
            let Some(stem) = name.strip_suffix(child).filter(|stem| !stem.is_empty()) else {
                continue;
            };
            let candidate = format!("{stem}{parent}");
            if candidate != name && siblings.contains(candidate.as_str()) {
                return Some(candidate);
            }
        }
    }
    None
}

/// Split one listing into top-level entries and the files nested under
/// each parent file (keyed by the parent's path), keeping listing order.
pub(super) fn group(entries: Vec<&Entry>) -> (Vec<&Entry>, HashMap<PathBuf, Vec<&Entry>>) {
    let name = |entry: &Entry| {
        entry
            .path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    let names: Vec<String> = entries.iter().map(|entry| name(entry)).collect();
    let files: HashSet<&str> = entries
        .iter()
        .zip(&names)
        .filter(|(entry, _)| !entry.directory)
        .map(|(_, name)| name.as_str())
        .collect();
    let by_name: HashMap<&str, &Entry> = entries
        .iter()
        .zip(&names)
        .map(|(entry, name)| (name.as_str(), *entry))
        .collect();
    let mut top = Vec::new();
    let mut nested: HashMap<PathBuf, Vec<&Entry>> = HashMap::new();
    for (entry, own) in entries.iter().zip(&names) {
        // Follow chains (`a.js.map` → `a.js` → `a.ts`) to the top file.
        let mut root: Option<String> = None;
        let mut at = own.clone();
        for _ in 0..4 {
            let Some(parent) = direct_parent(&at, &files) else {
                break;
            };
            at.clone_from(&parent);
            root = Some(parent);
        }
        match root.and_then(|root| by_name.get(root.as_str()).copied()) {
            Some(parent) if !entry.directory => {
                nested.entry(parent.path.clone()).or_default().push(entry);
            }
            _ => top.push(*entry),
        }
    }
    (top, nested)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_and_generated_files_nest_under_their_source() {
        let entries: Vec<Entry> = [
            "Cargo.lock",
            "Cargo.toml",
            "app.js",
            "app.js.map",
            "app.ts",
            "lone.js",
            "lone.js.map",
            "x.d.ts",
        ]
        .iter()
        .map(|name| Entry {
            path: PathBuf::from("/r").join(name),
            directory: false,
            ignored: false,
        })
        .collect();
        let (top, nested) = group(entries.iter().collect());
        let names = |list: &[&Entry]| {
            list.iter()
                .map(|entry| {
                    entry
                        .path
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                        .into_owned()
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(names(&top), ["Cargo.toml", "app.ts", "lone.js", "x.d.ts"]);
        assert_eq!(
            names(&nested[&PathBuf::from("/r/Cargo.toml")]),
            ["Cargo.lock"]
        );
        assert_eq!(
            names(&nested[&PathBuf::from("/r/app.ts")]),
            ["app.js", "app.js.map"]
        );
        assert_eq!(
            names(&nested[&PathBuf::from("/r/lone.js")]),
            ["lone.js.map"]
        );
    }
}
