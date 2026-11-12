//! Stateless file reads and writes behind native editor buffers: text
//! encoding detection, disk fingerprints, and safe in-place or atomic
//! writes.
use anyhow::Context as _;
use encoding_rs::{Encoding, UTF_8, UTF_16BE, UTF_16LE};
use std::path::Path;
use std::time::SystemTime;
use terminator_native_edit::indent::EndOfLine;

pub(super) struct LoadedFile {
    pub(super) text: String,
    pub(super) mtime: Option<SystemTime>,
    pub(super) encoding: TextEncoding,
    pub(super) fingerprint: Option<Fingerprint>,
}

/// What a finished save reports back.
#[derive(Clone, Copy, Debug)]
pub(super) struct SavedFile {
    pub(super) mtime: Option<SystemTime>,
    pub(super) fingerprint: Fingerprint,
    /// Hash of the buffer text written (see [`Doc::content_hash`]).
    ///
    /// [`Doc::content_hash`]: terminator_native_edit::doc::Doc::content_hash
    pub(super) text_hash: u64,
}

/// How a file's bytes map to text: the encoding, and whether the file
/// starts with a byte-order mark. Saves write the same form back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TextEncoding {
    pub(super) encoding: &'static Encoding,
    pub(super) bom: bool,
}

impl Default for TextEncoding {
    fn default() -> Self {
        Self {
            encoding: UTF_8,
            bom: false,
        }
    }
}

impl TextEncoding {
    /// Status-bar name: `UTF-8`, `UTF-8 BOM`, `windows-1252`, …
    pub(super) fn label(self) -> String {
        if self.bom {
            format!("{} BOM", self.encoding.name())
        } else {
            self.encoding.name().to_owned()
        }
    }

    /// Byte-order mark this form writes first.
    fn bom_bytes(self) -> &'static [u8] {
        match (self.bom, self.encoding) {
            (false, _) => b"",
            (true, e) if e == UTF_16LE => b"\xFF\xFE",
            (true, e) if e == UTF_16BE => b"\xFE\xFF",
            (true, _) => b"\xEF\xBB\xBF",
        }
    }

    /// Text from file bytes: a BOM wins, then UTF-8, then the
    /// `chardetng` guess. Binary files and bytes the guessed encoding
    /// cannot decode exactly are refused, so a save never corrupts them.
    pub(super) fn decode(bytes: Vec<u8>) -> anyhow::Result<(String, Self)> {
        if let Some((encoding, bom_len)) = Encoding::for_bom(&bytes) {
            let body = bytes.get(bom_len..).unwrap_or_default();
            let (text, had_errors) = encoding.decode_without_bom_handling(body);
            anyhow::ensure!(!had_errors, "File is not valid {}", encoding.name());
            return Ok((
                text.into_owned(),
                Self {
                    encoding,
                    bom: true,
                },
            ));
        }
        let bytes = match String::from_utf8(bytes) {
            Ok(text) => return Ok((text, Self::default())),
            Err(error) => error.into_bytes(),
        };
        anyhow::ensure!(!bytes.contains(&0), "File looks binary");
        let mut detector = chardetng::EncodingDetector::new();
        detector.feed(&bytes, true);
        let encoding = detector.guess(None, true);
        let (text, had_errors) = encoding.decode_without_bom_handling(&bytes);
        anyhow::ensure!(!had_errors, "File is not valid {}", encoding.name());
        Ok((
            text.into_owned(),
            Self {
                encoding,
                bom: false,
            },
        ))
    }

    /// File bytes for `text` in this form. Characters the encoding cannot
    /// represent refuse the save instead of writing replacements.
    pub(super) fn encode(self, text: &str) -> anyhow::Result<Vec<u8>> {
        let mut out = self.bom_bytes().to_vec();
        if self.encoding == UTF_16LE || self.encoding == UTF_16BE {
            let little = self.encoding == UTF_16LE;
            for unit in text.encode_utf16() {
                out.extend(if little {
                    unit.to_le_bytes()
                } else {
                    unit.to_be_bytes()
                });
            }
            return Ok(out);
        }
        let (bytes, _, unmappable) = self.encoding.encode(text);
        anyhow::ensure!(
            !unmappable,
            "Some characters cannot be saved as {}; convert the file to UTF-8",
            self.encoding.name()
        );
        out.extend_from_slice(&bytes);
        Ok(out)
    }
}

/// Size and content hash of a file's bytes: the save conflict check
/// compares these, not just the modification time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Fingerprint {
    pub(super) size: u64,
    pub(super) hash: u64,
}

