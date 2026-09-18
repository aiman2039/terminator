//! Source-level guardrails; dependency implementation auditing remains separate.
use anyhow::{Result, ensure};
use std::{fs, path::Path};
pub fn run() -> Result<()> {
    let root = crate::harness::root();
    let mut violations = Vec::new();
    for directory in [root.join("crates/app/src"), root.join("crates/core/src")] {
        scan(&directory, &mut violations)?;
    }
    ensure!(
        violations.is_empty(),
        "Disallowed async boundary:\n{}",
        violations.join("\n")
    );
    println!(
        "Async boundary source check passed; transitive library paths require a separate audit."
    );
    Ok(())
}
fn scan(directory: &Path, violations: &mut Vec<String>) -> Result<()> {
    for entry in fs::read_dir(directory)? {
        let path = entry?.path();
        if path.is_dir() {
            scan(&path, violations)?;
        } else if path.extension().is_some_and(|e| e == "rs") {
            let source = fs::read_to_string(&path)?;
            // Removing whitespace also catches qualified calls split across lines.
            let compact: String = source.chars().filter(|c| !c.is_whitespace()).collect();
            for forbidden in [
                "spawn_blocking",
                "block_in_place",
                "tokio::fs",
                "tokio::net::lookup_host",
                "reqwest::blocking",
                "SyncIoBridge",
            ] {
                if compact.contains(forbidden) {
                    violations.push(format!("{}: {forbidden}", path.display()));
                }
            }
        }
    }
    Ok(())
}
