use super::command::{Runner, System, cmd, git};
use anyhow::{Result, ensure};
use sha2::{Digest, Sha256};
use std::{fs, path::Path, process::Command};

/// GUI fixtures click on a timeline and use software rendering. With the
/// default 8 workers beside the Linux container, about a third of the
/// cases miss their timing; 4 keeps most of the speed with fewer retries.
const RELEASE_GUI_JOBS: &str = "4";

#[derive(Clone, Copy)]
pub(super) enum Platform {
    Mac,
    Linux,
    Windows,
}

pub fn run(output: &Path) -> Result<()> {
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
    })
}
/// Runs release-check commands with the isolated release environment.
struct Checks<'a, R> {
    root: &'a Path,
    output: &'a Path,
    runner: &'a R,
}

pub(super) struct RunWithInput<'a, R> {
    pub(super) root: &'a Path,
    pub(super) output: &'a Path,
    pub(super) platform: Platform,
    pub(super) runner: &'a R,
}

pub(super) fn run_with(input: RunWithInput<'_, impl Runner>) -> Result<()> {
    let RunWithInput {
        root,
        output,
        platform,
        runner,
    } = input;
    let source = source_state(root)?;
    fs::create_dir_all(output.join("state"))?;
    fs::create_dir_all(output.join("config"))?;
    let checks = Checks {
        root,
        output,
        runner,
    };
    for program in ["rustc", "cargo", "nvim"] {
        checks.run(&mut cmd(root, program, &["--version"]))?;
    }
    if matches!(platform, Platform::Mac) {
        checks.mac_only()?;
    }
    // `clippy` type-checks everything `cargo check` does, and the test /
    // test-support / package steps compile all test and binary targets, so the
    // release flow does not run `lint` or `build` twice over the same targets.
    checks.run(&mut cmd(root, "sh", &["scripts/check.sh", "clippy"]))?;
    checks.unit_tests(platform)?;
    if matches!(platform, Platform::Windows) {
        println!("Windows PTY and native GUI fixtures are not yet supported.");
        checks.run(
            cmd(root, "cargo", &["xtask", "package", "--debug", "--output"])
                .arg(output.join("package")),
        )?;
    } else {
        checks.fixtures(platform)?;
    }
    ensure!(
        source_state(root)? == source,
        "Validation changed source files."
    );
    Ok(())
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
    fn mac_only(&self) -> Result<()> {
        self.run(&mut cmd(
            self.root,
            "sh",
            &["scripts/check.sh", "windows-clippy"],
        ))?;
        for check in ["async-boundary", "fmt", "audit", "deny"] {
            self.run(&mut cmd(self.root, "sh", &["scripts/check.sh", check]))?;
        }
        Ok(())
    }
    fn unit_tests(&self, platform: Platform) -> Result<()> {
        if matches!(platform, Platform::Linux) {
            // Run unit tests and coverage in one pass on Linux via cargo-llvm-cov + nextest.
            // Instrumented GUI test links use substantial memory. Serialize builds
            // so Docker hosts with many CPUs do not run these links together.
            self.run(&mut cmd(
                self.root,
                "cargo",
                &[
                    "llvm-cov",
                    "nextest",
                    "--build-jobs",
                    "1",
                    "--workspace",
                    "--exclude",
                    "xtask",
                    "--all-features",
                    "--locked",
                    "--profile",
                    "ci",
                    "--summary-only",
                    "--fail-under-lines",
                    "50",
                    "--",
                    "--skip",
                    "signals::tests::hangup_ends_a_spawned_process_group",
                    "--skip",
                    "navigation_tests::reconnect_clears_transport_errors_but_preserves_failed_operations",
                ],
            ))?;
        } else {
            self.run(&mut cmd(self.root, "sh", &["scripts/check.sh", "test"]))?;
        }
        Ok(())
    }
}

impl<R: Runner + Sync> Checks<'_, R> {
    /// Build test-support binaries, then run integration, idle-close, native
    /// GUI and package checks.
    fn fixtures(&self, platform: Platform) -> Result<()> {
        let mut args = vec![
            "build",
            "--workspace",
            "--bins",
            "--examples",
            "--features",
            "terminator/test-support",
            "--locked",
        ];
        if matches!(platform, Platform::Linux) {
            // The GUI binaries and examples each need gigabytes to link with
            // debug info. With one job per CPU, parallel links exhaust the
            // Docker VM's memory and the linker is killed (signal 9).
            args.extend(["--jobs", "2"]);
        }
        self.run(&mut cmd(self.root, "cargo", &args))?;
        // These checks are independent once the binaries exist. Run them all
        // and report every failure together, so one release run shows all
        // problems instead of one per run.
        let failures: Vec<String> = [
            ("integration/idle-close", self.integration_and_idle_close()),
            ("native GUI", self.native_gui(platform)),
            ("package", self.package()),
        ]
        .into_iter()
        .filter_map(|(name, result)| result.err().map(|error| format!("{name}: {error:#}")))
        .collect();
        ensure!(
            failures.is_empty(),
            "{} fixture check(s) failed:\n{}",
            failures.len(),
            failures.join("\n")
        );
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
    fn integration_and_idle_close(&self) -> Result<()> {
        std::thread::scope(|scope| -> Result<()> {
            let integration = scope.spawn(|| {
                self.run(&mut cmd(
                    self.root,
                    "sh",
                    &["scripts/run-built-xtask.sh", "integration"],
                ))
            });
            let idle_close = scope.spawn(|| {
                self.run(&mut cmd(
                    self.root,
                    "sh",
                    &["scripts/run-built-xtask.sh", "idle-close"],
                ))
            });
            let integration_result = integration
                .join()
                .map_err(|_| anyhow::anyhow!("integration check panicked"))?;
            let idle_close_result = idle_close
                .join()
                .map_err(|_| anyhow::anyhow!("idle-close check panicked"))?;
            // These real-PTY checks use short timed waits and share the host
            // with each other and the Linux container. A failed check runs
            // once more alone; it fails only when that run fails too.
            let mut failures = Vec::new();
            for (check, result) in [
                ("integration", integration_result),
                ("idle-close", idle_close_result),
            ] {
                if let Err(first) = result {
                    println!("Retrying {check} alone after a failure: {first:#}");
                    if let Err(error) = self.run(&mut cmd(
                        self.root,
                        "sh",
                        &["scripts/run-built-xtask.sh", check],
                    )) {
                        failures.push(format!("{check}: {error:#}"));
                    }
                }
            }
            ensure!(failures.is_empty(), "{}", failures.join("\n"));
            Ok(())
        })?;
        Ok(())
    }
    fn native_gui(&self, platform: Platform) -> Result<()> {
        let native = self.output.join("native");
        match platform {
            Platform::Linux => {
                self.run(&mut cmd(
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
                ))?;
            }
            Platform::Mac => {
                self.run(
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
                )?;
            }
            Platform::Windows => unreachable!(),
        }
        Ok(())
    }
}
