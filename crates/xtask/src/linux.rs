//! Explicit disposable-container setup and desktop validation. This task refuses
//! to install system packages on a host. Application packages contain no browser.
use crate::{harness::*, integration, native};
use anyhow::{Context, Result, ensure};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::Duration,
};
fn status(mut command: Command) -> Result<()> {
    ensure!(command.status()?.success(), "Validation command failed");
    Ok(())
}
fn command(program: &str, args: &[&str]) -> Command {
    let mut c = Command::new(program);
    c.args(args);
    c
}
pub fn check(browser: bool, wayland: bool) -> Result<()> {
    ensure!(
        cfg!(target_os = "linux") && Path::new("/.dockerenv").exists(),
        "linux-check is only for a disposable Linux container"
    );
    let shell = xshell::Shell::new()?;
    let _env = shell.push_env("DEBIAN_FRONTEND", "noninteractive");
    xshell::cmd!(shell, "apt-get update -qq").run()?;
    xshell::cmd!(shell,"apt-get install -y -qq --no-install-recommends pkg-config libfontconfig1-dev libasound2-dev libx11-dev libxi-dev libxcursor-dev libxrandr-dev libxinerama-dev libgl1-mesa-dev libwayland-dev libxkbcommon-dev libxkbcommon-x11-0 git curl ca-certificates neovim xvfb xauth openbox dbus-x11 xdg-desktop-portal xdg-desktop-portal-gtk lsof weston").run()?;
    if browser {
        xshell::cmd!(
            shell,
            "apt-get install -y -qq --no-install-recommends chromium"
        )
        .run()?;
    }
    let tools = artifacts().join("tools");
    fs::create_dir_all(&tools)?;
    let arch = if std::env::consts::ARCH == "aarch64" {
        "arm64"
    } else {
        "x86_64"
    };
    let bundle = format!("nvim-linux-{arch}");
    let executable = tools.join(&bundle).join("bin/nvim");
    if !executable.exists() {
        let archive = tools.join(format!("{bundle}.tar.gz"));
        let url =
            format!("https://github.com/neovim/neovim/releases/download/v0.11.5/{bundle}.tar.gz");
        let mut download = command(
            "curl",
            &[
                "-fL",
                "--retry",
                "2",
                "--connect-timeout",
                "15",
                "--max-time",
                "180",
                "-o",
            ],
        );
        download.arg(&archive).arg(url);
        status(download)?;
        let bytes = fs::File::open(&archive)?;
        tar::Archive::new(flate2::read::GzDecoder::new(bytes)).unpack(&tools)?;
    }
    ensure!(executable.exists(), "Pinned Neovim was not extracted");
    let tool_path = format!(
        "{}:{}",
        executable.parent().unwrap().display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let mut build = command(
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
    );
    build.current_dir(root());
    status(build)?;
    let mut tests = command(
        "cargo",
        &["test", "--workspace", "--all-features", "--locked"],
    );
    tests.current_dir(root()).env("PATH", &tool_path);
    status(tests)?;
    let exe = bin().join("xtask");
    let mut integration = Command::new(&exe);
    integration.arg("integration").env("PATH", &tool_path);
    status(integration)?;
    let mut x11 = command(
        "xvfb-run",
        &[
            "-a",
            "-s",
            "-screen 0 3200x2000x24",
            "dbus-run-session",
            "--",
        ],
    );
    x11.arg(&exe)
        .args(["linux-desktop", "x11"])
        .env("PATH", &tool_path)
        .env("XDG_CURRENT_DESKTOP", "GNOME")
        .env("TERMINATOR_X11_TEST", "1")
        .env("LIBGL_ALWAYS_SOFTWARE", "1");
    if browser {
        x11.arg("--browser");
    }
    status(x11)?;
    if wayland {
        let runtime = tempfile::Builder::new()
            .prefix("term-wayland-")
            .tempdir_in("/tmp")?;
        let mut wl = command("dbus-run-session", &["--"]);
        wl.arg(&exe)
            .args(["linux-desktop", "wayland"])
            .env("PATH", &tool_path)
            .env_remove("DISPLAY")
            .env("XDG_RUNTIME_DIR", runtime.path())
            .env("WAYLAND_DISPLAY", "terminator-wayland")
            .env("WINIT_UNIX_BACKEND", "wayland")
            .env("LIBGL_ALWAYS_SOFTWARE", "1");
        status(wl)?;
    }
    Ok(())
}
fn start(program: &Path, args: &[&str], log: &Path) -> Result<Process> {
    let file = fs::File::create(log)?;
    let mut c = Command::new(program);
    c.args(args)
        .stdout(file.try_clone()?)
        .stderr(file)
        .stdin(Stdio::null());
    Ok(Process(c.spawn()?))
}
pub fn desktop(backend: &str, browser: bool, case: Option<&str>) -> Result<()> {
    ensure!(
        cfg!(target_os = "linux") && Path::new("/.dockerenv").exists(),
        "Desktop suite requires the disposable Linux container"
    );
    let output = artifacts().join("native").join(backend);
    fs::create_dir_all(&output)?;
    let mut children = Vec::new();
    if backend == "x11" {
        children.push(start(
            Path::new("/usr/bin/openbox"),
            &[],
            &output.join("openbox.log"),
        )?);
        for name in ["xdg-desktop-portal-gtk", "xdg-desktop-portal"] {
            let candidates = [
                PathBuf::from("/usr/libexec").join(name),
                PathBuf::from("/usr/lib/xdg-desktop-portal").join(name),
            ];
            let path = candidates
                .iter()
                .find(|p| p.exists())
                .context("Portal executable missing")?;
            children.push(start(path, &[], &output.join(format!("{name}.log")))?);
        }
    } else {
        let help = output_command("weston", &["--help"])?;
        let render = if help.contains("--use-pixman") {
            "--use-pixman"
        } else {
            "--renderer=pixman"
        };
        children.push(start(
            Path::new("/usr/bin/weston"),
            &[
                "--backend=headless-backend.so",
                render,
                "--socket=terminator-wayland",
                "--idle-time=0",
                "--width=3200",
                "--height=2000",
            ],
            &output.join("weston.log"),
        )?);
    }
    thread::sleep(Duration::from_secs(2));
    if let Some(case) = case {
        if case == "browser" {
            integration::browser_live()?;
        } else {
            native::run(
                case,
                native::Options {
                    output: output.join("targeted"),
                    ..Default::default()
                },
            )?;
        }
        drop(children);
        return Ok(());
    }
    for scale in [1.0, 2.0] {
        let opts = native::Options {
            scale,
            output: output.join(format!("{}x", scale as u32)),
            ..Default::default()
        };
        native::run("smoke", opts.clone())?;
        native::run("images", opts.clone())?;
        let mut narrow = opts.clone();
        narrow.narrow = true;
        narrow.output = opts.output.join("narrow");
        native::run("ui-cleanup", narrow)?;
    }
    let opts = native::Options {
        output: output.join("focused"),
        ..Default::default()
    };
    if backend == "x11" {
        for case in [
            "workspace-tabs",
            "file-close",
            "inline-rename",
            "editor-lifecycle",
            "focus-editor-close",
            "terminal-actions",
            "reviews",
            "legacy-diff",
            "control",
            "external-editor",
            "window-controls",
        ] {
            native::run(case, opts.clone())?;
        }
    } else {
        native::run("control", opts)?;
    }
    if browser {
        integration::browser_live()?;
    }
    drop(children);
    Ok(())
}
fn output_command(program: &str, args: &[&str]) -> Result<String> {
    Ok(String::from_utf8(output(command(program, args))?)?)
}
