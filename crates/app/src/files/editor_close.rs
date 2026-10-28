//! Close editor processes without treating file views as shell sessions.
use anyhow::{Context, Result, ensure};
use std::time::{Duration, Instant};
use terminator_core::*;

const QUIT_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Workspace(String, String),
    Pane(String),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Check,
    Save,
    Discard,
}

fn session_live(state: &State, id: &str) -> bool {
    state
        .sessions
        .iter()
        .any(|session| session.id == id && session.lifecycle.live())
}

fn live_editors(state: &State, ids: &[String]) -> Result<Vec<String>> {
    let live: Vec<String> = ids
        .iter()
        .filter(|id| session_live(state, id))
        .cloned()
        .collect();
    ensure!(
        live.iter().all(|id| state
            .sessions
            .iter()
            .any(|session| session.id == *id && session.kind == SessionKind::Editor)),
        "This view contains a running terminal"
    );
    ensure!(
        !state
            .agents
            .iter()
            .any(|agent| ids.contains(&agent.session_id)
                && !matches!(
                    agent.state,
                    AgentState::Completed | AgentState::Failed | AgentState::Stopped
                )),
        "An agent is running in this editor. Use the terminal session close controls."
    );
    Ok(live)
}

fn quit_keys(mode: Mode) -> &'static str {
    if mode == Mode::Discard {
        "<C-\\><C-N>:qa!<CR>"
    } else {
        "<C-\\><C-N>:qa<CR>"
    }
}

fn occupies_close(mode: Mode, lifecycle: &Lifecycle) -> bool {
    match mode {
        Mode::Discard => *lifecycle == Lifecycle::Running,
        Mode::Check | Mode::Save => lifecycle.live(),
    }
}

pub struct CloseAsyncInput<'a> {
    pub client: &'a async_client::Client,
    pub ids: &'a [String],
    pub mode: Mode,
    pub timeout: Duration,
}

pub async fn close_async(input: CloseAsyncInput<'_>) -> Result<()> {
    let CloseAsyncInput {
        client,
        ids,
        mode,
        timeout,
    } = input;
    let Response::State(state) = client.rpc(Request::Snapshot).await? else {
        anyhow::bail!("Could not inspect editor state")
    };
    let live = live_editors(&state, ids)?;
    for id in &live {
        save_or_check(client, id, mode).await?;
    }
    for id in &live {
        quit_editor(client, id, mode).await?;
    }
    for attempt in 0..2 {
        if wait_closed(WaitClosedInput {
            client,
            ids,
            mode,
            timeout,
        })
        .await?
        {
            return Ok(());
        }
        if mode != Mode::Discard || attempt != 0 {
            break;
        }
        stop_live(client, ids).await?;
    }
    anyhow::bail!("Editor did not close. Check for unsaved buffers or running editor jobs.")
}

/// Saves first in Save mode; except when discarding, fails on unsaved changes.
async fn save_or_check(client: &async_client::Client, id: &str, mode: Mode) -> Result<()> {
    if mode == Mode::Save {
        client
            .rpc(Request::EditorSave {
                session: id.to_owned(),
            })
            .await?;
    }
    if mode == Mode::Discard {
        return Ok(());
    }
    let Response::Text(status) = client
        .rpc(Request::EditorStatus {
            session: id.to_owned(),
        })
        .await?
    else {
        anyhow::bail!("Could not check unsaved changes")
    };
    ensure!(
        status
            .trim()
            .parse::<usize>()
            .context("Could not check unsaved changes")?
            == 0,
        "Unsaved changes"
    );
    Ok(())
}

/// Sends the quit keys. A discarding close stops an unreachable editor.
async fn quit_editor(client: &async_client::Client, id: &str, mode: Mode) -> Result<()> {
    let socket = client.editor_socket(id.to_owned()).await?;
    let result = async {
        let mut rpc =
            crate::nvim_rpc::AsyncConnection::connect(&socket, QUIT_TIMEOUT, client.cpu.clone())
                .await?;
        let keys = rpc
            .call(
                "nvim_replace_termcodes",
                serde_json::json!([quit_keys(mode), true, false, true]),
                4096,
            )
            .await?;
        // nvim_input acknowledges queued input; process exit is confirmed below.
        rpc.call("nvim_input", serde_json::json!([keys]), 4096)
            .await?;
        Ok::<_, anyhow::Error>(())
    }
    .await;
    if let Err(error) = result {
        if mode != Mode::Discard {
            return Err(error);
        }
        client
            .rpc(Request::Stop {
                session: id.to_owned(),
            })
            .await?;
    }
    Ok(())
}

