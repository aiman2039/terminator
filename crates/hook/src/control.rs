//! User-invoked high-level controls. Never called by observational agent hooks.
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD as B64};
use clap::{Parser, Subcommand};
use serde_json::json;
use std::{
    io::{Read, Write},
    path::PathBuf,
    time::Duration,
};
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
    Shutdown {
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
    },
    AddRemoteProject {
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
    },
    AddProject {
        path: PathBuf,
    },
    /// Create a shell/editor in a new tab. --background works without a GUI.
    Create {
        project: String,
        #[arg(long)]
        cwd: Option<PathBuf>,
        #[arg(long)]
        file: Option<PathBuf>,
        #[arg(long)]
        background: bool,
    },
    Split {
        session: String,
        #[arg(value_parser=["left","right","up","down"])]
        direction: String,
    },
    Focus {
        session: String,
    },
    /// Send literal text to a session. --stdin reads at most 1 MiB.
    Send {
        session: String,
        text: Option<String>,
        #[arg(long)]
        stdin: bool,
        #[arg(long)]
        enter: bool,
    },
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
    Notify {
        session: String,
        body: String,
        #[arg(long, default_value = "Terminal")]
        title: String,
    },
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
fn snapshot(paths: &Paths) -> Result<State> {
    match rpc(paths, Request::Snapshot)?.checked()? {
        Response::State(state) => Ok(*state),
        _ => anyhow::bail!("Expected daemon snapshot"),
    }
}
fn capability(state: &State, feature: &str) -> Result<()> {
    ensure!(
        state.capabilities.iter().any(|c| c == feature),
        "Running daemon lacks {feature}; existing live sessions were left running"
    );
    Ok(())
}
fn project(state: &State, name: &str) -> Result<Project> {
    let mut matches = state
        .projects
        .iter()
        .filter(|p| p.id == name || p.path.to_string_lossy() == name || p.name == name)
        .cloned()
        .collect::<Vec<_>>();
    matches.extend(
        state
            .remote_projects
            .iter()
            .filter(|p| p.id == name || p.name == name || p.label() == name)
            .map(|p| Project {
                id: p.id.clone(),
                name: p.name.clone(),
                path: p.identity_path(),
                layout: p.layout.clone(),
            }),
    );
    match matches.as_slice() {
        [project] => Ok(project.clone()),
        _ => anyhow::bail!("Project must match exactly one ID, location, or name"),
    }
}

