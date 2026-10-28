//! Stateless file reads and writes behind native editor buffers.
use anyhow::Context as _;
use std::path::Path;
use std::time::SystemTime;

pub(super) struct LoadedFile {
    pub(super) text: String,
    pub(super) mtime: Option<SystemTime>,
}

/// Disk access for native buffers; runs on the file worker pool.
pub(super) struct NativeFileIo;

impl NativeFileIo {
    pub(super) fn read(path: &Path, limit: u64) -> anyhow::Result<LoadedFile> {
        let file = std::fs::File::open(path).context("Open file")?;
        anyhow::ensure!(file.metadata()?.is_file(), "Not a regular file");
        let mut bytes = Vec::new();
        use std::io::Read as _;
        file.take(limit.saturating_add(1))
            .read_to_end(&mut bytes)
            .context("Read file")?;
        anyhow::ensure!(
            bytes.len() as u64 <= limit,
            "File exceeds the {} MiB viewer limit",
            limit / (1024 * 1024)
        );
        let text = String::from_utf8(bytes).context("File is not UTF-8")?;
        let mtime = Self::disk_mtime(path);
        Ok(LoadedFile { text, mtime })
    }

    pub(super) fn disk_mtime(path: &Path) -> Option<SystemTime> {
        std::fs::metadata(path).ok().and_then(|m| m.modified().ok())
    }

    pub(super) fn text_hash(text: &str) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut hasher);
        hasher.finish()
    }

    /// Replace a file's contents without a window where it is truncated or
    /// half-written: write a sibling temp file, give it the target's
    /// permissions, then rename it over the target. A symlink is followed so
    /// the link itself survives.
    pub(super) fn write_atomically(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
        use std::io::Write as _;
        let target = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let dir = target.parent().context("File has no parent folder")?;
        let name = target
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let temp = dir.join(format!(".{name}.terminator-save-{}", std::process::id()));
        let result = (|| {
            let mut file = std::fs::File::create(&temp).context("Create temporary file")?;
            file.write_all(bytes).context("Write file")?;
            file.sync_all().context("Flush file")?;
            if let Ok(meta) = std::fs::metadata(&target) {
                std::fs::set_permissions(&temp, meta.permissions()).context("Copy permissions")?;
            }
            std::fs::rename(&temp, &target).context("Replace file")
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temp);
        }
        result
    }
}
