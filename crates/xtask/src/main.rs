//! Repository automation in Rust. No Python interpreter or downloaded test runner.
#![forbid(unsafe_code)]
// The harness fails the process on a broken fixture. That is an assertion, not a library error.
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod async_boundary;
mod browser_fixture;
mod harness;
mod idle_fixture;
mod integration;
mod launch;
mod linux;
mod native;
mod notification_perf;
mod package;
mod regressions;
mod zip_writer;
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::{path::PathBuf, time::Duration};

#[derive(Parser)]
#[command(about = "Terminator build, packaging, and isolated regression tasks")]
struct Args {
    #[command(subcommand)]
    task: Task,
}
#[derive(Subcommand)]
enum Task {
    /// Reject disallowed blocking adapters in GUI and async-client sources.
    AsyncBoundary,
    /// Compose local launch assets from validated native captures.
    LaunchAssets,
    /// Verify idle shells close without confirmation using isolated real shells.
    IdleClose,
    BrowserCheck,
    /// Install test dependencies and validate inside a disposable Linux container.
    LinuxCheck {
        #[arg(long)]
        browser: bool,
        #[arg(long)]
        wayland: bool,
    },
    #[command(hide = true)]
    LinuxDesktop {
        #[arg(value_parser=["x11","wayland"])]
        backend: String,
        #[arg(long)]
        case: Option<String>,
        #[arg(long)]
        browser: bool,
    },
    /// Package local binaries; does not install or publish.
    Package {
        #[arg(long)]
        debug: bool,
        /// Emit Cargo HTML timing reports for the application build.
        #[arg(long)]
        timings: bool,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Build and package a local DMG without installing it.
    LocalDmg {
        #[arg(long)]
        release: bool,
        #[arg(long)]
        styled: bool,
        #[arg(long)]
        output: Option<PathBuf>,
        #[arg(long)]
        timings: bool,
    },
    /// Create a drag-to-Applications DMG from a prebuilt app (requires create-dmg).
    Dmg {
        #[arg(long)]
        app: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    /// Embed Sparkle into a prebuilt Apple Silicon app.
    Assemble {
        #[arg(long)]
        app: PathBuf,
        #[arg(long)]
        sparkle: PathBuf,
        #[arg(long)]
        build_number: u64,
        #[arg(long)]
        output: PathBuf,
    },
    /// Real-PTY, auth, hooks, reconnect, and daemon-recovery checks.
    Integration,
    /// Native renderer and input fixtures. Requires a desktop and test-support build.
    Gui {
        #[arg(default_value = "all")]
        case: String,
        #[arg(long, default_value = "1")]
        scale: f32,
        #[arg(long)]
        narrow: bool,
        #[arg(long)]
        output: Option<PathBuf>,
        #[arg(long, default_value = "50")]
        sessions: usize,
        #[arg(long, default_value = "5")]
        seconds: u64,
    },
    /// Bounded daemon transport/PTY load (does not measure GUI FPS).
    Load {
        #[arg(long, default_value = "10")]
        seconds: u64,
        #[arg(long)]
        conditional: bool,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Compare two load-report JSON files.
    CompareLoad {
        before: PathBuf,
        after: PathBuf,
    },
    /// Isolated A/B benchmark of the daemon notification path.
    NotificationPerf {
        /// Baseline daemon binary (defaults to the built debug daemon).
        #[arg(long)]
        baseline_bin: Option<PathBuf>,
        /// Candidate daemon binary (defaults to the built debug daemon).
        #[arg(long)]
        candidate_bin: Option<PathBuf>,
        #[arg(long, default_value = "60")]
        seconds: u64,
        #[arg(long, default_value = "10")]
        warmup: u64,
        #[arg(long, default_value = "3")]
        repetitions: u32,
        #[arg(long, default_value = "20")]
        burst: usize,
        #[arg(long, default_value = "all")]
        cases: String,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Count Git/PR subprocesses in an isolated native refresh fixture.
    CommandCounts {
        #[arg(long, default_value = "5")]
        seconds: u64,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Opt-in real Muse offline echo fixture (no model calls).
    MuseEcho {
        #[arg(long)]
        muse: PathBuf,
    },
}
fn main() -> Result<()> {
    if std::env::args_os()
        .next()
        .and_then(|p| {
            PathBuf::from(p)
                .file_name()
                .map(std::borrow::ToOwned::to_owned)
        })
        .is_some_and(|s| s == "terminator-test-shell")
    {
        // Native-input fixtures use a non-executing PTY sink, so desktop typing
        // can never become shell commands or appear in their captures.
        #[cfg(unix)]
        {
            let mut attributes = rustix::termios::tcgetattr(std::io::stdin())
                .map_err(|_| anyhow::anyhow!("Fixture sink requires a PTY"))?;
            attributes
                .local_modes
                .remove(rustix::termios::LocalModes::ECHO | rustix::termios::LocalModes::ECHONL);
            rustix::termios::tcsetattr(
                std::io::stdin(),
                rustix::termios::OptionalActions::Now,
                &attributes,
            )
            .map_err(|_| anyhow::anyhow!("Cannot disable fixture echo"))?;
        }
        #[cfg(not(unix))]
        anyhow::ensure!(
            false,
            "Fixture sink requires a Unix PTY; Windows fixtures are not implemented yet"
        );
        std::io::copy(&mut std::io::stdin(), &mut std::io::sink())?;
        return Ok(());
    }

    if std::env::args_os()
        .next()
        .and_then(|p| {
            PathBuf::from(p)
                .file_name()
                .map(std::borrow::ToOwned::to_owned)
        })
        .is_some_and(|name| name == "git" || name == "ps" || name == "gh")
    {
        return integration::git_shim();
    }
    match Args::parse().task {
        Task::AsyncBoundary => async_boundary::run(),
        Task::LaunchAssets => launch::run(),
        Task::IdleClose => idle_fixture::run(),
        Task::BrowserCheck => browser_fixture::run(),
        Task::LinuxCheck { browser, wayland } => linux::check(browser, wayland),
        Task::LinuxDesktop {
            backend,
            browser,
            case,
        } => linux::desktop(&backend, browser, case.as_deref()),
        Task::Package {
            debug,
            timings,
            output,
        } => package::run(debug, timings, output),
        Task::LocalDmg {
            release,
            styled,
            output,
            timings,
        } => package::local_dmg(release, styled, output, timings),
        Task::Dmg { app, output } => package::dmg(&app, &output),
        Task::Assemble {
            app,
            sparkle,
            build_number,
            output,
        } => package::assemble(package::AssembleInput {
            app: &app,
            sparkle: &sparkle,
            build_number,
            destination: &output,
        }),
        Task::Integration => {
            integration::run()?;
            integration::controls()?;
            integration::hook_controls()?;
            regressions::run()
        }
        Task::Gui {
            case,
            scale,
            narrow,
            output,
            sessions,
            seconds,
        } => native::run(
            &case,
            native::Options {
                scale,
                narrow,
                output: output.unwrap_or(harness::artifacts().join("native")),
                sessions,
                seconds,
            },
        ),
        Task::Load {
            seconds,
            output,
            conditional,
        } => integration::load(Duration::from_secs(seconds), output, conditional),
        Task::CompareLoad { before, after } => {
            let a: serde_json::Value = serde_json::from_slice(&std::fs::read(before)?)?;
            let b: serde_json::Value = serde_json::from_slice(&std::fs::read(after)?)?;
            let mut comparison = serde_json::Map::new();
            for key in ["snapshot_p95_ms", "daemon_peak_rss_kib", "bytes_received"] {
                let old = a
                    .get(key)
                    .and_then(serde_json::Value::as_f64)
                    .with_context(|| format!("Missing baseline metric {key}"))?;
                let new = b
                    .get(key)
                    .and_then(serde_json::Value::as_f64)
                    .with_context(|| format!("Missing current metric {key}"))?;
                comparison.insert(key.into(),serde_json::json!({"before":old,"after":new,"ratio":if old==0.0 {None}else{Some(new/old)}}));
            }
            println!("{}", serde_json::to_string_pretty(&comparison)?);
            Ok(())
        }
        Task::NotificationPerf {
            baseline_bin,
            candidate_bin,
            seconds,
            warmup,
            repetitions,
            burst,
            cases,
            output,
        } => notification_perf::run(notification_perf::Options {
            baseline_bin,
            candidate_bin,
            seconds,
            warmup,
            repetitions,
            burst,
            cases,
            output,
        }),
        Task::CommandCounts { seconds, output } => integration::command_counts(seconds, output),
        Task::MuseEcho { muse } => integration::muse(&muse),
    }
}
