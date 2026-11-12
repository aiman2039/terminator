//! Default per-user runtime directory, which holds the daemon socket.
use std::path::{Path, PathBuf};

/// FNV-1a (64-bit). Unlike `DefaultHasher`, the value never changes between
/// Rust releases, so every build finds the same socket directory.
fn stable_hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h: u64, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// The name older builds used. Kept so a new build finds a live older daemon.
fn legacy_hash(home: &Path) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    home.hash(&mut h);
    h.finish()
}

fn home_hash(home: &Path) -> u64 {
    stable_hash(home.as_os_str().as_encoded_bytes())
}

/// Picks the runtime directory for `home`. An existing legacy directory owned
/// by this user wins, so live older daemons stay reachable.
pub(crate) fn default_runtime(home: &Path) -> PathBuf {
    #[cfg(unix)]
    let base = PathBuf::from("/tmp");
    // Windows uses a TCP-loopback port file, so the runtime dir only
    // needs to be private, not short.
    #[cfg(not(unix))]
    let base = std::env::temp_dir();
    let legacy = base.join(format!("terminator-{:x}", legacy_hash(home)));
    let shared = base.join(format!("terminator-{:x}", home_hash(home)));
    let private = private_runtime().or_else(|| foreign_fallback(&shared, home));
    choose(legacy, private, shared)
}

fn choose(legacy: PathBuf, private: Option<PathBuf>, shared: PathBuf) -> PathBuf {
    if owned_dir(&legacy) {
        return legacy;
    }
    private.unwrap_or(shared)
}

/// When another user already holds the predictable shared directory
/// (#199), use a private one in the home directory. Every process of this
/// user sees the same foreign directory, so all of them pick the same
/// fallback.
fn foreign_fallback(shared: &Path, home: &Path) -> Option<PathBuf> {
    let meta = std::fs::symlink_metadata(shared).ok()?;
    if meta.is_dir() && owned(&meta) {
        return None;
    }
    Some(home.join(".terminator-run")).filter(|dir| socket_fits(dir))
}

/// True when a generation subdirectory and its socket fit under `dir`.
fn socket_fits(dir: &Path) -> bool {
    dir.join("12345678/daemon.sock").as_os_str().len() < crate::SOCKET_PATH_LIMIT
}

/// A per-user runtime directory that other users cannot pre-create.
#[cfg(target_os = "linux")]
fn private_runtime() -> Option<PathBuf> {
    let uid = rustix::process::geteuid().as_raw();
    // `/run/user/<uid>` first: agents may clear `XDG_RUNTIME_DIR` for hooks.
    [
        Some(PathBuf::from(format!("/run/user/{uid}"))),
        std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from),
    ]
    .into_iter()
    .flatten()
    .filter(|dir| dir.is_absolute() && owned_dir(dir))
    .map(|dir| dir.join("terminator"))
    // Room for a generation subdirectory and its socket.
    .find(|dir| socket_fits(dir))
}
#[cfg(not(target_os = "linux"))]
fn private_runtime() -> Option<PathBuf> {
    None
}

/// How often long-lived owners refresh their runtime files. macOS
/// `tmp_cleaner` deletes `/tmp` files whose access, change and modify
/// times are all older than 3 days, and a read does not refresh the access
/// time on APFS.
pub const KEEP_ALIVE_INTERVAL: std::time::Duration = std::time::Duration::from_hours(1);

/// Refreshes the modify time of an existing runtime file. A missing file
/// stays missing: a lock file must not get a new inode under its holder.
pub fn touch(path: &Path) -> std::io::Result<()> {
    std::fs::File::open(path)?.set_modified(std::time::SystemTime::now())
}

/// Writes `token` to a missing auth file, else refreshes its modify time.
pub fn keep_token(path: &Path, token: &str) -> Result<(), crate::AtomicWriteError> {
    match touch(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            crate::atomic_write(path, token.as_bytes())
        }
        result => Ok(result?),
    }
}

