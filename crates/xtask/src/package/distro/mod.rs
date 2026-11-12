//! Linux `.deb` and `.rpm` packages built in-process from the staged
//! `terminator/` folder (the tarball tree). The GUI, daemon and hook helper
//! stay side by side under `/usr/lib/PACKAGE`, as the app expects. No
//! external packaging tool runs; nothing is installed or published.
mod deb;
mod rpm_package;
#[cfg(test)]
mod tests;

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use terminator_core::distribution::{LINUX_PACKAGE_NAME, package_dir};

/// One entry of a Linux package, at its absolute install path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Entry {
    File {
        path: String,
        source: PathBuf,
        mode: u32,
    },
    Data {
        path: String,
        bytes: Vec<u8>,
        mode: u32,
    },
    Symlink {
        path: String,
        target: String,
    },
}

impl Entry {
    pub(super) fn path(&self) -> &str {
        match self {
            Self::File { path, .. } | Self::Data { path, .. } | Self::Symlink { path, .. } => path,
        }
    }
}

/// Package identity shared by both formats.
pub(super) struct Meta {
    pub(super) version: String,
    /// `std::env::consts::ARCH`: `x86_64` or `aarch64`.
    pub(super) arch: String,
}

pub(super) const SUMMARY: &str =
    "Native terminal workspace with persistent sessions and agent hooks";

/// The desktop entry: launches the packaged GUI by its full path.
pub(super) fn desktop_entry() -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName=Terminator\nGenericName=Terminal workspace\nExec={}/terminator\nIcon={LINUX_PACKAGE_NAME}\nTerminal=false\nCategories=Development;TerminalEmulator;\nStartupWMClass=terminator\n",
        package_dir()
    )
}

fn executable(path: &Path) -> Result<bool> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        Ok(std::fs::metadata(path)?.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(false)
    }
}

fn walk(root: &Path, dir: &Path, entries: &mut Vec<Entry>) -> Result<()> {
    let mut children: Vec<_> = std::fs::read_dir(dir)?.collect::<Result<_, _>>()?;
    children.sort_by_key(std::fs::DirEntry::file_name);
    for child in children {
        let path = child.path();
        let kind = child.file_type()?;
        if kind.is_dir() {
            walk(root, &path, entries)?;
            continue;
        }
        let relative = path
            .strip_prefix(root)?
            .to_string_lossy()
            .replace('\\', "/");
        let install = format!("{}/{relative}", package_dir());
        if kind.is_symlink() {
            let target = std::fs::read_link(&path)?.to_string_lossy().into_owned();
            entries.push(Entry::Symlink {
                path: install,
                target,
            });
        } else {
            let mode = if executable(&path)? { 0o755 } else { 0o644 };
            entries.push(Entry::File {
                path: install,
                source: path,
                mode,
            });
        }
    }
    Ok(())
}

/// Every entry of a package built from the staged `app` folder: the folder
/// under `/usr/lib/PACKAGE`, launchers in `/usr/bin`, the desktop entry
/// and the icon. The launcher names avoid `/usr/bin/terminator`.
pub(super) fn layout(app: &Path) -> Result<Vec<Entry>> {
    for name in [
        "terminator",
        "terminator-daemon",
        "terminator-hook",
        "terminator.png",
    ] {
        anyhow::ensure!(app.join(name).is_file(), "Staged package lacks {name}");
    }
    let mut entries = Vec::new();
    walk(app, app, &mut entries).context("Read the staged package")?;
    let dir = package_dir();
    entries.extend([
        Entry::Symlink {
            path: format!("/usr/bin/{LINUX_PACKAGE_NAME}"),
            target: format!("{dir}/terminator"),
        },
        Entry::Symlink {
            path: format!("/usr/bin/{LINUX_PACKAGE_NAME}-hook"),
            target: format!("{dir}/terminator-hook"),
        },
        Entry::Data {
            path: format!("/usr/share/applications/{LINUX_PACKAGE_NAME}.desktop"),
            bytes: desktop_entry().into_bytes(),
            mode: 0o644,
        },
        Entry::File {
            path: format!("/usr/share/icons/hicolor/512x512/apps/{LINUX_PACKAGE_NAME}.png"),
            source: app.join("terminator.png"),
            mode: 0o644,
        },
    ]);
    Ok(entries)
}

/// Writes `NAME_VERSION_ARCH.deb` and `NAME-VERSION-1.ARCH.rpm` into
/// `destination`; returns their paths.
pub(super) fn packages(app: &Path, destination: &Path, meta: &Meta) -> Result<Vec<PathBuf>> {
    let entries = layout(app)?;
    let deb = destination.join(deb::file_name(meta));
    deb::write_deb(&entries, meta, &deb)?;
    let rpm = destination.join(rpm_package::file_name(meta));
    rpm_package::write_rpm(&entries, meta, &rpm)?;
    Ok(vec![deb, rpm])
}
