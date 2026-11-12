//! Fills the Homebrew cask and AUR PKGBUILD templates in `packaging/` for
//! one release from its `SHA256SUMS.txt`. Writes local files only: nothing
//! is published, tapped, pushed or uploaded.
use crate::harness::root;
use anyhow::{Context, Result, bail, ensure};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use terminator_core::distribution::{LINUX_PACKAGE_NAME, RELEASE_REPO};

/// Release asset names, as `docs/RELEASING.md` lists them.
fn assets(version: &str) -> [(&'static str, String); 3] {
    [
        ("MACOS", format!("terminator-v{version}-macos.dmg")),
        (
            "LINUX_X86_64",
            format!("terminator-v{version}-x86_64-unknown-linux-gnu.tar.gz"),
        ),
        (
            "LINUX_AARCH64",
            format!("terminator-v{version}-aarch64-unknown-linux-gnu.tar.gz"),
        ),
    ]
}

/// `sha256  name` lines (as `sha256sum` writes them) by file name.
pub(super) fn checksums(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(|line| {
            let (sum, name) = line.split_once(char::is_whitespace)?;
            let name = name.trim().trim_start_matches('*');
            (sum.len() == 64 && sum.bytes().all(|b| b.is_ascii_hexdigit()))
                .then(|| (name.to_owned(), sum.to_ascii_lowercase()))
        })
        .collect()
}

/// The placeholder values for one release.
pub(super) fn values(
    version: &str,
    sums: &BTreeMap<String, String>,
) -> Result<Vec<(String, String)>> {
    semver::Version::parse(version).context("Version must be MAJOR.MINOR.PATCH")?;
    let mut values = vec![
        ("PACKAGE".to_owned(), LINUX_PACKAGE_NAME.to_owned()),
        ("VERSION".to_owned(), version.to_owned()),
        (
            "HOMEPAGE".to_owned(),
            format!("https://github.com/{RELEASE_REPO}"),
        ),
    ];
    for (key, asset) in assets(version) {
        let sum = sums
            .get(&asset)
            .with_context(|| format!("SHA256SUMS lacks {asset}"))?;
        values.push((
            format!("{key}_URL"),
            format!("https://github.com/{RELEASE_REPO}/releases/download/v{version}/{asset}"),
        ));
        values.push((format!("{key}_SHA256"), sum.clone()));
    }
    Ok(values)
}

/// Replaces every `@KEY@`; refuses a template with a placeholder left.
pub(super) fn fill(template: &str, values: &[(String, String)]) -> Result<String> {
    let mut text = template.to_owned();
    for (key, value) in values {
        text = text.replace(&format!("@{key}@"), value);
    }
    let left = text.split('@').skip(1).step_by(2).find(|word| {
        !word.is_empty()
            && word
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
    });
    if let Some(word) = left {
        bail!("Template placeholder @{word}@ has no value");
    }
    Ok(text)
}

/// The templates and the file name each one is written as.
fn templates() -> [(PathBuf, String); 2] {
    let dir = root().join("packaging");
    [
        (
            dir.join("homebrew/terminator-workspace.rb.in"),
            format!("{LINUX_PACKAGE_NAME}.rb"),
        ),
        (dir.join("aur/PKGBUILD.in"), "PKGBUILD".to_owned()),
    ]
}

pub fn run(version: &str, sums: &Path, output: Option<PathBuf>) -> Result<()> {
    let sums = checksums(&std::fs::read_to_string(sums).context("Read SHA256SUMS")?);
    let values = values(version, &sums)?;
    let output = output.unwrap_or_else(|| root().join("target/package/manifests"));
    std::fs::create_dir_all(&output)?;
    for (template, name) in templates() {
        let text = fill(&std::fs::read_to_string(&template)?, &values)?;
        ensure!(!text.is_empty(), "Empty template {}", template.display());
        let target = output.join(name);
        std::fs::write(&target, text)?;
        println!("{}", target.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sums(version: &str) -> String {
        assets(version)
            .iter()
            .enumerate()
            .map(|(index, (_, asset))| format!("{}  {asset}", index.to_string().repeat(64)))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn manifests_fill_every_placeholder_from_the_release_checksums() {
        let values = values("1.2.3", &checksums(&sums("1.2.3"))).unwrap();
        for (template, _) in templates() {
            let text = fill(&std::fs::read_to_string(&template).unwrap(), &values).unwrap();
            assert!(!text.contains("@VERSION@") && text.contains("1.2.3"));
            assert!(text.contains("terminator-workspace"));
        }
        let (cask, _) = &templates()[0];
        let cask = fill(&std::fs::read_to_string(cask).unwrap(), &values).unwrap();
        assert!(cask.contains(&format!("sha256 \"{}\"", "0".repeat(64))));
        assert!(cask.contains(
            "https://github.com/aiman2039/terminator/releases/download/v1.2.3/terminator-v1.2.3-macos.dmg"
        ));
        let (pkgbuild, _) = &templates()[1];
        let pkgbuild = fill(&std::fs::read_to_string(pkgbuild).unwrap(), &values).unwrap();
        assert!(pkgbuild.contains(&format!("sha256sums_aarch64=('{}')", "2".repeat(64))));
        assert!(pkgbuild.contains("pkgname=terminator-workspace-bin"));
    }

    #[test]
    fn manifests_refuse_a_missing_checksum_or_placeholder() {
        let partial = sums("1.2.3").lines().take(2).collect::<Vec<_>>().join("\n");
        assert!(values("1.2.3", &checksums(&partial)).is_err());
        assert!(values("latest", &checksums(&sums("latest"))).is_err());
        assert!(
            fill(
                "version @VERSION@ @OTHER@",
                &[("VERSION".into(), "1".into())]
            )
            .is_err()
        );
        assert_eq!(
            fill("a@b.c @VERSION@", &[("VERSION".into(), "1".into())]).unwrap(),
            "a@b.c 1"
        );
    }
}
