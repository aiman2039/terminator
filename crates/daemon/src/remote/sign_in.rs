//! SSH sign-in with prompts in the GUI (`ssh-askpass-v1`); see
//! [`terminator_core::ssh::sign_in`].
//!
//! One sign-in per OpenSSH master. The `ssh` child gets the path of a
//! private socket (0600, in the runtime folder) through
//! `TERMINATOR_ASKPASS_SOCKET`; `terminator-hook` in askpass mode asks
//! there. The prompt waits in memory until the GUI answers or cancels, the
//! sign-in ends, or five minutes pass. Answers are never saved or logged.
use super::checked;
use crate::server::{Shared, relock};
use anyhow::{Context, Result, bail, ensure};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc, LazyLock, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use terminator_core::ssh::{
    SshOptions,
    sign_in::{AskpassAnswer, AskpassQuestion, MAX_ASKPASS_TEXT, Secret, SignInPrompt},
};

/// The longest a sign-in runs, prompts included.
const LIMIT: Duration = Duration::from_mins(5);

/// One question line: a JSON-escaped prompt plus its frame.
const QUESTION_LIMIT: u64 = 2 * MAX_ASKPASS_TEXT as u64 + 64;

fn after(duration: Duration) -> Instant {
    Instant::now()
        .checked_add(duration)
        .unwrap_or_else(Instant::now)
}

/// A running sign-in: the prompt it waits on and where its answer goes.
#[derive(Default)]
struct SignIn {
    waiting: Mutex<Option<(SignInPrompt, mpsc::Sender<Option<Secret>>)>>,
    next_id: AtomicU64,
}

/// Running sign-ins by control socket path.
static ACTIVE: LazyLock<Mutex<HashMap<PathBuf, Arc<SignIn>>>> = LazyLock::new(Default::default);

/// Forgets the sign-in when it ends, however it ends.
struct Registered(PathBuf);
impl Drop for Registered {
    fn drop(&mut self) {
        relock(&ACTIVE).remove(&self.0);
    }
}

/// Removes the sign-in's socket and log files.
struct Files(Vec<PathBuf>);
impl Drop for Files {
    fn drop(&mut self) {
        for path in &self.0 {
            let _ = std::fs::remove_file(path);
        }
    }
}

impl Shared {
    /// Signs in to `ssh.host`, leaving its master running. Returns at once
    /// when a master already answers.
    pub(crate) fn ssh_sign_in(&self, ssh: &SshOptions) -> Result<()> {
        ssh.validate()?;
        let control = self
            .ssh_control(ssh)
            .context("SSH sign-in needs a shorter runtime folder path")?;
        if master_alive(&control) {
            return Ok(());
        }
        let sign_in = Arc::new(SignIn::default());
        {
            let mut active = relock(&ACTIVE);
            ensure!(
                !active.contains_key(&control),
                "A sign-in to this host is already running"
            );
            active.insert(control.clone(), Arc::clone(&sign_in));
        }
        let _registered = Registered(control.clone());
        run(RunInput {
            ssh,
            control: &control,
            helper: &self.helper.executable,
            sign_in: &sign_in,
        })
    }

    /// The prompt the sign-in for `ssh` waits on, if any.
    pub(crate) fn ssh_sign_in_prompt(&self, ssh: &SshOptions) -> Option<SignInPrompt> {
        let control = self.ssh_control(ssh)?;
        let sign_in = relock(&ACTIVE).get(&control).cloned()?;
        let waiting = relock(&sign_in.waiting);
        waiting.as_ref().map(|(prompt, _)| prompt.clone())
    }

    /// Answers prompt `id`; `None` cancels the sign-in.
    pub(crate) fn answer_ssh_prompt(
        &self,
        ssh: &SshOptions,
        id: u64,
        answer: Option<Secret>,
    ) -> Result<()> {
        let control = self.ssh_control(ssh).context("No sign-in for this host")?;
        let sign_in = relock(&ACTIVE)
            .get(&control)
            .cloned()
            .context("No sign-in for this host")?;
        let mut waiting = relock(&sign_in.waiting);
        match waiting.take() {
            Some((prompt, sender)) if prompt.id == id => {
                let _ = sender.send(answer);
                Ok(())
            }
            other => {
                *waiting = other;
                bail!("That sign-in prompt is no longer waiting")
            }
        }
    }
}

/// Whether a master already answers on `control` (2 s at most).
fn master_alive(control: &Path) -> bool {
    let Ok(mut child) = terminator_core::ssh::check_control(control)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    else {
        return false;
    };
    let deadline = after(Duration::from_secs(2));
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

struct RunInput<'a> {
    ssh: &'a SshOptions,
    control: &'a Path,
    helper: &'a Path,
    sign_in: &'a Arc<SignIn>,
}