fn session<'a>(state: &'a State, sid: &str) -> Result<&'a Session> {
    state
        .sessions
        .iter()
        .find(|s| s.id == sid)
        .context("Unknown session ID")
}
fn create(
    paths: &Paths,
    project: &Project,
    cwd: Option<PathBuf>,
    file: Option<PathBuf>,
) -> Result<Session> {
    let file = if project.path.starts_with("/__terminator_remote__") {
        file
    } else {
        file.map(std::path::absolute).transpose()?
    };
    match rpc(
        paths,
        Request::Create {
            project: project.id.clone(),
            cwd,
            file: file.clone(),
            line: None,
            column: None,
            editor: file.is_some(),
        },
    )?
    .checked()?
    {
        Response::Created(s) => Ok(s),
        _ => anyhow::bail!("Daemon did not create session"),
    }
}
pub fn run(args: &[String]) -> Result<()> {
    let cli = Cli::parse_from(
        std::iter::once("terminator-hook ctl".to_owned()).chain(args.iter().cloned()),
    );
    let mut paths = if let Some(path) = cli.data_dir {
        Paths::at(std::path::absolute(path)?)
    } else {
        Paths::discover()?
    };
    if let Some(path) = cli.runtime_dir {
        paths.runtime = std::path::absolute(path)?;
    }
    if let Control::Browser { command } = cli.command {
        return super::browser::run(command);
    }
    paths = generations::workspace_paths(&paths)?;
    let state = snapshot(&paths)?;
    let value = match cli.command {
        Control::Shutdown {
            stop_all,
            timeout,
            relaunch,
            exe,
            confirmed_inventory,
        } => {
            let relaunch = if relaunch {
                Some(match exe {
                    Some(path) => path,
                    None => std::env::current_exe()?.with_file_name("terminator"),
                })
            } else {
                None
            };
            super::shutdown::run(
                &paths,
                state,
                super::shutdown::Options {
                    stop_all,
                    confirmed: confirmed_inventory
                        .map(|json| serde_json::from_str(&json))
                        .transpose()?,
                    timeout: Duration::from_secs(timeout),
                    relaunch,
                },
            )?
        }
        Control::List => {
            json!({"generation":state.generation,"projects":state.projects,"remote_projects":state.remote_projects,"sessions":state.sessions,"worktrees":state.worktrees,"agents":state.agents,"terminal_notices":state.terminal_notices})
        }
        Control::AddRemoteProject {
            host,
            port,
            user,
            root,
            name,
            identity,
            known_hosts,
        } => {
            capability(&state, remote::CAPABILITY)?;
            let project = remote::RemoteProject {
                id: String::new(),
                name,
                root,
                ssh: ssh::SshOptions {
                    host,
                    port,
                    username: user,
                    identity_file: identity,
                    known_hosts_file: known_hosts,
                },
                layout: serde_json::Value::Null,
                group: None,
                forwards: Vec::new(),
            };
            project.validate()?;
            rpc(
                &paths,
                Request::AddRemoteProject {
                    project: project.clone(),
                },
            )?
            .checked()?;
            let state = snapshot(&paths)?;
            serde_json::to_value(
                state
                    .remote_projects
                    .iter()
                    .find(|p| p.ssh == project.ssh && p.root == project.root)
                    .context("Registered SSH project missing")?,
            )?
        }
        Control::AddProject { path } => {
            let path = std::path::absolute(path)?;
            rpc(&paths, Request::AddProject { path: path.clone() })?.checked()?;
            let state = snapshot(&paths)?;
            serde_json::to_value(project(&state, &path.canonicalize()?.to_string_lossy())?)?
        }
        Control::Create {
            project: name,
            cwd,
            file,
            background,
        } => {
            if !background {
                ui_control::rpc(&paths, ui_control::Request::Ping)?;
            }
            let s = create(&paths, &project(&state, &name)?, cwd, file)?;
            if !background {
                ui_control::rpc(
                    &paths,
                    ui_control::Request::ShowSession {
                        session: s.id.clone(),
                        anchor: None,
                        split: None,
                    },
                )
                .with_context(|| {
                    format!("Session {} is running, but its GUI attachment failed", s.id)
                })?;
            }
            serde_json::to_value(s)?
        }
        Control::Split {
            session: sid,
            direction,
        } => {
            ui_control::rpc(&paths, ui_control::Request::Ping)?;
            let origin = session(&state, &sid)?;
            ensure!(origin.lifecycle.live(), "Origin session has ended");
            let s = create(
                &paths,
                &project(&state, &origin.project_id)?,
                Some(origin.cwd.clone()),
                None,
            )?;
            ui_control::rpc(
                &paths,
                ui_control::Request::ShowSession {
                    session: s.id.clone(),
                    anchor: Some(sid),
                    split: Some(direction),
                },
            )
            .with_context(|| {
                format!(
                    "Session {} is running, but its split could not be shown",
                    s.id
                )
            })?;
            serde_json::to_value(s)?
        }
        Control::Focus { session } => {
            ui_control::rpc(&paths, ui_control::Request::Focus { session })?
        }
        Control::Send {
            session: sid,
            text,
            stdin,
            enter,
        } => {
            ensure!(
                stdin ^ text.is_some(),
                "Supply either literal text or --stdin"
            );
            session(&state, &sid)?;
            let mut bytes = if stdin {
                let mut bytes = Vec::new();
                std::io::stdin()
                    .take(1024 * 1024 + 1)
                    .read_to_end(&mut bytes)?;
                bytes
            } else {
                text.context("Supply either literal text or --stdin")?
                    .into_bytes()
            };
            ensure!(bytes.len() <= 1024 * 1024, "Input exceeds 1 MiB");
            if enter {
                bytes.push(b'\r');
            }
            let rec = session(&state, &sid)?;
            let owner = generations::owner_for(
                &paths,
                &Request::History {
                    session: sid.clone(),
                },
            )?;
            let mut stream = transport::Stream::connect_ipc(&owner.socket())?;
            stream.set_timeouts(Duration::from_secs(5), Duration::from_secs(5))?;
            write_frame(
                &mut stream,
                &Envelope {
                    version: PROTOCOL_VERSION,
                    auth: owner.token()?,
                    request: Request::Attach {
                        session: sid,
                        rows: rec.rows.max(1),
                        cols: rec.cols.max(1),
                    },
                    snapshot_hint: None,
                    snapshot_chunks: false,
                    ssh_errors: false,
                },
            )?;
            read_frame::<Response>(&mut stream)?.checked()?;
            write_frame(
                &mut stream,
                &Request::Input {
                    data: B64.encode(bytes),
                },
            )?;
            stream.flush()?;
            // Read the frames the daemon streams back before closing. A close
            // that leaves unread output in the socket resets the connection,
            // and a reset discards the input frames the daemon has not read
            // yet, so the text would silently never reach the terminal. Five
            // 100 ms reads bound the wait to half a second.
            stream.set_read_timeout(Some(Duration::from_millis(100)))?;
            for _ in 0..5 {
                match read_frame::<Response>(&mut stream) {
                    Ok(Response::End) | Err(_) => break,
                    Ok(_) => {}
                }
            }
            json!({"sent":true})
        }
        Control::Read { session, screen } => {
            if screen {
                capability(&state, SCREEN_CAPABILITY)?;
            }
            let request = if screen {
                Request::Screen { session }
            } else {
                Request::History { session }
            };
            match rpc(&paths, request)?.checked()? {
                Response::Text(text) => {
                    crate::output::Output::text(text);
                    return Ok(());
                }
                _ => anyhow::bail!("No terminal text returned"),
            }
        }
        Control::OpenFile {
            project: name,
            path,
            text,
        } => ui_control::rpc(
            &paths,
            ui_control::Request::OpenFile {
                project: project(&state, &name)?.id.clone(),
                path: std::path::absolute(path)?,
                as_text: text,
            },
        )?,
        Control::Notify {
            session,
            title,
            body,
        } => {
            capability(&state, TERMINAL_NOTICES_CAPABILITY)?;
            rpc(
                &paths,
                Request::TerminalNotify {
                    session,
                    title,
                    body,
                },
            )?
            .checked()?;
            json!({"notified":true})
        }
        Control::DismissNotice { id } => {
            capability(&state, TERMINAL_NOTICES_CAPABILITY)?;
            rpc(&paths, Request::DismissTerminalNotice { id })?.checked()?;
            json!({"dismissed":true})
        }
        Control::Metadata { session: sid, pr } => {
            let s = session(&state, &sid)?;
            serde_json::to_value(metadata::Cache::default().collect(
                &s.cwd,
                s.pid.map(|pid| (pid, s.created)),
                pr,
            ))?
        }
        Control::Worktree { command } => {
            capability(&state, WORKTREES_CAPABILITY)?;
            let request = match command {
                Worktree::List { project: name } => Request::WorktreeList {
                    project: project(&state, &name)?.id.clone(),
                },
                Worktree::Add {
                    project: name,
                    path,
                    branch,
                    start,
                } => Request::WorktreeAdd {
                    project: project(&state, &name)?.id.clone(),
                    path: std::path::absolute(path)?,
                    branch,
                    start,
                },
                Worktree::Remove { project: name } => Request::WorktreeRemove {
                    project: project(&state, &name)?.id.clone(),
                },
            };
            let response = rpc(&paths, request)?.checked()?;
            serde_json::to_value(response)?
        }
        Control::UiSnapshot => ui_control::rpc(&paths, ui_control::Request::Snapshot)?,
        Control::Window { action } => {
            ui_control::rpc(&paths, ui_control::Request::Window { action })?
        }
        Control::Browser { .. } => unreachable!(),
    };
    crate::output::Output::line(serde_json::to_string_pretty(&value)?);
    Ok(())
}
