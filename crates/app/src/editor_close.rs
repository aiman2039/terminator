//! Close editor processes without treating file views as shell sessions.
use anyhow::{Context, Result, ensure};
use std::{
    process::Command,
    time::{Duration, Instant},
};
use terminator_core::*;

const QUIT_TIMEOUT: Duration = Duration::from_secs(2);
const CLOSE_WAIT: Duration = Duration::from_secs(2);

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

pub fn close(paths: &Paths, ids: &[String], mode: Mode) -> Result<()> {
    let state = snapshot(paths)?;
    let live = live_editors(&state, ids)?;
    preflight(paths, &live, mode)?;
    request_quit(paths, &state, &live, mode)?;
    finish_close(paths, ids, mode)
}

fn snapshot(paths: &Paths) -> Result<Box<State>> {
    let Response::State(state) = rpc(paths, Request::Snapshot)? else {
        anyhow::bail!("Could not inspect editor state")
    };
    Ok(state)
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

fn preflight(paths: &Paths, live: &[String], mode: Mode) -> Result<()> {
    for id in live {
        if mode == Mode::Save {
            rpc(
                paths,
                Request::EditorSave {
                    session: id.clone(),
                },
            )?;
        }
        if mode == Mode::Discard {
            continue;
        }
        let Response::Text(status) = rpc(
            paths,
            Request::EditorStatus {
                session: id.clone(),
            },
        )?
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
    Ok(())
}

fn request_quit(paths: &Paths, state: &State, live: &[String], mode: Mode) -> Result<()> {
    for id in live {
        quit_one(paths, state, id, mode)?;
    }
    Ok(())
}

fn quit_one(paths: &Paths, state: &State, id: &str, mode: Mode) -> Result<()> {
    if mode == Mode::Discard && !paths.editor_socket(id).exists() {
        return stop(paths, id);
    }
    match send_quit(paths, state, id, mode) {
        Ok(()) => Ok(()),
        Err(_) if mode == Mode::Discard && already_gone(paths, id)? => Ok(()),
        Err(_) if mode == Mode::Discard => stop(paths, id),
        Err(error) => Err(error),
    }
}

fn send_quit(paths: &Paths, state: &State, id: &str, mode: Mode) -> Result<()> {
    let editor = find_executable(if state.sessions.iter().any(|s| s.id == id && s.review) {
        "nvim"
    } else {
        &state.settings.editor_program
    })
    .context("Editor executable unavailable")?;
    let mut command = Command::new(editor);
    command
        .arg("--server")
        .arg(paths.editor_socket(id))
        .arg("--remote-send")
        .arg(quit_keys(mode));
    let output = bounded_output(command, QUIT_TIMEOUT)?;
    ensure!(
        output.status.success(),
        "Could not close editor: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

fn quit_keys(mode: Mode) -> &'static str {
    if mode == Mode::Discard {
        "<C-\\><C-N>:qa!<CR>"
    } else {
        "<C-\\><C-N>:qa<CR>"
    }
}

fn already_gone(paths: &Paths, id: &str) -> Result<bool> {
    Ok(!session_live(snapshot(paths)?.as_ref(), id))
}

fn stop(paths: &Paths, id: &str) -> Result<()> {
    match rpc(
        paths,
        Request::Stop {
            session: id.to_owned(),
        },
    ) {
        Ok(_) => Ok(()),
        Err(_) if already_gone(paths, id)? => Ok(()),
        Err(error) => Err(error),
    }
}

fn stop_live(paths: &Paths, ids: &[String]) -> Result<()> {
    let state = snapshot(paths)?;
    for id in ids {
        if session_live(&state, id) {
            stop(paths, id)?;
        }
    }
    Ok(())
}

fn occupies_close(mode: Mode, lifecycle: &Lifecycle) -> bool {
    match mode {
        Mode::Discard => *lifecycle == Lifecycle::Running,
        Mode::Check | Mode::Save => lifecycle.live(),
    }
}

fn wait_until_closed(paths: &Paths, ids: &[String], timeout: Duration, mode: Mode) -> Result<bool> {
    let started = Instant::now();
    loop {
        if !snapshot(paths)?
            .sessions
            .iter()
            .any(|session| ids.contains(&session.id) && occupies_close(mode, &session.lifecycle))
        {
            return Ok(true);
        }
        if started.elapsed() >= timeout {
            return Ok(false);
        }
        std::thread::sleep(Duration::from_millis(40));
    }
}

fn finish_close(paths: &Paths, ids: &[String], mode: Mode) -> Result<()> {
    if wait_until_closed(paths, ids, CLOSE_WAIT, mode)? {
        return Ok(());
    }
    if mode == Mode::Discard {
        stop_live(paths, ids)?;
        if wait_until_closed(paths, ids, CLOSE_WAIT, mode)? {
            return Ok(());
        }
    }
    anyhow::bail!("Editor did not close. Check for unsaved buffers or running editor jobs.")
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
