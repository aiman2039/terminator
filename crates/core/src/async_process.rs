//! Temporary child ownership: cancellation kills the process group and reaps it.
use crate::{
    CommandOptions,
    async_service::{CancellationToken, Failure},
};
use anyhow::{Result, ensure};
use std::{
    collections::VecDeque,
    process::{Command, Output, Stdio},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    sync::{mpsc, oneshot},
    task::JoinSet,
};
struct Request {
    command: Command,
    options: CommandOptions,
    key: Option<String>,
    cancel: CancellationToken,
    reply: oneshot::Sender<Result<Output>>,
}
#[derive(Clone)]
pub struct Processes {
    requests: mpsc::Sender<Request>,
    children: Arc<AtomicUsize>,
}
impl Processes {
    pub fn new(
        shutdown: CancellationToken,
    ) -> (Self, impl std::future::Future<Output = Result<()>> + Send) {
        let (requests, mut incoming) = mpsc::channel::<Request>(128);
        let children = Arc::new(AtomicUsize::new(0));
        let count = children.clone();
        let actor = async move {
            let mut tasks = JoinSet::new();
            let mut pending = VecDeque::<Request>::new();
            let mut keys = Vec::new();
            let mut closed = false;
            let mut cancellations = Vec::new();
            loop {
                let mut index = 0;
                while tasks.len() < 4 && index < pending.len() {
                    let request = &pending[index];
                    if request.cancel.is_cancelled() {
                        pending.remove(index);
                        continue;
                    }
                    if request.key.as_ref().is_some_and(|key| keys.contains(key)) {
                        index += 1;
                        continue;
                    }
                    let request = pending.remove(index).unwrap();
                    if let Some(key) = &request.key {
                        keys.push(key.clone());
                    }
                    let count = count.clone();
                    cancellations.push(request.cancel.clone());
                    tasks.spawn(async move {
                        count.fetch_add(1, Ordering::AcqRel);
                        let result =
                            execute(request.command, request.options, &request.cancel).await;
                        count.fetch_sub(1, Ordering::AcqRel);
                        let _ = request.reply.send(result);
                        request.key
                    });
                }
                if closed && pending.is_empty() && tasks.is_empty() {
                    break;
                }
                tokio::select! {
                    _ = shutdown.cancelled(), if !closed => {
                        incoming.close();
                        closed = true;
                        pending.clear();
                        for cancel in &cancellations { cancel.cancel(); }
                    }
                    request = incoming.recv(), if !closed && pending.len() < 128 => match request {
                        Some(request) => pending.push_back(request),
                        None => closed = true,
                    },
                    Some(result) = tasks.join_next(), if !tasks.is_empty() => {
                        if let Ok(Some(key)) = result { keys.retain(|k| k != &key); }
                        cancellations.retain(|c| !c.is_cancelled());
                    }
                }
            }
            Ok(())
        };
        (Self { requests, children }, actor)
    }
    pub fn children(&self) -> usize {
        self.children.load(Ordering::Acquire)
    }
    pub async fn run(
        &self,
        command: Command,
        options: CommandOptions,
        key: Option<String>,
    ) -> Result<Output> {
        let cancel = CancellationToken::new();
        let _cancel_on_drop = cancel.clone().drop_guard();
        let (reply, response) = oneshot::channel();
        let request = Request {
            command,
            options,
            key,
            cancel: cancel.clone(),
            reply,
        };
        tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(Failure::Cancelled.into()),
            result = self.requests.send(request) => {
                result.map_err(|_| Failure::Closed)?;
            }
        }
        response.await.map_err(|_| Failure::Closed)?
    }
}
async fn drain(mut reader: impl AsyncRead + Unpin, limit: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut buffer = [0; 8192];
    loop {
        let n = reader.read(&mut buffer).await?;
        if n == 0 {
            return Ok(bytes);
        }
        ensure!(
            bytes.len().saturating_add(n) <= limit,
            "Command output exceeds {limit} bytes"
        );
        bytes.extend_from_slice(&buffer[..n]);
    }
}
async fn execute(
    command: Command,
    options: CommandOptions,
    cancel: &CancellationToken,
) -> Result<Output> {
    if cancel.is_cancelled() {
        return Err(Failure::Cancelled.into());
    }
    let mut command = tokio::process::Command::from(command);
    command
        .process_group(0)
        .kill_on_drop(true)
        .stdin(if options.input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn()?;
    let group = child.id().unwrap() as i32;
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let input = child.stdin.take();
    let transfer = async {
        let (stdout, stderr, (), status) = tokio::try_join!(
            drain(stdout, options.stdout_limit),
            drain(stderr, options.stderr_limit),
            async {
                if let Some(mut input) = input {
                    match input
                        .write_all(options.input.as_deref().unwrap_or_default())
                        .await
                    {
                        Err(e) if e.kind() != std::io::ErrorKind::BrokenPipe => {
                            return Err(e.into());
                        }
                        _ => {}
                    }
                }
                Ok(())
            },
            async { Ok(child.wait().await?) }
        )?;
        if let Some(accepted) = &options.accepted_exit_codes {
            ensure!(
                status.code().is_some_and(|c| accepted.contains(&c)),
                "Command failed ({status}): {}",
                String::from_utf8_lossy(&stderr)
            );
        }
        Ok(Output {
            status,
            stdout,
            stderr,
        })
    };
    let result: Result<Output> = tokio::select! {
        result = transfer => result,
        _ = cancel.cancelled() => Err(Failure::Cancelled.into()),
        _ = tokio::time::sleep(options.timeout) => Err(anyhow::anyhow!("Command timed out after {:?}", options.timeout)),
    };
    if result.is_err() {
        unsafe {
            libc::kill(-group, libc::SIGKILL);
        }
        let _ = child.start_kill();
        let _ = child.wait().await;
    }
    result
}

/// Filesystem-only repository identity; call on a native filesystem worker.
/// Linked checkouts share their canonical common Git directory.
pub fn git_key(cwd: &std::path::Path) -> String {
    use std::{io::Read, os::unix::fs::OpenOptionsExt};
    let read = |path: &std::path::Path| -> Option<String> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(path)
            .ok()?;
        if !file.metadata().ok()?.is_file() {
            return None;
        }
        let mut text = String::new();
        file.take(4097).read_to_string(&mut text).ok()?;
        (text.len() <= 4096).then_some(text)
    };
    let cwd = cwd.canonicalize().unwrap_or_else(|_| cwd.into());
    for root in cwd.ancestors() {
        let dotgit = root.join(".git");
        let git = if dotgit.is_dir() {
            dotgit
        } else if let Some(text) =
            read(&dotgit).and_then(|s| s.strip_prefix("gitdir:").map(|s| s.trim().to_owned()))
        {
            root.join(text)
        } else {
            continue;
        };
        let common = read(&git.join("commondir"))
            .map(|text| git.join(text.trim()))
            .unwrap_or(git);
        return common
            .canonicalize()
            .unwrap_or(common)
            .to_string_lossy()
            .into_owned();
    }
    cwd.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    fn shell(script: &str) -> Command {
        let mut command = Command::new("sh");
        command.args(["-c", script]);
        command
    }
    #[tokio::test]
    async fn stalled_children_inherited_pipes_and_overflow_are_reaped() {
        let stop = CancellationToken::new();
        let (processes, actor) = Processes::new(stop.clone());
        let owner = tokio::spawn(actor);
        for script in ["sleep 10", "sleep 10 & exit 0", "yes x", "yes x >&2"] {
            let started = Instant::now();
            assert!(
                processes
                    .run(
                        shell(script),
                        CommandOptions {
                            timeout: Duration::from_millis(80),
                            stdout_limit: 128,
                            stderr_limit: 128,
                            ..Default::default()
                        },
                        None
                    )
                    .await
                    .is_err()
            );
            assert!(started.elapsed() < Duration::from_secs(2));
            assert_eq!(processes.children(), 0);
        }
        stop.cancel();
        owner.await.unwrap().unwrap();
    }
    #[tokio::test]
    async fn cancellation_reaps_child_and_other_repository_progresses() {
        let stop = CancellationToken::new();
        let (processes, actor) = Processes::new(stop.clone());
        let owner = tokio::spawn(actor);
        let other = processes.clone();
        let blocked = tokio::spawn(async move {
            other
                .run(
                    shell("sleep 10"),
                    CommandOptions::default(),
                    Some("one".into()),
                )
                .await
        });
        let output = processes
            .run(
                shell("printf ready"),
                CommandOptions::default(),
                Some("two".into()),
            )
            .await
            .unwrap();
        assert_eq!(output.stdout, b"ready");
        blocked.abort();
        let _ = blocked.await;
        let deadline = Instant::now() + Duration::from_secs(2);
        while processes.children() != 0 {
            assert!(Instant::now() < deadline);
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        stop.cancel();
        owner.await.unwrap().unwrap();
    }
}
