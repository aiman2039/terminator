//! Fatal panic dump. Writes a crash log, then lets the process die.
//! Does not resume. Recovered worker panics are not dumped.

use std::{
    backtrace::Backtrace,
    ffi::OsStr,
    fs,
    os::unix::fs::PermissionsExt,
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
            location: info
                .location()
                .map(|location| {
                    let file = location.file();
                    let line = location.line();
                    let column = location.column();
                    format!("{file}:{line}:{column}")
                })
                .unwrap_or_else(|| "unknown".into()),
            summary: info
                .payload_as_str()
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| "unknown panic payload".into()),
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
    let drop = files.len() - KEEP;
    for entry in files.into_iter().take(drop) {
        let _ = fs::remove_file(entry.path());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        process::Command,
        time::{Duration, UNIX_EPOCH},
    };

    fn report() -> Report {
        Report {
            binary: "terminator",
            version: "0.0-test",
            thread: "main".into(),
            location: "crates/app/src/workspace.rs:271:23".into(),
            summary: "index out of bounds: the len is 0 but the index is 0".into(),
            backtrace: "stack".into(),
        }
    }

    #[test]
    fn crash_report_includes_panic_message_location_and_binary() {
        let text = report().render();
        assert!(text.contains("binary: terminator"));
        assert!(text.contains("version: 0.0-test"));
        assert!(text.contains("crates/app/src/workspace.rs:271:23"));
        assert!(text.contains("index out of bounds: the len is 0 but the index is 0"));
        assert!(text.contains("backtrace:\nstack"));
    }

    #[test]
    fn write_report_creates_file_under_crashes() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write_report(tmp.path(), "terminator", &report()).unwrap();
        assert!(path.starts_with(tmp.path().join(CRASHES)));
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("binary: terminator"));
        assert!(text.contains("index out of bounds"));
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn prune_keeps_the_newest_twenty_logs() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(CRASHES);
        fs::create_dir_all(&dir).unwrap();
        for i in 0..=KEEP {
            let path = dir.join(format!("terminator-{i}.log"));
            fs::write(&path, b"x").unwrap();
            let file = fs::File::open(&path).unwrap();
            file.set_modified(UNIX_EPOCH + Duration::from_secs(i as u64))
                .unwrap();
        }
        fs::write(dir.join("notes.txt"), b"keep").unwrap();
        prune_old(&dir);
        assert!(!dir.join("terminator-0.log").exists());
        assert!(dir.join(format!("terminator-{KEEP}.log")).exists());
        assert!(dir.join("notes.txt").exists());
        let logs = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|entry| {
                let path = entry.ok()?.path();
                (path.extension()? == "log").then_some(path)
            })
            .count();
        assert_eq!(logs, KEEP);
    }

    #[test]
    fn other_thread_panics_do_not_write_a_dump() {
        let owner = std::thread::current().id();
        assert!(is_owner(owner));
        let other = std::thread::spawn(move || is_owner(owner)).join().unwrap();
        assert!(!other);
    }

    #[test]
    fn write_report_refuses_a_symlink_crash_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("target");
        fs::create_dir(&target).unwrap();
        std::os::unix::fs::symlink(&target, tmp.path().join(CRASHES)).unwrap();
        assert!(write_report(tmp.path(), "terminator", &report()).is_none());
        assert!(fs::read_dir(&target).unwrap().next().is_none());
    }

    #[test]
    fn paths_crash_dir_matches_the_dump_directory() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(
            crate::Paths::at(tmp.path().into()).crash_dir(),
            crash_dir(tmp.path())
        );
    }

    #[test]
    fn crash_dialog_body_includes_the_log_path() {
        let notice = CrashNotice {
            path: PathBuf::from("/tmp/terminator/crashes/terminator-1.log"),
            summary: "index out of bounds".into(),
        };
        let body = notice.dialog_body();
        assert!(body.contains("index out of bounds"));
        assert!(body.contains("/tmp/terminator/crashes/terminator-1.log"));
        let missing = CrashNotice {
            path: PathBuf::new(),
            summary: "index out of bounds".into(),
        };
        assert!(
            missing
                .dialog_body()
                .contains("could not write a crash log")
        );
    }

    #[test]
    fn crash_dialog_skips_ci_fixtures_and_explicit_off() {
        assert!(dialog_allowed(false, false, None));
        assert!(dialog_allowed(false, false, Some(OsStr::new("1"))));
        assert!(!dialog_allowed(true, false, None));
        assert!(!dialog_allowed(false, true, None));
        assert!(!dialog_allowed(false, false, Some(OsStr::new("0"))));
    }

    fn notify_writes_marker(notice: &CrashNotice) {
        let _ = fs::write(notice.path.with_extension("notified"), notice.dialog_body());
    }

    fn panicking_notify(_: &CrashNotice) {
        panic!("notify failed");
    }

    #[test]
    fn notify_writes_the_dialog_body_next_to_the_log() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("crash.log");
        fire_notify(
            Some(notify_writes_marker),
            Some(path.clone()),
            "index out of bounds".into(),
        );
        let body = fs::read_to_string(path.with_extension("notified")).unwrap();
        assert!(body.contains("index out of bounds"));
        assert!(body.contains("crash.log"));
    }

    #[test]
    fn missing_notify_callback_is_a_no_op() {
        fire_notify(None, None, "index out of bounds".into());
    }

    #[test]
    fn panicking_notify_is_swallowed() {
        fire_notify(Some(panicking_notify), None, "index out of bounds".into());
    }

    #[test]
    fn unique_path_disambiguates_an_existing_log_name() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("terminator-1.log"), b"x").unwrap();
        assert_eq!(
            unique_path(tmp.path(), "terminator-1.log")
                .file_name()
                .unwrap(),
            "terminator-1.log.1"
        );
    }

    #[test]
    fn write_report_fails_when_the_data_directory_is_a_file() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("not-a-dir");
        fs::write(&file, b"x").unwrap();
        assert!(write_report(&file, "terminator", &report()).is_none());
    }

    #[test]
    fn crash_directory_is_owner_only() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write_report(tmp.path(), "terminator", &report()).unwrap();
        let mode = fs::metadata(path.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o700);
    }

    fn spawn_isolated(test: &str, dir: &Path, capture: bool) -> std::process::Output {
        let mut cmd = Command::new(std::env::current_exe().unwrap());
        cmd.args(["--exact", test, "--nocapture"])
            .env("TERMINATOR_CRASH_TEST_CHILD", test)
            .env("TERMINATOR_CRASH_TEST_DIR", dir)
            .env_remove("CI")
            .env_remove("TERMINATOR_CRASH_DIALOG");
        if capture {
            cmd.env("TERMINATOR_CAPTURE_PATH", dir.join("capture.png"));
        } else {
            cmd.env_remove("TERMINATOR_CAPTURE_PATH");
        }
        cmd.output().unwrap()
    }

    fn crash_logs(dir: &Path) -> Vec<PathBuf> {
        fs::read_dir(dir.join(CRASHES))
            .map(|entries| {
                entries
                    .filter_map(|entry| {
                        let path = entry.ok()?.path();
                        (path.extension()? == "log").then_some(path)
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn run_panic_child(check_worker: bool) {
        let dir = PathBuf::from(std::env::var("TERMINATOR_CRASH_TEST_DIR").unwrap());
        install(CrashInstall {
            binary: "terminator",
            version: "0.0-test",
            data_dir: dir.clone(),
            notify: Some(notify_writes_marker),
        });
        if check_worker {
            let _ = std::thread::spawn(|| panic!("worker boom")).join();
            assert!(crash_logs(&dir).is_empty(), "worker panic wrote a dump");
        }
        panic!("index out of bounds: the len is 0 but the index is 0");
    }

    #[test]
    fn owner_thread_panic_writes_a_dump_and_worker_panic_does_not() {
        const NAME: &str =
            "crash::tests::owner_thread_panic_writes_a_dump_and_worker_panic_does_not";
        if std::env::var("TERMINATOR_CRASH_TEST_CHILD").as_deref() != Ok(NAME) {
            let tmp = tempfile::tempdir().unwrap();
            let output = spawn_isolated(NAME, tmp.path(), false);
            let logs = crash_logs(tmp.path());
            assert_eq!(
                logs.len(),
                1,
                "status={}\n{}\n{}",
                output.status,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(!output.status.success());
            let text = fs::read_to_string(&logs[0]).unwrap();
            assert!(text.contains("binary: terminator"));
            assert!(text.contains("index out of bounds: the len is 0 but the index is 0"));
            assert!(!text.contains("worker boom"));
            assert!(text.contains("backtrace:"));
            let body = fs::read_to_string(logs[0].with_extension("notified")).unwrap();
            let path = logs[0].display();
            assert!(body.contains(&path.to_string()));
            return;
        }
        run_panic_child(true);
    }

    #[test]
    fn fixture_capture_still_writes_a_dump_without_a_dialog() {
        const NAME: &str = "crash::tests::fixture_capture_still_writes_a_dump_without_a_dialog";
        if std::env::var("TERMINATOR_CRASH_TEST_CHILD").as_deref() != Ok(NAME) {
            let tmp = tempfile::tempdir().unwrap();
            let output = spawn_isolated(NAME, tmp.path(), true);
            let logs = crash_logs(tmp.path());
            assert_eq!(
                logs.len(),
                1,
                "status={}\n{}\n{}",
                output.status,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(!output.status.success());
            assert!(!logs[0].with_extension("notified").exists());
            let text = fs::read_to_string(&logs[0]).unwrap();
            assert!(text.contains("index out of bounds: the len is 0 but the index is 0"));
            return;
        }
        run_panic_child(false);
    }
}
