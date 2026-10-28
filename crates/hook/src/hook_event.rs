//! Observational agent hooks: `event`, `emit`, `cwd`, and `prompt`.
use crate::output::Output;
use anyhow::{Context, Result, bail, ensure};
use std::{io::Read, time::Duration};
use terminator_core::*;

/// The agent process found among this hook's ancestors.
struct Ancestry {
    parent: String,
    actual: Option<String>,
    ancestors: Vec<u32>,
    process: Option<agents::ProcessIdentity>,
}
impl Ancestry {
    fn empty(ancestors: Vec<u32>) -> Self {
        Self {
            parent: String::new(),
            actual: None,
            ancestors,
            process: None,
        }
    }
}
/// A process selected while walking the ancestors, before its identity is verified.
struct Candidate {
    pid: u32,
    start_time: u64,
    kind: Option<String>,
}

/// Inputs for building the daemon request a hook command delivers.
struct HookRequest<'a> {
    command: &'a str,
    args: &'a [String],
    sid: String,
    ancestry: Ancestry,
}

pub(crate) struct HookCommand;
impl HookCommand {
    pub(crate) fn run(args: &[String]) -> Result<()> {
        let command = args.first().context("Missing hook command")?;
        let mut paths = Self::paths(args)?;
        let ancestry = if command == "event"
            || command == "emit"
            || std::env::var_os("TERMINATOR_SESSION_ID").is_none()
        {
            Self::agent_parent()
        } else {
            Ancestry::empty(vec![])
        };
        if command == "event" && ancestry.parent.is_empty() {
            return Ok(());
        }
        let (sid, token) = Self::identify(&mut paths, &ancestry.ancestors)?;
        let Some(request) = Self::request(HookRequest {
            command,
            args,
            sid,
            ancestry,
        })?
        else {
            return Ok(());
        };
        let mut stream = connect(&paths, request, Some(token))?;
        stream.set_read_timeout(Some(Duration::from_millis(500)))?;
        let response: Response = read_frame(&mut stream)?;
        if command == "prompt"
            && let Response::Text(generation) = response
        {
            Output::line(generation);
        }
        Ok(())
    }
    fn paths(args: &[String]) -> Result<Paths> {
        let mut paths = Paths::discover()?;
        if let Some(i) = args.iter().position(|a| a == "--data-dir") {
            let next = i.checked_add(1).context("Missing data path")?;
            paths.data = args.get(next).context("Missing data path")?.into();
        }
        if let Some(i) = args.iter().position(|a| a == "--runtime-dir") {
            let next = i.checked_add(1).context("Missing runtime path")?;
            paths.runtime = args.get(next).context("Missing runtime path")?.into();
        }
        Ok(paths)
    }
    fn identify(paths: &mut Paths, ancestors: &[u32]) -> Result<(String, String)> {
        if let (Ok(sid), Ok(token)) = (
            std::env::var("TERMINATOR_SESSION_ID"),
            std::env::var("TERMINATOR_SESSION_TOKEN"),
        ) {
            return Ok((sid, token));
        }
        // Some agents deliberately clear hook environments. Correlate only to an
        // actual live ancestor shell owned by this daemon, never by cwd or title.
        let Response::State(state) = rpc(paths, Request::Snapshot)? else {
            bail!("No session inventory")
        };
        let matches: Vec<_> = state
            .sessions
            .iter()
            .filter(|s| s.lifecycle.live() && s.pid.is_some_and(|pid| ancestors.contains(&pid)))
            .collect();
        ensure!(
            matches.len() == 1,
            "Hook ancestry must identify exactly one owned session"
        );
        let session = matches
            .first()
            .copied()
            .context("Hook ancestry must identify exactly one owned session")?;
        *paths = generations::owner_for(
            paths,
            &Request::History {
                session: session.id.clone(),
            },
        )?;
        Ok((session.id.clone(), paths.token()?))
    }
    fn request(input: HookRequest<'_>) -> Result<Option<Request>> {
        let HookRequest {
            command,
            args,
            sid,
            ancestry,
        } = input;
        if command == "prompt" {
            return Ok(Some(Self::prompt_request(args, sid)?));
        }
        if command == "cwd" {
            return Ok(Some(Request::Cwd {
                session: sid,
                path: args.get(1).context("Missing cwd")?.into(),
            }));
        }
        let data = Self::read_payload()?;
        if command == "emit" {
            let mut event: HookEvent = serde_json::from_slice(&data)?;
            event.terminal_session_id = sid;
            // Ancestor-supplied identity; the daemon verifies it against live
            // presence before linking lifecycle to a detected process.
            event.process = ancestry.process;
            return Ok(Some(Request::Hook(event)));
        }
        let kind = args.get(1).context("Missing adapter")?;
        let payload: serde_json::Value = serde_json::from_slice(&data)?;
        // Grok imports Claude hooks. Avoid treating those imported hooks as Claude events.
        if ancestry.actual.as_deref().is_some_and(|a| a != kind) {
            return Ok(None);
        }
        let Some(mut event) =
            terminator_integrations::normalize(kind, &sid, &ancestry.parent, &payload)?
        else {
            return Ok(None);
        };
        event.process = ancestry.process;
        Ok(Some(Request::Hook(event)))
    }
    fn prompt_request(args: &[String], sid: String) -> Result<Request> {
        if args.get(1).is_some_and(|s| s == "begin") {
            return Ok(Request::ShellCommand { session: sid });
        }
        Ok(Request::ShellPrompt {
            session: sid,
            generation: args.get(1).context("Missing prompt generation")?.parse()?,
            jobs_empty: args.get(2).is_some_and(String::is_empty),
        })
    }
    fn read_payload() -> Result<Vec<u8>> {
        // Bound stdin time as well as bytes: a misbehaving hook must not hang a CLI.
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut data = Vec::new();
            let r = std::io::stdin().take(131073).read_to_end(&mut data);
            let _ = tx.send((r, data));
        });
        let (r, data) = rx.recv_timeout(Duration::from_millis(500))?;
        r?;
        if data.len() > 131072 {
            bail!("Payload too large")
        }
        Ok(data)
    }
    /// PIDs of this process's ancestors, nearest first.
    pub(crate) fn ancestors() -> Vec<u32> {
        Self::agent_parent().ancestors
    }
    fn agent_parent() -> Ancestry {
        use sysinfo::System;
        let mut system = System::new();
        let (ancestors, selected) = Self::walk_ancestors(&mut system, Self::parent_pid());
        if let Some(candidate) = selected
            && let Some((identity, process)) = Self::verify(&mut system, &candidate)
        {
            return Ancestry {
                parent: identity,
                actual: candidate.kind,
                ancestors,
                process: Some(process),
            };
        }
        // No guessed PID-only ownership when process inspection is unavailable.
        Ancestry::empty(ancestors)
    }
    #[cfg(unix)]
    fn parent_pid() -> sysinfo::Pid {
        sysinfo::Pid::from_u32(std::os::unix::process::parent_id())
    }
    #[cfg(not(unix))]
    fn parent_pid() -> sysinfo::Pid {
        use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
        let mut system = System::new();
        let current = Pid::from_u32(std::process::id());
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[current]),
            false,
            ProcessRefreshKind::nothing(),
        );
        Pid::from_u32(
            system
                .process(current)
                .and_then(|process| process.parent())
                .map(|parent| parent.as_u32())
                .unwrap_or(0),
        )
    }
    fn walk_ancestors(
        system: &mut sysinfo::System,
        mut pid: sysinfo::Pid,
    ) -> (Vec<u32>, Option<Candidate>) {
        use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, UpdateKind};
        let mut ancestors = Vec::new();
        let mut selected = None;
        for _ in 0..16 {
            if ancestors.contains(&pid.as_u32()) {
                break;
            }
            system.refresh_processes_specifics(
                ProcessesToUpdate::Some(&[pid]),
                true,
                ProcessRefreshKind::nothing()
                    .with_exe(UpdateKind::Always)
                    .with_cmd(UpdateKind::Always),
            );
            let Some(process) = system.process(pid) else {
                break;
            };
            ancestors.push(pid.as_u32());
            if selected.is_none() {
                selected = Self::candidate(pid, process);
            }
            let Some(next) = process.parent().filter(|p| p.as_u32() > 1) else {
                break;
            };
            pid = next;
        }
        (ancestors, selected)
    }
    fn candidate(pid: sysinfo::Pid, process: &sysinfo::Process) -> Option<Candidate> {
        let executable = process
            .exe()
            .and_then(|p| p.file_name())
            .unwrap_or(process.name())
            .to_string_lossy();
        if [
            "sh",
            "bash",
            "zsh",
            "dash",
            "fish",
            "env",
            "terminator-hook",
        ]
        .contains(&executable.as_ref())
        {
            return None;
        }
        let executable_name = executable.as_ref();
        let command = process
            .cmd()
            .iter()
            .map(|arg| arg.to_string_lossy())
            .collect::<Vec<_>>();
        let argv = command
            .iter()
            .map(std::convert::AsRef::as_ref)
            .collect::<Vec<_>>();
        let kind = agents::match_agent(executable_name, &argv)
            .filter(|kind| terminator_integrations::AGENTS.contains(kind))
            .map(std::string::ToString::to_string);
        Some(Candidate {
            pid: pid.as_u32(),
            start_time: process.start_time(),
            kind,
        })
    }
    fn verify(
        system: &mut sysinfo::System,
        candidate: &Candidate,
    ) -> Option<(String, agents::ProcessIdentity)> {
        use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate};
        let Candidate {
            pid, start_time, ..
        } = *candidate;
        let mut cmd = std::process::Command::new("ps");
        cmd.args(["-p", &pid.to_string(), "-o", "lstart="]);
        let out = bounded_output(cmd, Duration::from_millis(500)).ok()?;
        let selected_pid = Pid::from_u32(pid);
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[selected_pid]),
            true,
            ProcessRefreshKind::nothing(),
        );
        if !out.status.success()
            || !system
                .process(selected_pid)
                .is_some_and(|p| p.start_time() == start_time)
        {
            return None;
        }
        let identity = Self::legacy_identity(pid, &out.stdout)?;
        Some((identity, agents::ProcessIdentity { pid, start_time }))
    }
    fn legacy_identity(pid: u32, text: &[u8]) -> Option<String> {
        let text = std::str::from_utf8(text).ok()?;
        let parts = text.split_whitespace().collect::<Vec<_>>();
        (parts.len() == 5).then(|| format!("{pid}:{}", parts.join("-")))
    }
}

#[cfg(test)]
mod identity_tests {
    use super::*;
    #[test]
    fn legacy_identity_is_preserved_without_pid_only_fallback() {
        assert_eq!(
            HookCommand::legacy_identity(42, b"Tue Sep  8 10:11:12 2026\n").as_deref(),
            Some("42:Tue-Sep-8-10:11:12-2026")
        );
        assert!(HookCommand::legacy_identity(42, b"").is_none());
        assert!(HookCommand::legacy_identity(42, b"42").is_none());
    }
}