struct WaitClosedInput<'a> {
    client: &'a async_client::Client,
    ids: &'a [String],
    mode: Mode,
    timeout: Duration,
}

/// Polls until no listed session still occupies the close, up to `timeout`.
async fn wait_closed(input: WaitClosedInput<'_>) -> Result<bool> {
    let WaitClosedInput {
        client,
        ids,
        mode,
        timeout,
    } = input;
    let started = Instant::now();
    loop {
        let Response::State(state) = client.rpc(Request::Snapshot).await? else {
            anyhow::bail!("Could not inspect editor state")
        };
        if !state
            .sessions
            .iter()
            .any(|s| ids.contains(&s.id) && occupies_close(mode, &s.lifecycle))
        {
            return Ok(true);
        }
        if started.elapsed() >= timeout {
            return Ok(false);
        }
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
}

async fn stop_live(client: &async_client::Client, ids: &[String]) -> Result<()> {
    let Response::State(state) = client.rpc(Request::Snapshot).await? else {
        anyhow::bail!("Could not inspect editor state")
    };
    for id in ids {
        if session_live(&state, id) {
            client
                .rpc(Request::Stop {
                    session: id.clone(),
                })
                .await?;
        }
    }
    Ok(())
}

#[cfg(test)]
#[cfg(unix)]
pub struct CloseInput<'a> {
    pub paths: &'a Paths,
    pub ids: &'a [String],
    pub mode: Mode,
    pub timeout: Duration,
}

#[cfg(test)]
#[cfg(unix)]
pub fn close(input: CloseInput<'_>) -> Result<()> {
    let CloseInput {
        paths,
        ids,
        mode,
        timeout,
    } = input;
    let client = async_client::Client::new(
        paths.clone(),
        async_service::NativePool::new("close-catalog-test", 1)?,
        async_service::NativePool::new("close-cpu-test", 2)?,
    );
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(close_async(CloseAsyncInput {
            client: &client,
            ids,
            mode,
            timeout,
        }))
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discard_sends_force_quit_keys() {
        assert_eq!(quit_keys(Mode::Discard), "<C-\\><C-N>:qa!<CR>");
        assert_eq!(quit_keys(Mode::Save), "<C-\\><C-N>:qa<CR>");
        assert_eq!(quit_keys(Mode::Check), "<C-\\><C-N>:qa<CR>");
    }

    #[test]
    fn discard_treats_stopping_as_closed() {
        assert!(!occupies_close(Mode::Discard, &Lifecycle::Stopping));
        assert!(!occupies_close(Mode::Discard, &Lifecycle::Ended));
        assert!(occupies_close(Mode::Discard, &Lifecycle::Running));
        assert!(occupies_close(Mode::Save, &Lifecycle::Stopping));
        assert!(occupies_close(Mode::Check, &Lifecycle::Stopping));
        assert!(!occupies_close(Mode::Save, &Lifecycle::Ended));
    }

    #[test]
    fn vanished_editor_is_already_gone() {
        let mut state = State::default();
        state.sessions.push(Session {
            remote: None,
            review: false,
            id: "alive".into(),
            project_id: "p".into(),
            label: "file.rs".into(),
            cwd: "/tmp".into(),
            kind: SessionKind::Editor,
            file: None,
            created: 0,
            lifecycle: Lifecycle::Running,
            pid: None,
            truncated: false,
            exit_code: None,
            rows: 24,
            cols: 80,
            generation: "fixture".into(),
            cwd_confirmed: true,
        });
        assert!(!session_live(&state, "missing"));
        assert!(session_live(&state, "alive"));
        state.sessions[0].lifecycle = Lifecycle::Stopping;
        assert!(session_live(&state, "alive"));
        state.sessions[0].lifecycle = Lifecycle::Ended;
        assert!(!session_live(&state, "alive"));
    }
}
