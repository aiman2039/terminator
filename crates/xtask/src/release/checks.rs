use super::command::{Runner, System, cmd, git};
use anyhow::{Result, ensure};
use std::{fs, path::Path, process::Command};

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
        git(root, &["status", "--porcelain"])?.is_empty(),
        "Validation changed source files."
    );
    Ok(())
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
        ))?;
        self.integration_and_idle_close()?;
        self.native_gui(platform)?;
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
        )?;
        Ok(())
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
            integration_result?;
            idle_close_result?;
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
                        r#"
openbox > "$RUNNER_TEMP/window-manager.log" 2>&1 &
manager_pid=$!
trap 'kill "$manager_pid" 2>/dev/null || true' EXIT
sh scripts/run-built-xtask.sh gui all --output "$RUNNER_TEMP/native"
"#,
                    ],
                ))?;
            }
            Platform::Mac => {
                self.run(
                    cmd(
                        self.root,
                        "sh",
                        &["scripts/run-built-xtask.sh", "gui", "all", "--output"],
                    )
                    .arg(native),
                )?;
            }
            Platform::Windows => unreachable!(),
        }
        Ok(())
    }
}