#[cfg(unix)]
fn run(input: RunInput<'_>) -> Result<()> {
    use std::os::unix::{
        fs::{OpenOptionsExt, PermissionsExt},
        net::UnixListener,
    };
    let RunInput {
        ssh,
        control,
        helper,
        sign_in,
    } = input;
    let tag = uuid::Uuid::new_v4().simple().to_string();
    let tag = tag.get(..8).unwrap_or("00000000");
    let socket = control.with_file_name(format!("ap-{tag}.sock"));
    let log = control.with_file_name(format!("ap-{tag}.log"));
    let _files = Files(vec![socket.clone(), log.clone()]);
    let listener = UnixListener::bind(&socket).context("Open the sign-in socket")?;
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
    listener.set_nonblocking(true)?;
    let stderr = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&log)?;
    let mut command =
        super::command_with_args(ssh, terminator_core::ssh::sign_in::arguments(ssh, control)?);
    command
        .arg("true")
        .env("SSH_ASKPASS", helper)
        .env("SSH_ASKPASS_REQUIRE", "force")
        .env(terminator_core::ssh::sign_in::ASKPASS_SOCKET_ENV, &socket)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(stderr);
    if std::env::var_os("DISPLAY").is_none() {
        // OpenSSH before 8.4 asks through SSH_ASKPASS only with DISPLAY set.
        command.env("DISPLAY", "terminator:0");
    }
    let mut child = command.spawn().context("Start ssh")?;
    let deadline = after(LIMIT);
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!("SSH sign-in took longer than 5 minutes");
        }
        if let Ok((stream, _)) = listener.accept() {
            let sign_in = Arc::clone(sign_in);
            let _ = std::thread::Builder::new()
                .name("ssh-askpass".into())
                .spawn(move || serve(&sign_in, stream, deadline));
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    // Unblock any prompt still waiting: its `ssh` is gone.
    relock(&sign_in.waiting).take();
    let stderr = std::fs::read(&log).unwrap_or_default();
    let stderr = stderr.get(..super::STDERR_LIMIT).unwrap_or(&stderr);
    checked(status, stderr)
}

#[cfg(not(unix))]
fn run(_input: RunInput<'_>) -> Result<()> {
    bail!("SSH sign-in in the app needs macOS or Linux")
}

/// One askpass question: show it, wait for the GUI, reply.
#[cfg(unix)]
fn serve(sign_in: &SignIn, stream: std::os::unix::net::UnixStream, deadline: Instant) {
    use std::io::{BufRead, BufReader, Read, Write};
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut line = String::new();
    let read = BufReader::new((&stream).take(QUESTION_LIMIT)).read_line(&mut line);
    let question = read
        .ok()
        .and_then(|_| serde_json::from_str::<AskpassQuestion>(&line).ok());
    let answer = question.and_then(|question| wait_for_answer(sign_in, &question.prompt, deadline));
    let Ok(mut reply) = serde_json::to_vec(&AskpassAnswer { answer }) else {
        return;
    };
    reply.push(b'\n');
    let _ = (&stream).write_all(&reply);
}

/// Shows `prompt` and waits for its answer; `None` on cancel or timeout.
fn wait_for_answer(sign_in: &SignIn, prompt: &str, deadline: Instant) -> Option<Secret> {
    let text: String = prompt
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_ASKPASS_TEXT)
        .collect();
    let (sender, receiver) = mpsc::channel();
    let id = sign_in
        .next_id
        .fetch_add(1, Ordering::Relaxed)
        .saturating_add(1);
    *relock(&sign_in.waiting) = Some((SignInPrompt { id, text }, sender));
    let wait = deadline.saturating_duration_since(Instant::now());
    let answer = receiver.recv_timeout(wait).ok().flatten();
    let mut waiting = relock(&sign_in.waiting);
    if waiting.as_ref().is_some_and(|(prompt, _)| prompt.id == id) {
        waiting.take();
    }
    answer.filter(|a| a.expose().len() <= MAX_ASKPASS_TEXT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_waiting_prompt_takes_one_answer_and_then_clears() {
        let sign_in = Arc::new(SignIn::default());
        let waiter = {
            let sign_in = Arc::clone(&sign_in);
            std::thread::spawn(move || wait_for_answer(&sign_in, "Password:\x07", after(LIMIT)))
        };
        let prompt = loop {
            if let Some((prompt, _)) = relock(&sign_in.waiting).as_ref() {
                break prompt.clone();
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        assert_eq!(prompt.text, "Password:");
        let sender = relock(&sign_in.waiting).take().unwrap().1;
        sender.send(Some(Secret::new("pw".into()))).unwrap();
        assert_eq!(waiter.join().unwrap().unwrap().expose(), "pw");
        assert!(relock(&sign_in.waiting).is_none());
    }

    #[test]
    fn a_prompt_with_no_answer_cancels_at_the_deadline() {
        let sign_in = SignIn::default();
        let answer = wait_for_answer(&sign_in, "Code:", Instant::now());
        assert!(answer.is_none());
        assert!(relock(&sign_in.waiting).is_none());
    }
}