impl Fingerprint {
    pub(super) fn of(bytes: &[u8]) -> Self {
        Self {
            size: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
            hash: terminator_native_edit::doc::hash_bytes(bytes),
        }
    }

    /// Fingerprint of the file on disk; `None` when it does not exist.
    pub(super) fn read(path: &Path) -> anyhow::Result<Option<Self>> {
        match std::fs::read(path) {
            Ok(bytes) => Ok(Some(Self::of(&bytes))),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error).context("Read file"),
        }
    }
}

/// Line breaks of a loaded file. Only uniform CRLF converts: the buffer
/// holds `\n` (so carets never land on a `\r`) and saves write `\r\n`
/// back; mixed endings keep their bytes unchanged.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum LineEnding {
    #[default]
    Lf,
    Crlf,
    Mixed,
}

impl LineEnding {
    /// Buffer text for file text, and the file's line breaks.
    pub(crate) fn decode(text: String) -> (String, Self) {
        let ending = Self::detect(&text);
        if ending == Self::Crlf {
            (text.replace("\r\n", "\n"), ending)
        } else {
            (text, ending)
        }
    }

    /// Breaks for `text` as loaded: a file with no break yet (a new or
    /// one-line file) takes `.editorconfig`'s `end_of_line` when it asks
    /// for CRLF. Files with breaks keep their own.
    pub(super) fn or_configured(self, text: &str, configured: Option<EndOfLine>) -> Self {
        if configured == Some(EndOfLine::Crlf) && !text.contains(['\n', '\r']) {
            Self::Crlf
        } else {
            self
        }
    }

    /// File text for buffer text (the inverse of [`LineEnding::decode`]).
    /// A `\r\n` already in the buffer (for example after Undo of a
    /// conversion) stays one break, never `\r\r\n`.
    pub(crate) fn encode(self, text: String) -> String {
        if self == Self::Crlf {
            text.replace("\r\n", "\n").replace('\n', "\r\n")
        } else {
            text
        }
    }

    /// Status-bar and menu name.
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Lf => "LF",
            Self::Crlf => "CRLF",
            Self::Mixed => "Mixed",
        }
    }

    /// `\r\n` pairs count as one break; anything else with a `\r` is mixed.
    pub(super) fn detect(text: &str) -> Self {
        let crlf = text.matches("\r\n").count();
        let cr = text.matches('\r').count();
        let lf = text.matches('\n').count();
        match (crlf, cr == crlf, lf == crlf) {
            (0, true, _) => Self::Lf,
            (_, true, true) => Self::Crlf,
            _ => Self::Mixed,
        }
    }
}

/// Disk access for native buffers; runs on the file worker pool.
pub(super) struct NativeFileIo;

impl NativeFileIo {
    /// [`Self::read`]; with `new_file`, a missing file is an empty new
    /// buffer with no disk fingerprint, so its first save sees no conflict.
    pub(super) fn read_or_new(
        path: &Path,
        limit: u64,
        new_file: bool,
    ) -> anyhow::Result<LoadedFile> {
        match std::fs::symlink_metadata(path) {
            Err(error) if new_file && error.kind() == std::io::ErrorKind::NotFound => {
                Ok(LoadedFile {
                    text: String::new(),
                    mtime: None,
                    encoding: TextEncoding::default(),
                    fingerprint: None,
                })
            }
            _ => Self::read(path, limit),
        }
    }

    pub(super) fn read(path: &Path, limit: u64) -> anyhow::Result<LoadedFile> {
        #[cfg(unix)]
        use std::os::unix::fs::OpenOptionsExt;
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        // A FIFO with no writer would block `open` and the file-pool thread.
        #[cfg(unix)]
        options.custom_flags(libc::O_NONBLOCK);
        let file = options.open(path).context("Open file")?;
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
        let fingerprint = Fingerprint::of(&bytes);
        let (text, encoding) = TextEncoding::decode(bytes)?;
        let mtime = Self::disk_mtime(path);
        Ok(LoadedFile {
            text,
            mtime,
            encoding,
            fingerprint: Some(fingerprint),
        })
    }

    pub(super) fn disk_mtime(path: &Path) -> Option<SystemTime> {
        std::fs::metadata(path).ok().and_then(|m| m.modified().ok())
    }

    /// Write `bytes` as the new contents of `path`. A file with other
    /// hard links, or one owned by another user, is rewritten in place
    /// (a rename would split the links or change the owner); every other
    /// file is replaced atomically.
    pub(super) fn write(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
        if Self::write_in_place_needed(path) {
            let target = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
            Self::write_in_place(&target, bytes)
        } else {
            Self::write_atomically(path, bytes)
        }
    }

