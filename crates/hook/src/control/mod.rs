//! User-invoked high-level controls. Never called by observational agent hooks.
mod lookup;
mod projects;
mod sessions;

use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use lookup::Ctl;
use std::path::PathBuf;
use terminator_core::*;
#[derive(Parser)]
#[command(
    about = "Explicit workspace, terminal, worktree, metadata, and external-browser controls"
)]
struct Cli {
    #[arg(long, global = true)]
    data_dir: Option<PathBuf>,
    #[arg(long, global = true)]
    runtime_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Control,
}
#[derive(Subcommand)]
enum Control {
    List,
    /// Close the GUI and shut down the daemon after flushing saved history.
    Shutdown(ShutdownArgs),
    AddRemoteProject(AddRemoteProjectArgs),
    AddProject {
        path: PathBuf,
    },
    /// Create a shell/editor in a new tab. --background works without a GUI.
    Create(CreateArgs),
    Split(SplitArgs),
    Focus {
        session: String,
    },
    /// Send literal text to a session. --stdin reads at most 1 MiB.
    Send(SendArgs),
    Read {
        session: String,
        #[arg(long)]
        screen: bool,
    },
    OpenFile {
        project: String,
        path: PathBuf,
        #[arg(long)]
        text: bool,
    },
    Notify(NotifyArgs),
    DismissNotice {
        id: String,
    },
    Metadata {
        session: String,
        #[arg(long)]
        pr: bool,
    },
    Worktree {
        #[command(subcommand)]
        command: Worktree,
    },
    UiSnapshot,
    Window {
        #[arg(value_parser=["minimize","maximize","restore","focus","close"])]
        action: String,
    },
    Browser {
        #[command(subcommand)]
        command: super::browser::BrowserCommand,
    },
}
#[derive(Args)]
struct ShutdownArgs {
    /// Terminate every live session, discarding unsaved editor buffers and stopping jobs.
    #[arg(long)]
    stop_all: bool,
    /// Maximum seconds to wait for each cleanup phase; never force-kills a process.
    #[arg(long, default_value = "30", value_parser = clap::value_parser!(u64).range(1..=300))]
    timeout: u64,
    /// After shutdown, reopen this installation's GUI.
    #[arg(long)]
    relaunch: bool,
    /// GUI executable to reopen. Defaults to terminator beside this helper.
    #[arg(long, requires = "relaunch")]
    exe: Option<PathBuf>,
    /// Exact inventory approved in the GUI.
    #[arg(long, hide = true, requires = "stop_all")]
    confirmed_inventory: Option<String>,
}
#[derive(Args)]
struct AddRemoteProjectArgs {
    #[arg(long)]
    host: String,
    /// Omit to use `~/.ssh/config` (or 22).
    #[arg(long)]
    port: Option<u16>,
    #[arg(long, default_value = "")]
    user: String,
    #[arg(long)]
    root: String,
    #[arg(long)]
    name: String,
    #[arg(long)]
    identity: Option<PathBuf>,
    #[arg(long)]
    known_hosts: Option<PathBuf>,
}
#[derive(Args)]
struct CreateArgs {
    project: String,
    #[arg(long)]
    cwd: Option<PathBuf>,
    #[arg(long)]
    file: Option<PathBuf>,
    #[arg(long)]
    background: bool,
}
#[derive(Args)]
struct SplitArgs {
    session: String,
    #[arg(value_parser=["left","right","up","down"])]
    direction: String,
}
#[derive(Args)]
struct NotifyArgs {
    session: String,
    body: String,
    #[arg(long, default_value = "Terminal")]
    title: String,
}
#[derive(Args)]
struct SendArgs {
    session: String,
    text: Option<String>,
    #[arg(long)]
    stdin: bool,
    #[arg(long)]
    enter: bool,
}
#[derive(Subcommand)]
enum Worktree {
    List {
        project: String,
    },
    Add {
        project: String,
        path: PathBuf,
        #[arg(long)]
        branch: Option<String>,
        #[arg(long, default_value = "HEAD")]
        start: String,
    },
    /// Git refuses dirty/locked checkouts. Live Terminator sessions must be stopped.
    Remove {
        project: String,
    },
}
pub fn run(args: &[String]) -> Result<()> {
    let cli = Cli::parse_from(
        std::iter::once("terminator-hook ctl".to_owned()).chain(args.iter().cloned()),
    );
    let paths = cli_paths(cli.data_dir, cli.runtime_dir)?;
    if let Control::Browser { command } = cli.command {
        return super::browser::run(command);
    }
    let ctl = Ctl::connect(&paths)?;
    let value = match cli.command {
        Control::Shutdown(args) => ctl.shutdown(args)?,
        Control::List => ctl.list(),
        Control::AddRemoteProject(args) => ctl.add_remote_project(args)?,
        Control::AddProject { path } => ctl.add_project(path)?,
        Control::Create(args) => ctl.create(args)?,
        Control::Split(args) => ctl.split(args)?,
        Control::Focus { session } => ctl.focus(session)?,
        Control::Send(args) => ctl.send(args)?,
        Control::Read { session, screen } => return ctl.read(session, screen),
        Control::OpenFile {
            project,
            path,
            text,
        } => ctl.open_file(&project, path, text)?,
        Control::Notify(args) => ctl.notify(args)?,
        Control::DismissNotice { id } => ctl.dismiss_notice(id)?,
        Control::Metadata { session, pr } => ctl.metadata(&session, pr)?,
        Control::Worktree { command } => ctl.worktree(command)?,
        Control::UiSnapshot => ctl.ui_snapshot()?,
        Control::Window { action } => ctl.window(action)?,
        Control::Browser { .. } => unreachable!(),
    };
    crate::output::Output::line(serde_json::to_string_pretty(&value)?);
    Ok(())
}
fn cli_paths(data_dir: Option<PathBuf>, runtime_dir: Option<PathBuf>) -> Result<Paths> {
    let mut paths = if let Some(path) = data_dir {
        Paths::at(std::path::absolute(path)?)
    } else {
        Paths::discover()?
    };
    if let Some(path) = runtime_dir {
        paths.runtime = std::path::absolute(path)?;
    }
    Ok(paths)
}
