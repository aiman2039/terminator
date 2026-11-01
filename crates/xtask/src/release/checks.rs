use super::command::{Runner, System, cmd, git};
use anyhow::{Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs, path::Path, process::Command, sync::Mutex, time::Instant};

/// GUI fixtures click on a timeline and use software rendering. With the
/// default 8 workers beside the Linux container, about a third of the
/// cases miss their timing; 4 keeps most of the speed with fewer retries.
const RELEASE_GUI_JOBS: &str = "4";

/// Steps that need only the built binaries. A failure of one of them can be
/// load, so the release reruns just these steps (`--only`) after a parallel
/// failure. Other steps fail the same way every time.
pub(super) const FIXTURE_STEPS: [&str; 4] = ["integration", "idle-close", "gui", "package"];

/// The record file each release-check run writes into its output directory.
pub(super) const STEPS_FILE: &str = "steps.json";

/// Lines of a step error kept in reports.
const ERROR_LINES: usize = 12;

#[derive(Clone, Copy)]
pub(super) enum Platform {
    Mac,
    Linux,
    Windows,
}

/// The time and result of one release-check step.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Step {
    pub(super) name: String,
    pub(super) status: String,
    pub(super) seconds: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) error: Option<String>,
}

pub fn run(output: &Path, only: &[String]) -> Result<()> {
    let output = std::path::absolute(output)?;
    let platform = if cfg!(target_os = "macos") {
        Platform::Mac
    } else if cfg!(target_os = "linux") {
        Platform::Linux
    } else {
        Platform::Windows
    };
    run_with(RunWithInput {
        root: &crate::harness::root(),
        output: &output,
        platform,
        runner: &System,
        only,
    })
}
/// Runs release-check commands with the isolated release environment.
struct Checks<'a, R> {
    root: &'a Path,
    output: &'a Path,
    runner: &'a R,
    steps: Mutex<Vec<Step>>,
}

pub(super) struct RunWithInput<'a, R> {
    pub(super) root: &'a Path,
    pub(super) output: &'a Path,
    pub(super) platform: Platform,
    pub(super) runner: &'a R,
    /// Run only these fixture steps (after the build); empty runs all steps.
    pub(super) only: &'a [String],
}

pub(super) fn run_with(input: RunWithInput<'_, impl Runner>) -> Result<()> {
    let RunWithInput {
        root,
        output,
        platform,
        runner,
        only,
    } = input;
    for name in only {
        ensure!(
            FIXTURE_STEPS.contains(&name.as_str()),
            "Only fixture steps can run alone: {name}"
        );
    }
    let source = source_state(root)?;
    fs::create_dir_all(output.join("state"))?;
    fs::create_dir_all(output.join("config"))?;
    let checks = Checks {
        root,
        output,
        runner,
        steps: Mutex::new(Vec::new()),
    };
    for program in ["rustc", "cargo", "nvim"] {
        checks.run(&mut cmd(root, program, &["--version"]))?;
    }
    if only.is_empty() {
        checks.static_and_unit(platform)?;
    }
    if matches!(platform, Platform::Windows) {
        println!("Windows PTY and native GUI fixtures are not yet supported.");
        checks.step("package", || {
            checks.run(
                cmd(root, "cargo", &["xtask", "package", "--debug", "--output"])
                    .arg(output.join("package")),
            )
        })?;
    } else {
        checks.fixtures(platform, only)?;
    }
    ensure!(
        source_state(root)? == source,
        "Validation changed source files."
    );
    checks.verdict()
}

