//! `ui-preferences.json` in the data folder: the GUI's navigation and
//! sidebar preferences. The GUI writes it; other programs only read the
//! project trust answer from it.
use std::path::Path;

/// The file name in the data folder.
pub const FILE: &str = "ui-preferences.json";
/// The key of the project trust answers (project folder: trusted or not),
/// which language servers, project tasks and flows share.
pub const TRUST_KEY: &str = "lsp_trust";

/// The trust answer the user gave in the GUI for the project folder
/// `root`, if any. Blocking: call it off the GUI thread.
pub fn trust_answer(data: &Path, root: &Path) -> Option<bool> {
    let text = std::fs::read(data.join(FILE)).ok()?;
    let preferences: serde_json::Value = serde_json::from_slice(&text).ok()?;
    preferences
        .get(TRUST_KEY)?
        .get(root.to_string_lossy().as_ref())?
        .as_bool()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trust_answer_is_read_only_for_its_own_folder() {
        let dir = tempfile::tempdir().unwrap();
        let root = Path::new("/src/app");
        assert_eq!(trust_answer(dir.path(), root), None);
        std::fs::write(
            dir.path().join(FILE),
            r#"{"lsp_trust":{"/src/app":true,"/src/other":false}}"#,
        )
        .unwrap();
        assert_eq!(trust_answer(dir.path(), root), Some(true));
        assert_eq!(
            trust_answer(dir.path(), Path::new("/src/other")),
            Some(false)
        );
        assert_eq!(trust_answer(dir.path(), Path::new("/src/new")), None);
    }
}
