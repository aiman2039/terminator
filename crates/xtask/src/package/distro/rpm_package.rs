//! `.rpm` with the pure-Rust `rpm` crate (gzip payload, unsigned).
use super::{Entry, Meta, SUMMARY};
use anyhow::Result;
use std::path::Path;
use terminator_core::distribution::{LINUX_PACKAGE_NAME, RELEASE_REPO};

pub(super) fn file_name(meta: &Meta) -> String {
    format!("{LINUX_PACKAGE_NAME}-{}-1.{}.rpm", meta.version, meta.arch)
}

fn permissions(mode: u32) -> u16 {
    u16::try_from(mode & 0o7777).unwrap_or(0o644)
}

pub(super) fn write_rpm(entries: &[Entry], meta: &Meta, output: &Path) -> Result<()> {
    let mut builder = rpm::PackageBuilder::new(
        LINUX_PACKAGE_NAME,
        &meta.version,
        "MIT",
        &meta.arch,
        SUMMARY,
    );
    builder
        .using_config(rpm::BuildConfig::default().compression(rpm::CompressionType::Gzip))
        .url(format!("https://github.com/{RELEASE_REPO}"))
        .description(format!(
            "GUI, session daemon and hook helper installed side by side in /usr/lib/{LINUX_PACKAGE_NAME}."
        ))
        .requires(rpm::Dependency::any("webkit2gtk4.1"))
        .requires(rpm::Dependency::any("gtk3"))
        .recommends(rpm::Dependency::any("neovim"))
        .recommends(rpm::Dependency::any("git"));
    for entry in entries {
        match entry {
            Entry::File { path, source, mode } => {
                builder.with_file(
                    source,
                    rpm::FileOptions::new(path.as_str()).permissions(permissions(*mode)),
                )?;
            }
            Entry::Data { path, bytes, mode } => {
                builder.with_file_contents(
                    bytes.clone(),
                    rpm::FileOptions::new(path.as_str()).permissions(permissions(*mode)),
                )?;
            }
            Entry::Symlink { path, target } => {
                builder.with_symlink(rpm::FileOptions::symlink(path.as_str(), target.as_str()))?;
            }
        }
    }
    builder.build()?.write_file(output)?;
    Ok(())
}
