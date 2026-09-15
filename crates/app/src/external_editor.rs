//! Shell-free external launching. Waiting and stderr draining never occupy the GUI
//! or its settings/refresh worker; a long-lived editor is never timed out or killed.
use anyhow::{Context, Result};
use std::{
    io::Read,
    path::Path,
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    thread,
};
use terminator_core::{Settings, find_executable};

pub const PRESETS: [&str; 6] = [
    "System default",
    "VS Code",
    "Cursor",
    "RustRover",
    "Zed",
    "Custom",
];
pub const CUSTOM: usize = 5;
pub fn preset(index: usize) -> (String, Vec<String>) {
    let apps = ["", "Visual Studio Code", "Cursor", "RustRover", "Zed"];
    let commands = ["xdg-open", "code", "cursor", "rustrover", "zed"];
    if cfg!(target_os = "macos") {
        (
            "open".into(),
            if index == 0 {
                vec![]
            } else {
                vec!["-a".into(), apps[index].into()]
            },
        )
    } else {
        (commands[index].into(), vec![])
    }
}
pub fn selected(settings: &Settings) -> usize {
    (0..CUSTOM)
        .find(|&i| {
            preset(i)
                == (
                    settings.external_editor.clone(),
                    settings.external_args.clone(),
                )
        })
        .unwrap_or(CUSTOM)
}
/// macOS `open file.png` asks Launch Services for a UTI handler and can fail for
/// owner-only temp files (`NSOSStatusErrorDomain` -10661). Naming Preview skips
/// that lookup. Linux `xdg-open` does not have the same failure mode.
pub fn image_opener() -> (String, Vec<String>) {
    if cfg!(target_os = "macos") {
        (
            "open".into(),
            vec!["-a".into(), "Preview".into(), "--".into()],
        )
    } else {
        ("xdg-open".into(), vec![])
    }
}
fn command(program: &str, args: &[String], path: &Path) -> Result<Command> {
    let executable = find_executable(program)
        .with_context(|| format!("External editor executable not found: {program}"))?;
    let path = std::path::absolute(path).context("Resolve external editor file path")?;
    let mut command = Command::new(executable);
    command
        .args(args)
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    Ok(command)
}
pub fn launch(
    program: &str,
    args: &[String],
    path: &Path,
    done: impl FnOnce(Result<()>) + Send + 'static,
) -> Result<()> {
    let mut child = command(program, args, path)?
        .spawn()
        .with_context(|| format!("Start external editor {program}"))?;
    let mut stderr = child.stderr.take().expect("piped stderr");
    let diagnostic = Arc::new(Mutex::new(Vec::new()));
    let output = diagnostic.clone();
    let (drained, drain_complete) = std::sync::mpsc::channel();
    thread::spawn(move || {
        let mut bytes = [0; 1024];
        loop {
            match stderr.read(&mut bytes) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let mut output = output.lock().unwrap();
                    let count = n.min(4096usize.saturating_sub(output.len()));
                    output.extend_from_slice(&bytes[..count]);
                }
            }
        }
        let _ = drained.send(());
    });
    let program = program.to_owned();
    thread::spawn(move || {
        let result = child
            .wait()
            .with_context(|| format!("Wait for external editor {program}"))
            .and_then(|status| {
                if status.success() {
                    return Ok(());
                }
                let _ = drain_complete.recv_timeout(std::time::Duration::from_millis(100));
                // Do not join the pipe reader: a GUI editor's descendant may inherit it.
                let bytes = diagnostic.lock().unwrap();
                let message: String = String::from_utf8_lossy(&bytes)
                    .chars()
                    .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
                    .take(2048)
                    .collect();
                anyhow::bail!(
                    "External editor {program} exited with {status}: {}",
                    message.trim()
                );
            });
        done(result);
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::mpsc,
        time::{Duration, Instant},
    };
    #[test]
    fn arguments_and_absolute_path_are_separate_and_literal() {
        let args = vec!["--wait".into(), "a b; $(touch unwanted)".into(), "".into()];
        let cmd = command("/bin/echo", &args, Path::new("a b;$x.rs")).unwrap();
        let actual: Vec<_> = cmd.get_args().map(|a| a.to_os_string()).collect();
        assert_eq!(
            &actual[..3],
            &args
                .iter()
                .map(std::ffi::OsString::from)
                .collect::<Vec<_>>()
        );
        assert_eq!(actual[3], std::path::absolute("a b;$x.rs").unwrap());
    }
    #[test]
    fn missing_executable_and_spawn_failure_are_reported() {
        assert!(
            launch(
                "terminator-nonexistent-fixture-editor",
                &[],
                Path::new("file"),
                |_| {}
            )
            .unwrap_err()
            .to_string()
            .contains("not found")
        );
        let dir = tempfile::tempdir().unwrap();
        let invalid = dir.path().join("invalid");
        std::fs::write(&invalid, "#!/missing/fixture/interpreter\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&invalid, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(launch(invalid.to_str().unwrap(), &[], Path::new("file"), |_| {}).is_err());
    }
    #[test]
    fn failing_exit_is_reported_and_long_running_editor_does_not_block() {
        let (tx, rx) = mpsc::channel();
        let start = Instant::now();
        launch(
            "/bin/sh",
            &[
                "-c".into(),
                "sleep 1; echo fixture-failure >&2; exit 7".into(),
            ],
            Path::new("file"),
            move |result| {
                tx.send(result.map_err(|e| e.to_string())).unwrap();
            },
        )
        .unwrap();
        assert!(start.elapsed() < Duration::from_millis(500));
        assert!(rx.recv_timeout(Duration::from_millis(100)).is_err());
        let error = rx
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap_err();
        assert!(error.contains('7'));
        assert!(error.contains("fixture-failure"));
    }
    #[test]
    fn noisy_failure_is_drained_but_diagnostic_is_bounded() {
        let (tx, rx) = mpsc::channel();
        launch(
            "/bin/sh",
            &[
                "-c".into(),
                "i=0; while [ $i -lt 3000 ]; do echo diagnostic-line >&2; i=$((i+1)); done; exit 2"
                    .into(),
            ],
            Path::new("file"),
            move |r| {
                tx.send(r.map_err(|e| e.to_string())).unwrap();
            },
        )
        .unwrap();
        let error = rx
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap_err();
        assert!(error.contains("diagnostic-line"));
        assert!(error.len() < 2200);
    }
    #[test]
    fn presets_round_trip_and_unmatched_settings_stay_custom() {
        let mut settings = Settings::default();
        for i in 0..CUSTOM {
            (settings.external_editor, settings.external_args) = preset(i);
            assert_eq!(selected(&settings), i);
        }
        settings.external_args.push("--my-config".into());
        assert_eq!(selected(&settings), CUSTOM);
    }
    #[test]
    fn images_use_preview_and_source_files_keep_the_configured_editor() {
        assert!(crate::image_preview::supported(Path::new("shot.PNG")));
        assert!(!crate::image_preview::supported(Path::new("src/main.rs")));
        let (program, args) = image_opener();
        #[cfg(target_os = "macos")]
        {
            assert_eq!(program, "open");
            assert_eq!(args, vec!["-a", "Preview", "--"]);
            let cmd = command(&program, &args, Path::new("paste.png")).unwrap();
            let actual: Vec<_> = cmd
                .get_args()
                .map(|a| a.to_string_lossy().into_owned())
                .collect();
            assert_eq!(&actual[..3], &["-a", "Preview", "--"]);
            assert!(actual[3].ends_with("paste.png"));
        }
        #[cfg(not(target_os = "macos"))]
        {
            assert_eq!(program, "xdg-open");
            assert!(args.is_empty());
        }
    }
}
