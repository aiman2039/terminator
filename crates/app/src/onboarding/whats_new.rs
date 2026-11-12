//! The bundled changelog and the section for one version.

/// CHANGELOG.md as built into this binary.
pub(super) const CHANGELOG: &str = include_str!("../../../../CHANGELOG.md");

/// The body of `## <version>` (also `## v<version>` or `## [<version>]`).
/// A build whose version has no section yet shows the first section,
/// which holds the newest changes (`## Unreleased` before a release).
pub(crate) fn changelog_section(changelog: &str, version: &str) -> String {
    let sections = sections(changelog);
    let wanted = [
        version.to_owned(),
        format!("v{version}"),
        format!("[{version}]"),
    ];
    sections
        .iter()
        .find(|(title, _)| {
            let first = title.split_whitespace().next().unwrap_or_default();
            wanted.iter().any(|name| name == first)
        })
        .or_else(|| sections.first())
        .map_or_else(String::new, |(_, body)| body.trim().to_owned())
}

/// `(title, body)` for each `## ` heading.
fn sections(changelog: &str) -> Vec<(&str, String)> {
    let mut found: Vec<(&str, String)> = Vec::new();
    for line in changelog.lines() {
        if let Some(title) = line.strip_prefix("## ") {
            found.push((title.trim(), String::new()));
        } else if let Some((_, body)) = found.last_mut() {
            body.push_str(line);
            body.push('\n');
        }
    }
    found
}
