use super::{
    CreateArgs, NotifyArgs, SendArgs, SplitArgs,
    lookup::{Ctl, NewSession},
};
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD as B64};
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    path::PathBuf,
    time::Duration,
};
use terminator_core::*;

impl Ctl {
    pub(super) fn create(&self, args: CreateArgs) -> Result<Value> {
        let CreateArgs {
            project: name,
            cwd,
            file,
            background,
        } = args;
        if !background {
            ui_control::rpc(&self.paths, ui_control::Request::Ping)?;
        }
        let s = self.create_session(NewSession {
            project: &Self::project(&self.state, &name)?,
            cwd,
            file,
        })?;
        if !background {
            ui_control::rpc(
                &self.paths,
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
        Ok(serde_json::to_value(s)?)
    }
    pub(super) fn split(&self, args: SplitArgs) -> Result<Value> {
        let SplitArgs {
            session: sid,
            direction,
        } = args;
        ui_control::rpc(&self.paths, ui_control::Request::Ping)?;
        let origin = self.session(&sid)?;
        ensure!(origin.lifecycle.live(), "Origin session has ended");
        let s = self.create_session(NewSession {
            project: &Self::project(&self.state, &origin.project_id)?,
            cwd: Some(origin.cwd.clone()),
            file: None,
        })?;
        ui_control::rpc(
            &self.paths,
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
        Ok(serde_json::to_value(s)?)
    }
    pub(super) fn focus(&self, session: String) -> Result<Value> {
        Ok(ui_control::rpc(
            &self.paths,
            ui_control::Request::Focus { session },
        )?)
    }
    pub(super) fn send(&self, args: SendArgs) -> Result<Value> {
        let SendArgs {
            session: sid,
            text,
            stdin,
            enter,
        } = args;
        ensure!(
            stdin ^ text.is_some(),
            "Supply either literal text or --stdin"
        );
        self.session(&sid)?;
        let mut bytes = Self::send_bytes(stdin, text)?;
        ensure!(bytes.len() <= 1024 * 1024, "Input exceeds 1 MiB");
        if enter {
            bytes.push(b'\r');
        }
        self.attach_and_input(sid, bytes)?;
        Ok(json!({"sent":true}))
    }
    fn send_bytes(stdin: bool, text: Option<String>) -> Result<Vec<u8>> {
        if stdin {
            let mut bytes = Vec::new();
            std::io::stdin()
                .take(1024 * 1024 + 1)
                .read_to_end(&mut bytes)?;
            return Ok(bytes);
        }
        Ok(text
            .context("Supply either literal text or --stdin")?
            .into_bytes())
    }
    fn attach_and_input(&self, sid: String, bytes: Vec<u8>) -> Result<()> {
        let rec = self.session(&sid)?;
        let owner = generations::owner_for(
            &self.paths,
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
        Ok(())
    }
    pub(super) fn read(&self, session: String, screen: bool) -> Result<()> {
        if screen {
            self.capability(SCREEN_CAPABILITY)?;
        }
        let request = if screen {
            Request::Screen { session }
        } else {
            Request::History { session }
        };
        match rpc(&self.paths, request)?.checked()? {
            Response::Text(text) => {
                crate::output::Output::text(text);
                Ok(())
            }
            _ => anyhow::bail!("No terminal text returned"),
        }
    }
    pub(super) fn open_file(&self, name: &str, path: PathBuf, text: bool) -> Result<Value> {
        Ok(ui_control::rpc(
            &self.paths,
            ui_control::Request::OpenFile {
                project: Self::project(&self.state, name)?.id.clone(),
                path: std::path::absolute(path)?,
                as_text: text,
            },
        )?)
    }
    pub(super) fn notify(&self, args: NotifyArgs) -> Result<Value> {
        let NotifyArgs {
            session,
            body,
            title,
        } = args;
        self.capability(TERMINAL_NOTICES_CAPABILITY)?;
        rpc(
            &self.paths,
            Request::TerminalNotify {
                session,
                title,
                body,
            },
        )?
        .checked()?;
        Ok(json!({"notified":true}))
    }
    pub(super) fn dismiss_notice(&self, id: String) -> Result<Value> {
        self.capability(TERMINAL_NOTICES_CAPABILITY)?;
        rpc(&self.paths, Request::DismissTerminalNotice { id })?.checked()?;
        Ok(json!({"dismissed":true}))
    }
    pub(super) fn metadata(&self, sid: &str, pr: bool) -> Result<Value> {
        let s = self.session(sid)?;
        Ok(serde_json::to_value(metadata::Cache::default().collect(
            &s.cwd,
            s.pid.map(|pid| (pid, s.created)),
            pr,
        ))?)
    }
    pub(super) fn ui_snapshot(&self) -> Result<Value> {
        Ok(ui_control::rpc(&self.paths, ui_control::Request::Snapshot)?)
    }
    pub(super) fn window(&self, action: String) -> Result<Value> {
        Ok(ui_control::rpc(
            &self.paths,
            ui_control::Request::Window { action },
        )?)
    }
}