/// Read-only comparison also works with a dirty checkout mounted in Docker.
fn source_state(root: &Path) -> Result<Vec<u8>> {
    let mut hash = Sha256::new();
    hash.update(git(root, &["status", "--porcelain"])?);
    hash.update(git(root, &["diff", "HEAD", "--binary"])?);
    for name in git(root, &["ls-files", "--others", "--exclude-standard", "-z"])?
        .split('\0')
        .filter(|name| !name.is_empty())
    {
        hash.update(name.len().to_le_bytes());
        hash.update(name);
        let path = root.join(name);
        let contents = if fs::symlink_metadata(&path)?.file_type().is_symlink() {
            fs::read_link(&path)?
                .as_os_str()
                .as_encoded_bytes()
                .to_vec()
        } else {
            fs::read(&path)?
        };
        hash.update(contents.len().to_le_bytes());
        hash.update(contents);
    }
    Ok(hash.finalize().to_vec())
}

/// The first lines of an error, for step records and the final report.
pub(super) fn error_head(error: &anyhow::Error) -> String {
    format!("{error:#}")
        .lines()
        .take(ERROR_LINES)
        .collect::<Vec<_>>()
        .join("\n")
}

/// One report line per failed step.
pub(super) fn failure_report(steps: &[Step]) -> Option<String> {
    let failed: Vec<_> = steps.iter().filter(|s| s.status == "failed").collect();
    if failed.is_empty() {
        return None;
    }
    let mut report = format!("{} release check step(s) failed:", failed.len());
    for step in failed {
        report.push_str(&format!(
            "\n- {}: {}",
            step.name,
            step.error.as_deref().unwrap_or("failed")
        ));
    }
    Some(report)
}

impl<R: Runner> Checks<'_, R> {
    fn run(&self, command: &mut Command) -> Result<()> {
        let output = self.output;
        command
            .env("RUNNER_TEMP", output)
            .env("CARGO_HUSKY_DONT_INSTALL_HOOKS", "1")
            .env("TERMINATOR_DATA_DIR", output.join("state"))
            .env("TERMINATOR_CONFIG_DIR", output.join("config"))
            .env("TERMINATOR_RUNTIME_DIR", output.join("runtime"))
            .env_remove("TERMINATOR_SESSION_ID")
            .env_remove("TERMINATOR_TEST_BIN_DIR")
            .env("TERMINATOR_FIXTURE_RENDERER", "glow")
            .env("LIBGL_ALWAYS_SOFTWARE", "1");
        println!("==> {command:?}");
        self.runner.run(command)
    }
    /// Record a finished step and rewrite the step file.
    fn record(&self, name: &str, started: Instant, result: &Result<()>) -> Result<()> {
        let mut steps = self
            .steps
            .lock()
            .map_err(|_| anyhow::anyhow!("Release steps lock poisoned"))?;
        steps.push(Step {
            name: name.into(),
            status: if result.is_ok() { "passed" } else { "failed" }.into(),
            seconds: started.elapsed().as_secs_f64(),
            error: result.as_ref().err().map(error_head),
        });
        fs::write(
            self.output.join(STEPS_FILE),
            serde_json::to_vec_pretty(&*steps)?,
        )?;
        Ok(())
    }
    /// Run and record one step; returns the step's own result.
    fn step(&self, name: &str, work: impl FnOnce() -> Result<()>) -> Result<()> {
        let started = Instant::now();
        let result = work();
        self.record(name, started, &result)?;
        result
    }
    fn check_script(&self, check: &str) -> Result<()> {
        self.step(check, || {
            self.run(&mut cmd(self.root, "sh", &["scripts/check.sh", check]))
        })
    }
    /// Static checks run to the end. Clippy or unit-test failures stop the
    /// release: nothing later can pass without a clean build.
    fn static_and_unit(&self, platform: Platform) -> Result<()> {
        if matches!(platform, Platform::Mac) {
            for check in ["windows-clippy", "async-boundary", "fmt", "audit", "deny"] {
                let _ = self.check_script(check);
            }
        }
        // `clippy` type-checks everything `cargo check` does, and the test /
        // test-support / package steps compile all test and binary targets, so
        // the release flow does not run `lint` or `build` twice.
        if self.check_script("clippy").is_err() || self.check_script("test").is_err() {
            return self.verdict();
        }
        Ok(())
    }
    /// Fail with every failed step, or pass when none failed.
    fn verdict(&self) -> Result<()> {
        let steps = self
            .steps
            .lock()
            .map_err(|_| anyhow::anyhow!("Release steps lock poisoned"))?;
        if let Some(report) = failure_report(&steps) {
            bail!("{report}");
        }
        Ok(())
    }
}

