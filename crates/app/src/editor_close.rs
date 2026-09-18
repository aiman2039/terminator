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

pub async fn close_async(
    client: &async_client::Client,
    ids: &[String],
    mode: Mode,
    timeout: Duration,
) -> Result<()> {
    let Response::State(state) = client.rpc(Request::Snapshot).await? else {
        anyhow::bail!("Could not inspect editor state")
    };
    let live = live_editors(&state, ids)?;
    for id in &live {
        if mode == Mode::Save {
            client
                .rpc(Request::EditorSave {
                    session: id.clone(),
                })
                .await?;
        }
        if mode != Mode::Discard {
            let Response::Text(status) = client
                .rpc(Request::EditorStatus {
                    session: id.clone(),
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
        }
    }
    for id in &live {
        let socket = client.editor_socket(id.clone()).await?;
        let result = async {
            let mut rpc = crate::nvim_rpc::AsyncConnection::connect(
                &socket,
                QUIT_TIMEOUT,
                client.cpu.clone(),
            )
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
                    session: id.clone(),
                })
                .await?;
        }
    }
    for attempt in 0..2 {
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
                return Ok(());
            }
            if started.elapsed() >= timeout {
                break;
            }
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
        if mode != Mode::Discard || attempt != 0 {
            break;
        }
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
    }
    anyhow::bail!("Editor did not close. Check for unsaved buffers or running editor jobs.")
}

#[cfg(test)]
pub fn close(paths: &Paths, ids: &[String], mode: Mode, timeout: Duration) -> Result<()> {
    let client = async_client::Client::new(
        paths.clone(),
        async_service::NativePool::new("close-catalog-test", 1)?,
        async_service::NativePool::new("close-cpu-test", 2)?,
    );
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(close_async(&client, ids, mode, timeout))
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
