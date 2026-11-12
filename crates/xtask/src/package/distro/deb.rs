//! `.deb`: an `ar` archive of `debian-binary`, `control.tar.gz` and
//! `data.tar.gz`, written with the `ar`, `tar` and `flate2` crates.
use super::{Entry, Meta, SUMMARY};
use anyhow::Result;
use std::collections::BTreeSet;
use std::path::Path;
use terminator_core::distribution::{LINUX_PACKAGE_NAME, RELEASE_REPO};

/// Debian's name for the machine architecture.
pub(super) fn arch(meta: &Meta) -> &str {
    match meta.arch.as_str() {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        other => other,
    }
}

pub(super) fn file_name(meta: &Meta) -> String {
    format!("{LINUX_PACKAGE_NAME}_{}_{}.deb", meta.version, arch(meta))
}

fn installed_kib(entries: &[Entry]) -> Result<u64> {
    let mut bytes = 0_u64;
    for entry in entries {
        bytes = bytes.saturating_add(match entry {
            Entry::File { source, .. } => std::fs::metadata(source)?.len(),
            Entry::Data { bytes, .. } => u64::try_from(bytes.len())?,
            Entry::Symlink { .. } => 0,
        });
    }
    Ok(bytes.div_ceil(1024))
}

pub(super) fn control(entries: &[Entry], meta: &Meta) -> Result<String> {
    Ok(format!(
        "Package: {LINUX_PACKAGE_NAME}\nVersion: {}\nArchitecture: {}\nMaintainer: Terminator maintainers <https://github.com/{RELEASE_REPO}>\nInstalled-Size: {}\nDepends: libc6, libgtk-3-0, libwebkit2gtk-4.1-0, libxkbcommon0, libfontconfig1, libasound2t64 | libasound2\nRecommends: neovim, git\nSection: devel\nPriority: optional\nHomepage: https://github.com/{RELEASE_REPO}\nDescription: {SUMMARY}\n GUI, session daemon and hook helper installed side by side in\n /usr/lib/{LINUX_PACKAGE_NAME}. Launch it as {LINUX_PACKAGE_NAME}.\n",
        meta.version,
        arch(meta),
        installed_kib(entries)?
    ))
}

fn mtime() -> u64 {
    std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0)
}

fn header(kind: tar::EntryType, mode: u32, size: u64) -> tar::Header {
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(kind);
    header.set_mode(mode);
    header.set_size(size);
    header.set_uid(0);
    header.set_gid(0);
    header.set_mtime(mtime());
    header
}

fn gzip_tar(
    fill: impl FnOnce(&mut tar::Builder<flate2::write::GzEncoder<Vec<u8>>>) -> Result<()>,
) -> Result<Vec<u8>> {
    let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let mut builder = tar::Builder::new(encoder);
    fill(&mut builder)?;
    Ok(builder.into_inner()?.finish()?)
}

/// Every parent folder of the entries; sorted, so a parent comes first.
fn folders(entries: &[Entry]) -> BTreeSet<String> {
    let mut folders = BTreeSet::new();
    for entry in entries {
        let mut parent = Path::new(entry.path()).parent();
        while let Some(dir) = parent.filter(|dir| *dir != Path::new("/")) {
            folders.insert(format!(".{}/", dir.display()));
            parent = dir.parent();
        }
    }
    folders
}

fn data_tar(entries: &[Entry]) -> Result<Vec<u8>> {
    gzip_tar(|tar| {
        for folder in folders(entries) {
            let mut head = header(tar::EntryType::Directory, 0o755, 0);
            tar.append_data(&mut head, folder, std::io::empty())?;
        }
        for entry in entries {
            let path = format!(".{}", entry.path());
            match entry {
                Entry::File { source, mode, .. } => {
                    let file = std::fs::File::open(source)?;
                    let mut head = header(tar::EntryType::Regular, *mode, file.metadata()?.len());
                    tar.append_data(&mut head, path, file)?;
                }
                Entry::Data { bytes, mode, .. } => {
                    let mut head =
                        header(tar::EntryType::Regular, *mode, u64::try_from(bytes.len())?);
                    tar.append_data(&mut head, path, bytes.as_slice())?;
                }
                Entry::Symlink { target, .. } => {
                    let mut head = header(tar::EntryType::Symlink, 0o777, 0);
                    tar.append_link(&mut head, path, target)?;
                }
            }
        }
        Ok(())
    })
}

pub(super) fn write_deb(entries: &[Entry], meta: &Meta, output: &Path) -> Result<()> {
    let control_text = control(entries, meta)?;
    let control_tar = gzip_tar(|tar| {
        let mut head = header(
            tar::EntryType::Regular,
            0o644,
            u64::try_from(control_text.len())?,
        );
        tar.append_data(&mut head, "./control", control_text.as_bytes())?;
        Ok(())
    })?;
    let data = data_tar(entries)?;
    let mut archive = ar::Builder::new(std::fs::File::create(output)?);
    for (name, bytes) in [
        ("debian-binary", b"2.0\n".to_vec()),
        ("control.tar.gz", control_tar),
        ("data.tar.gz", data),
    ] {
        let mut head = ar::Header::new(name.as_bytes().to_vec(), u64::try_from(bytes.len())?);
        head.set_mode(0o100_644);
        head.set_mtime(mtime());
        archive.append(&head, bytes.as_slice())?;
    }
    Ok(())
}
