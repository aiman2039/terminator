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
    run_with(&crate::harness::root(), &output, platform, &System)
}

pub(super) fn run_with(
    root: &Path,
    output: &Path,
    platform: Platform,
    runner: &impl Runner,
) -> Result<()> {
    fs::create_dir_all(output.join("state"))?;
    fs::create_dir_all(output.join("config"))?;
    let run = |command: &mut Command| -> Result<()> {
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
        runner.run(command)
    };
    for program in ["rustc", "cargo", "nvim"] {
        run(&mut cmd(root, program, &["--version"]))?;
    }
    if matches!(platform, Platform::Mac) {
        run(&mut cmd(
            root,
            "sh",
            &["scripts/check.sh", "windows-clippy"],
        ))?;
    }
    for check in [
        "async-boundary",
        "fmt",
        "lint",
        "build",
        "clippy",
        "test",
        "audit",
        "deny",
    ] {
        run(&mut cmd(root, "sh", &["scripts/check.sh", check]))?;
    }
    if matches!(platform, Platform::Windows) {
        println!("Windows PTY and native GUI fixtures are not yet supported.");
    } else {
        run(&mut cmd(
            root,
            "cargo",
            &[
                "build",
                "--locked",
                "--bin",
                "terminator-daemon",
                "--bin",
                "terminator-hook",
            ],
        ))?;
        run(&mut cmd(root, "cargo", &["xtask", "integration"]))?;
        run(&mut cmd(root, "cargo", &["xtask", "idle-close"]))?;
        run(&mut cmd(
            root,
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
        let native = output.join("native");
        match platform {
            Platform::Linux => {
                run(&mut cmd(
                    root,
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
cargo xtask gui all --output "$RUNNER_TEMP/native"
"#,
                    ],
                ))?;
            }
            Platform::Mac => {
                run(cmd(root, "cargo", &["xtask", "gui", "all", "--output"]).arg(native))?;
            }
            Platform::Windows => unreachable!(),
        }
    }
    run(cmd(root, "cargo", &["xtask", "package", "--output"]).arg(output.join("package")))?;
    if matches!(platform, Platform::Linux) {
        run(&mut cmd(
            root,
            "cargo",
            &[
                "llvm-cov",
                "--workspace",
                "--all-features",
                "--locked",
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
    }
    ensure!(
        git(root, &["status", "--porcelain"])?.is_empty(),
        "Validation changed source files."
    );
    Ok(())
}
