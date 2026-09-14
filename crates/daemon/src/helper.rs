//! Each daemon keeps its own executable copy until its last session is gone.
//! Bundle replacement must never replace code used by an existing shell hook.
use anyhow::{Context, Result, ensure};
use std::{
    fs,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};
use terminator_core::executable_available;

const PREFIX: &str = ".daemon-helper-";

pub struct Helper {
    pub executable: PathBuf,
    _directory: tempfile::TempDir,
}

impl Helper {
    pub fn stage(source: &Path, data: &Path) -> Result<Self> {
        ensure!(
            executable_available(source),
            "Required helper is missing or not executable: {}. Reinstall the complete Terminator app",
            source.display()
        );
        let directory = tempfile::Builder::new().prefix(PREFIX).tempdir_in(data)?;
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700))?;
        let executable = directory.path().join("terminator-hook");
        // Create a new inode, never a symlink or hard link into the app bundle.
        // Publish the path only after the complete executable is on disk.
        let mut source = fs::File::open(source)?;
        let mut target = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&executable)?;
        std::io::copy(&mut source, &mut target).context("Copy daemon helper")?;
        target.set_permissions(fs::Permissions::from_mode(0o500))?;
        target.sync_all()?;
        ensure!(
            executable_available(&executable),
            "Private helper is not executable"
        );
        Ok(Self {
            executable,
            _directory: directory,
        })
    }
}

/// Called only while holding this data directory's exclusive daemon lock.
/// Normal shutdown drops the TempDir; this removes leftovers from crashes.
pub fn cleanup_abandoned(data: &Path) -> Result<()> {
    for entry in fs::read_dir(data)? {
        let entry = entry?;
        if entry.file_name().to_string_lossy().starts_with(PREFIX) && entry.file_type()?.is_dir() {
            fs::remove_dir_all(entry.path())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{MetadataExt, symlink};

    #[test]
    fn replacement_and_removal_leave_each_daemons_helper_intact() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("terminator-hook");
        fs::write(&source, b"first build").unwrap();
        fs::set_permissions(&source, fs::Permissions::from_mode(0o700)).unwrap();
        let first = Helper::stage(&source, temp.path()).unwrap();
        assert_ne!(
            fs::metadata(&source).unwrap().ino(),
            fs::metadata(&first.executable).unwrap().ino()
        );
        fs::write(&source, b"second build").unwrap();
        let second = Helper::stage(&source, temp.path()).unwrap();
        fs::remove_file(source).unwrap();
        assert_eq!(fs::read(&first.executable).unwrap(), b"first build");
        assert_eq!(fs::read(&second.executable).unwrap(), b"second build");
        assert!(executable_available(&first.executable));
        assert_eq!(
            fs::metadata(first.executable.parent().unwrap())
                .unwrap()
                .mode()
                & 0o777,
            0o700
        );
        let path = first.executable.clone();
        drop(first);
        assert!(!path.exists());
        assert!(second.executable.exists());
    }

    #[test]
    fn missing_source_does_not_leave_a_partial_installation() {
        let temp = tempfile::tempdir().unwrap();
        assert!(Helper::stage(&temp.path().join("missing"), temp.path()).is_err());
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
    }

    #[test]
    fn cleanup_does_not_follow_symlinks_or_remove_other_data() {
        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("keep"), b"keep").unwrap();
        symlink(outside.path(), temp.path().join(format!("{PREFIX}link"))).unwrap();
        fs::create_dir(temp.path().join(format!("{PREFIX}abandoned"))).unwrap();
        fs::write(temp.path().join("state.sqlite"), b"keep").unwrap();
        cleanup_abandoned(temp.path()).unwrap();
        assert!(outside.path().join("keep").exists());
        assert!(temp.path().join("state.sqlite").exists());
        assert!(!temp.path().join(format!("{PREFIX}abandoned")).exists());
    }
}