    #[cfg(unix)]
    fn write_in_place_needed(path: &Path) -> bool {
        use std::os::unix::fs::MetadataExt as _;
        std::fs::metadata(path).is_ok_and(|meta| meta.nlink() > 1)
    }

    #[cfg(not(unix))]
    fn write_in_place_needed(_path: &Path) -> bool {
        false
    }

    /// A new file in the target's folder gets another owner or group than
    /// the target: renaming it over the target would change them.
    #[cfg(unix)]
    fn owner_differs(temp: &Path, target: &Path) -> bool {
        use std::os::unix::fs::MetadataExt as _;
        match (std::fs::metadata(temp), std::fs::metadata(target)) {
            (Ok(temp), Ok(target)) => temp.uid() != target.uid() || temp.gid() != target.gid(),
            _ => false,
        }
    }

    #[cfg(not(unix))]
    fn owner_differs(_temp: &Path, _target: &Path) -> bool {
        false
    }

    /// Rewrite `target` in place (hard links and owner stay) without
    /// losing text: the new bytes go to a synced sibling copy first, then
    /// over the old bytes (no truncation before the write), then the
    /// length is set. If the in-place write fails, the copy is kept and
    /// the error names it.
    fn write_in_place(target: &Path, bytes: &[u8]) -> anyhow::Result<()> {
        use std::io::Write as _;
        let dir = target.parent().context("File has no parent folder")?;
        let name = target
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let copy = dir.join(format!(".{name}.terminator-unsaved-{}", std::process::id()));
        let mut file = std::fs::File::create(&copy).context("Create safety copy")?;
        let staged = file
            .write_all(bytes)
            .and_then(|()| file.sync_all())
            .context("Write safety copy");
        drop(file);
        if let Err(error) = staged {
            let _ = std::fs::remove_file(&copy);
            return Err(error);
        }
        let written = (|| -> std::io::Result<()> {
            let mut file = std::fs::OpenOptions::new().write(true).open(target)?;
            file.write_all(bytes)?;
            file.set_len(u64::try_from(bytes.len()).unwrap_or(u64::MAX))?;
            file.sync_all()
        })();
        match written {
            Ok(()) => {
                let _ = std::fs::remove_file(&copy);
                Ok(())
            }
            Err(error) => Err(anyhow::Error::new(error).context(format!(
                "Write file; the new text is kept in {}",
                copy.display()
            ))),
        }
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
            if Self::owner_differs(&temp, &target) {
                drop(file);
                let _ = std::fs::remove_file(&temp);
                return Self::write_in_place(&target, bytes);
            }
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

    /// The save step on the worker: refuse when the disk bytes are not
    /// the ones last loaded or saved (unless forced), then write.
    pub(super) fn save(request: SaveRequest) -> anyhow::Result<SavedFile> {
        let SaveRequest {
            path,
            text,
            line_ending,
            encoding,
            expected,
            force,
        } = request;
        if !force && Fingerprint::read(&path)? != expected {
            return Err(ChangedOnDisk.into());
        }
        let text_hash = terminator_native_edit::doc::hash_text(&text);
        let bytes = encoding.encode(&line_ending.encode(text))?;
        Self::write(&path, &bytes)?;
        Ok(SavedFile {
            mtime: Self::disk_mtime(&path),
            fingerprint: Fingerprint::of(&bytes),
            text_hash,
        })
    }
}

/// The save worker refused: the disk bytes are not the ones last loaded.
#[derive(Debug)]
pub(super) struct ChangedOnDisk;

impl std::fmt::Display for ChangedOnDisk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Changed on disk since loading")
    }
}

impl std::error::Error for ChangedOnDisk {}

/// Why the last save failed. Only a disk conflict offers "Save anyway".
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SaveError {
    ChangedOnDisk,
    Failed(String),
}

impl SaveError {
    pub(super) fn of(error: &anyhow::Error) -> Self {
        if error.downcast_ref::<ChangedOnDisk>().is_some() {
            Self::ChangedOnDisk
        } else {
            Self::Failed(format!("{error:#}"))
        }
    }
}

impl std::fmt::Display for SaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ChangedOnDisk => ChangedOnDisk.fmt(f),
            Self::Failed(text) => f.write_str(text),
        }
    }
}

/// What the save worker writes, and what it expects on disk first.
pub(super) struct SaveRequest {
    pub(super) path: std::path::PathBuf,
    /// The buffer text (LF breaks for a CRLF file).
    pub(super) text: String,
    pub(super) line_ending: LineEnding,
    pub(super) encoding: TextEncoding,
    pub(super) expected: Option<Fingerprint>,
    pub(super) force: bool,
}