/// True for a real directory (not a symlink) owned by this user.
pub(crate) fn owned_dir(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.is_dir() && owned(&m))
}

#[cfg(unix)]
pub(crate) fn owned(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    meta.uid() == rustix::process::geteuid().as_raw()
}
#[cfg(not(unix))]
pub(crate) fn owned(_meta: &std::fs::Metadata) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_directory_hash_is_stable_across_toolchains() {
        assert_eq!(stable_hash(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(stable_hash(b"a"), 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn existing_legacy_directory_keeps_older_daemon_reachable() {
        let root = tempfile::tempdir().unwrap();
        let legacy = root.path().join("legacy");
        let shared = root.path().join("shared");
        assert_eq!(choose(legacy.clone(), None, shared.clone()), shared);
        std::fs::create_dir(&legacy).unwrap();
        assert_eq!(choose(legacy.clone(), None, shared), legacy);
    }

    #[test]
    fn deleted_auth_file_comes_back_with_the_live_token() {
        let root = tempfile::tempdir().unwrap();
        let auth = root.path().join("auth");
        keep_token(&auth, "live").unwrap();
        assert_eq!(std::fs::read_to_string(&auth).unwrap(), "live");
        std::fs::remove_file(&auth).unwrap();
        keep_token(&auth, "live").unwrap();
        assert_eq!(std::fs::read_to_string(&auth).unwrap(), "live");
    }

    #[test]
    fn keep_alive_refreshes_old_runtime_files_without_creating_locks() {
        let root = tempfile::tempdir().unwrap();
        let lock = root.path().join("daemon.lock");
        assert!(touch(&lock).is_err());
        assert!(!lock.exists());
        let file = std::fs::File::create(&lock).unwrap();
        let old = std::time::SystemTime::UNIX_EPOCH
            .checked_add(std::time::Duration::from_secs(1_000_000))
            .unwrap();
        file.set_modified(old).unwrap();
        touch(&lock).unwrap();
        assert!(std::fs::metadata(&lock).unwrap().modified().unwrap() > old);
        let auth = root.path().join("auth");
        std::fs::write(&auth, "other").unwrap();
        std::fs::File::open(&auth)
            .unwrap()
            .set_modified(old)
            .unwrap();
        keep_token(&auth, "live").unwrap();
        assert_eq!(std::fs::read_to_string(&auth).unwrap(), "other");
        assert!(std::fs::metadata(&auth).unwrap().modified().unwrap() > old);
    }

    #[test]
    fn private_runtime_wins_over_shared_tmp_without_legacy() {
        let root = tempfile::tempdir().unwrap();
        let private = root.path().join("private");
        let chosen = choose(
            root.path().join("missing"),
            Some(private.clone()),
            root.path().join("shared"),
        );
        assert_eq!(chosen, private);
    }

    #[cfg(unix)]
    #[test]
    fn foreign_shared_directory_falls_back_to_home() {
        let root = tempfile::tempdir().unwrap();
        // Short, like a real home; the temp root is too long for a socket.
        let home = Path::new("/h");
        // Missing or owned shared directories keep the shared path.
        assert_eq!(foreign_fallback(&root.path().join("missing"), home), None);
        assert_eq!(foreign_fallback(root.path(), home), None);
        // A plain file (or a directory of another user) cannot be used.
        let file = root.path().join("taken");
        std::fs::write(&file, "").unwrap();
        assert_eq!(
            foreign_fallback(&file, home),
            Some(home.join(".terminator-run"))
        );
        if !rustix::process::geteuid().is_root() {
            assert_eq!(
                foreign_fallback(Path::new("/"), home),
                Some(home.join(".terminator-run"))
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn init_rejects_runtime_directory_owned_by_another_user() {
        if rustix::process::geteuid().is_root() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let paths = crate::Paths {
            data: root.path().join("data"),
            runtime: PathBuf::from("/"),
        };
        assert!(matches!(
            paths.init(),
            Err(crate::PathsError::ForeignOwner { .. })
        ));
    }
}
