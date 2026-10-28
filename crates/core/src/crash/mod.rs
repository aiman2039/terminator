//! Fatal panic dump. Writes a crash log, then lets the process die.
//! Does not resume. Recovered worker panics are not dumped.

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::{
    backtrace::Backtrace,
    ffi::OsStr,
    fs,
    panic::{AssertUnwindSafe, PanicHookInfo},
    path::{Path, PathBuf},
    sync::OnceLock,
    thread::ThreadId,
    time::SystemTime,
};

const KEEP: usize = 20;
const CRASHES: &str = "crashes";

pub struct CrashInstall {
    pub binary: &'static str,
    pub version: &'static str,
    pub data_dir: PathBuf,
    pub notify: Option<fn(&CrashNotice)>,
}

pub struct CrashNotice {
    pub path: PathBuf,
    pub summary: String,
}

struct Installed {
    binary: &'static str,
    version: &'static str,
    data_dir: PathBuf,
    owner: ThreadId,
    notify: Option<fn(&CrashNotice)>,
}

struct Report {
    binary: &'static str,
    version: &'static str,
    thread: String,
    location: String,
    summary: String,
    backtrace: String,
}

static INSTALLED: OnceLock<Installed> = OnceLock::new();

pub fn install(
    CrashInstall {
        binary,
        version,
        data_dir,
        notify,
    }: CrashInstall,
) {
    let installed = Installed {
        binary,
        version,
        data_dir,
        owner: std::thread::current().id(),
        notify,
    };
    if INSTALLED.set(installed).is_err() {
        return;
    }
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = std::panic::catch_unwind(AssertUnwindSafe(|| handle(info)));
        previous(info);
    }));
}

impl CrashNotice {
    #[must_use]
    pub fn dialog_body(&self) -> String {
        let summary = &self.summary;
        if self.path.as_os_str().is_empty() {
            return format!(
                "{summary}\n\nTerminator could not write a crash log. The app will now quit. Terminal sessions keep running in the background."
            );
        }
        let path = self.path.display();
        format!(
            "{summary}\n\nA crash log was written to:\n{path}\n\nThe app will now quit. Terminal sessions keep running in the background."
        )
    }
}

fn handle(info: &PanicHookInfo<'_>) {
    let Some(cfg) = INSTALLED.get() else {
        return;
    };
    if !is_owner(cfg.owner) {
        return;
    }
    let report = Report::from_panic(cfg, info);
    let path = write_report(&cfg.data_dir, cfg.binary, &report);
    prune_old(&crash_dir(&cfg.data_dir));
    notify_owner(cfg.notify, path, report.summary);
}

fn is_owner(owner: ThreadId) -> bool {
    std::thread::current().id() == owner
}

fn crash_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(CRASHES)
}

fn dialog_enabled() -> bool {
    dialog_allowed(
        std::env::var_os("CI").is_some(),
        std::env::var_os("TERMINATOR_CAPTURE_PATH").is_some(),
        std::env::var_os("TERMINATOR_CRASH_DIALOG").as_deref(),
    )
}

fn dialog_allowed(ci: bool, capture: bool, crash_dialog: Option<&OsStr>) -> bool {
    if ci || capture {
        return false;
    }
    crash_dialog.is_none_or(|value| value != "0")
}

fn notify_owner(notify: Option<fn(&CrashNotice)>, path: Option<PathBuf>, summary: String) {
    if !dialog_enabled() {
        return;
    }
    fire_notify(notify, path, summary);
}

fn fire_notify(notify: Option<fn(&CrashNotice)>, path: Option<PathBuf>, summary: String) {
    let Some(notify) = notify else {
        return;
    };
    let notice = CrashNotice {
        path: path.unwrap_or_default(),
        summary,
    };
    let _ = std::panic::catch_unwind(AssertUnwindSafe(|| notify(&notice)));
}

impl Report {
    fn from_panic(cfg: &Installed, info: &PanicHookInfo<'_>) -> Self {
        Self {
            binary: cfg.binary,
            version: cfg.version,
            thread: std::thread::current()
                .name()
                .unwrap_or("unnamed")
                .to_owned(),
            location: info.location().map_or_else(
                || "unknown".into(),
                |location| {
                    let file = location.file();
                    let line = location.line();
                    let column = location.column();
                    format!("{file}:{line}:{column}")
                },
            ),
            summary: info
                .payload_as_str()
                .map_or_else(|| "unknown panic payload".into(), ToOwned::to_owned),
            backtrace: format!("{}", Backtrace::force_capture()),
        }
    }

    fn render(&self) -> String {
        let Self {
            binary,
            version,
            thread,
            location,
            summary,
            backtrace,
        } = self;
        let time = crate::now();
        format!(
            "Terminator crash\nbinary: {binary}\nversion: {version}\ntime: {time}\nthread: {thread}\npanic: {summary}\nlocation: {location}\n\nbacktrace:\n{backtrace}\n"
        )
    }
}

fn write_report(data_dir: &Path, binary: &str, report: &Report) -> Option<PathBuf> {
    let dir = crash_dir(data_dir);
    if !prepare_crash_dir(&dir) {
        return None;
    }
    let stamp = crate::now();
    let path = unique_path(&dir, &format!("{binary}-{stamp}.log"));
    crate::atomic_write(&path, report.render().as_bytes()).ok()?;
    Some(path)
}

fn prepare_crash_dir(dir: &Path) -> bool {
    if dir.exists() && fs::symlink_metadata(dir).is_ok_and(|meta| meta.file_type().is_symlink()) {
        return false;
    }
    if fs::create_dir_all(dir).is_err() {
        return false;
    }
    #[cfg(unix)]
    let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));
    true
}

fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    if !path.exists() {
        return path;
    }
    for n in 1..100 {
        let candidate = dir.join(format!("{name}.{n}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    dir.join(format!("{name}.{}", crate::id()))
}

fn prune_old(dir: &Path) {
    let mut files: Vec<_> = fs::read_dir(dir)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let entry = entry.ok()?;
            (entry.path().extension()? == "log").then_some(entry)
        })
        .collect();
    if files.len() <= KEEP {
        return;
    }
    files.sort_by_key(|entry| {
        entry
            .metadata()
            .ok()
            .and_then(|meta| meta.modified().ok())
            .unwrap_or(SystemTime::UNIX_EPOCH)
    });
    let drop = files.len().saturating_sub(KEEP);
    for entry in files.into_iter().take(drop) {
        let _ = fs::remove_file(entry.path());
    }
}

#[cfg(test)]
mod tests;