impl<R: Runner + Sync> Checks<'_, R> {
    /// Build test-support binaries, then run every selected fixture step,
    /// even after another one fails.
    fn fixtures(&self, platform: Platform, only: &[String]) -> Result<()> {
        let selected = |name: &str| only.is_empty() || only.iter().any(|o| o == name);
        self.step("build", || {
            self.run(&mut cmd(
                self.root,
                "cargo",
                &[
                    "build",
                    "--workspace",
                    "--bins",
                    "--examples",
                    "--features",
                    "terminator/test-support",
                    "--locked",
                ],
            ))
        })?;
        let real_pty: Vec<&str> = ["integration", "idle-close"]
            .into_iter()
            .filter(|name| selected(name))
            .collect();
        self.real_pty(&real_pty)?;
        if selected("gui") {
            let _ = self.step("gui", || self.native_gui(platform));
        }
        if selected("package") {
            let _ = self.step("package", || self.package());
        }
        Ok(())
    }
    fn package(&self) -> Result<()> {
        self.run(
            cmd(
                self.root,
                "sh",
                &[
                    "scripts/run-built-xtask.sh",
                    "package",
                    "--debug",
                    "--output",
                ],
            )
            .arg(self.output.join("package")),
        )
    }
    fn xtask(&self, check: &str) -> Result<()> {
        self.run(&mut cmd(
            self.root,
            "sh",
            &["scripts/run-built-xtask.sh", check],
        ))
    }
    /// Run the real-PTY checks in parallel. These use short timed waits and
    /// share the host with each other and the Linux container, so a failed
    /// check runs once more alone; it fails only when that run fails too.
    fn real_pty(&self, checks: &[&str]) -> Result<()> {
        let started = Instant::now();
        let first: Vec<(&str, Result<()>)> = std::thread::scope(|scope| {
            let tasks: Vec<_> = checks
                .iter()
                .map(|&check| (check, scope.spawn(move || self.xtask(check))))
                .collect();
            tasks
                .into_iter()
                .map(|(check, task)| {
                    let result = task
                        .join()
                        .unwrap_or_else(|_| Err(anyhow::anyhow!("{check} check panicked")));
                    (check, result)
                })
                .collect()
        });
        for (check, result) in first {
            let result = match result {
                Err(error) => {
                    println!("Retrying {check} alone after a failure: {error:#}");
                    self.xtask(check)
                }
                passed => passed,
            };
            self.record(check, started, &result)?;
        }
        Ok(())
    }
    fn native_gui(&self, platform: Platform) -> Result<()> {
        let native = self.output.join("native");
        match platform {
            Platform::Linux => self.run(&mut cmd(
                self.root,
                "xvfb-run",
                &[
                    "--auto-servernum",
                    "--server-args=-screen 0 3200x2000x24",
                    "dbus-run-session",
                    "--",
                    "bash",
                    "-euo",
                    "pipefail",
                    "-c",
                    &format!(
                        r#"
openbox > "$RUNNER_TEMP/window-manager.log" 2>&1 &
manager_pid=$!
trap 'kill "$manager_pid" 2>/dev/null || true' EXIT
sh scripts/run-built-xtask.sh gui all --jobs {RELEASE_GUI_JOBS} --output "$RUNNER_TEMP/native"
"#
                    ),
                ],
            )),
            Platform::Mac => self.run(
                cmd(
                    self.root,
                    "sh",
                    &[
                        "scripts/run-built-xtask.sh",
                        "gui",
                        "all",
                        "--jobs",
                        RELEASE_GUI_JOBS,
                        "--output",
                    ],
                )
                .arg(native),
            ),
            Platform::Windows => bail!("Windows has no native GUI fixtures"),
        }
    }
}
