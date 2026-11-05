//! Temporary child ownership: cancellation kills the process group and reaps it.
//! A long user command (`run_cancellable`) gets Ctrl-C first and keeps the
//! end of a large output.
use crate::{
    CommandOptions,
    async_service::{CancellationToken, Failure},
    signals::{ProcSignal, signal_group},
};
use std::{
    collections::VecDeque,
    process::{Command, Output, Stdio},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    process::{ChildStderr, ChildStdin, ChildStdout},
    sync::{mpsc, oneshot},
    task::JoinSet,
};
/// Why a pooled child process did not complete successfully.
#[derive(Debug, thiserror::Error)]
pub enum AsyncProcessError {
    #[error("child pid missing")]
    MissingPid,
    #[error("{pipe} pipe missing")]
    MissingPipe { pipe: &'static str },
    #[error("Command output exceeds {limit} bytes")]
    OutputTooLarge { limit: usize },
    #[error("Command failed ({status}): {stderr}")]
    Failed {
        status: std::process::ExitStatus,
        stderr: String,
    },
    #[error("Command timed out after {timeout:?}")]
    TimedOut { timeout: std::time::Duration },
    #[error(transparent)]
    Stopped(#[from] Failure),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
type Result<T, E = AsyncProcessError> = std::result::Result<T, E>;

/// How long a long user command may clean up after Ctrl-C before its
/// process group is killed.
const INTERRUPT_GRACE: Duration = Duration::from_secs(5);

struct Request {
    command: Command,
    options: CommandOptions,
    key: Option<String>,
    cancel: CancellationToken,
    /// A long user command ([`Processes::run_cancellable`]).
    long: bool,
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
        let count = Arc::clone(&children);
        let actor = async move {
            let mut tasks = JoinSet::new();
            let mut pending = VecDeque::<Request>::new();
            let mut keys = Vec::new();
            let mut closed = false;
            let mut cancellations = Vec::new();
            loop {
                let mut index = 0;
                while tasks.len() < 4 && index < pending.len() {
                    let Some(request) = pending.get(index) else {
                        break;
                    };
                    let cancelled = request.cancel.is_cancelled();
                    let blocked = request.key.as_ref().is_some_and(|key| keys.contains(key));
                    if cancelled {
                        pending.remove(index);
                        continue;
                    }
                    if blocked {
                        index = index.saturating_add(1);
                        continue;
                    }
                    let Some(request) = pending.remove(index) else {
                        continue;
                    };
                    if let Some(key) = &request.key {
                        keys.push(key.clone());
                    }
                    let count = Arc::clone(&count);
                    cancellations.push(request.cancel.clone());
                    tasks.spawn(async move {
                        count.fetch_add(1, Ordering::AcqRel);
                        let stop = (&request.cancel, request.long);
                        let result = execute(request.command, request.options, stop).await;
                        count.fetch_sub(1, Ordering::AcqRel);
                        let _ = request.reply.send(result);
                        request.key
                    });
                }
                if closed && pending.is_empty() && tasks.is_empty() {
                    break;
                }
                tokio::select! {
                    () = shutdown.cancelled(), if !closed => {
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
    #[must_use]
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
            cancel,
            long: false,
            reply,
        };
        self.submit(request, response).await
    }
    /// Like [`Self::run`] without a key, for long user-driven commands.
    /// Cancelling `cancel` (or the timeout) sends Ctrl-C to the process
    /// group, so Git and hooks clean up, and kills what still runs after
    /// [`INTERRUPT_GRACE`]; the call returns only after the child is
    /// reaped. Output past a limit keeps its end instead of failing.
    pub async fn run_cancellable(
        &self,
        command: Command,
        options: CommandOptions,
        cancel: &CancellationToken,
    ) -> Result<Output> {
        let cancel = cancel.child_token();
        let _cancel_on_drop = cancel.clone().drop_guard();
        let (reply, response) = oneshot::channel();
        let request = Request {
            command,
            options,
            key: None,
            cancel,
            long: true,
            reply,
        };
        self.submit(request, response).await
    }
    async fn submit(
        &self,
        request: Request,
        response: oneshot::Receiver<Result<Output>>,
    ) -> Result<Output> {
        let cancel = request.cancel.clone();
        tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(Failure::Cancelled.into()),
            result = self.requests.send(request) => {
                result.map_err(|_| Failure::Closed)?;
            }
        }
        // A request cancelled while queued is dropped without a reply.
        response.await.map_err(|_| {
            if cancel.is_cancelled() {
                Failure::Cancelled
            } else {
                Failure::Closed
            }
        })?
    }
}
/// Read a pipe to its end. Past `limit` bytes a quick call fails; a long
/// one (`tail`) keeps the last `limit` bytes, where a failure shows.
async fn drain(mut reader: impl AsyncRead + Unpin, limit: usize, tail: bool) -> Result<Vec<u8>> {
    let mut bytes = VecDeque::new();
    let mut buffer = [0; 8192];
    loop {
        let n = reader.read(&mut buffer).await?;
        if n == 0 {
            return Ok(bytes.into());
        }
        if bytes.len().saturating_add(n) > limit && !tail {
            return Err(AsyncProcessError::OutputTooLarge { limit });
        }
        if let Some(chunk) = buffer.get(..n) {
            bytes.extend(chunk);
        }
        let over = bytes.len().saturating_sub(limit);
        bytes.drain(..over);
    }
}
/// Run one child. `(cancel, long)`: cancelling kills the process group;
/// a long user command gets Ctrl-C first (see [`Processes::run_cancellable`]).
async fn execute(
    command: Command,
    options: CommandOptions,
    (cancel, long): (&CancellationToken, bool),
) -> Result<Output> {
    if cancel.is_cancelled() {
        return Err(Failure::Cancelled.into());
    }
    let mut command = tokio::process::Command::from(command);
    #[cfg(unix)]
    command.process_group(0);
    command
        .kill_on_drop(true)
        .stdin(if options.input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn()?;
    let group = child.id().ok_or(AsyncProcessError::MissingPid)?;
    let stdout = child
        .stdout
        .take()
        .ok_or(AsyncProcessError::MissingPipe { pipe: "stdout" })?;
    let stderr = child
        .stderr
        .take()
        .ok_or(AsyncProcessError::MissingPipe { pipe: "stderr" })?;
    let input = child.stdin.take();
    let result: Result<Output> = {
        let transfer = collect(&mut child, (stdout, stderr, input), &options, long);
        tokio::pin!(transfer);
        let stopped = tokio::select! {
            result = &mut transfer => Ok(result),
            () = cancel.cancelled() => Err(AsyncProcessError::from(Failure::Cancelled)),
            () = tokio::time::sleep(options.timeout) => Err(AsyncProcessError::TimedOut { timeout: options.timeout }),
        };
        match stopped {
            Ok(result) => result,
            Err(stop) => {
                if long {
                    // Ctrl-C first, as in a terminal: Git removes its lock
                    // files and a hook manager puts stashed edits back.
                    let _ = signal_group(group, ProcSignal::Interrupt);
                    let _ = tokio::time::timeout(INTERRUPT_GRACE, &mut transfer).await;
                }
                Err(stop)
            }
        }
    };
    if result.is_err() {
        let _ = signal_group(group, ProcSignal::Kill);
        let _ = child.start_kill();
        let _ = child.wait().await;
    }
    result
}

/// The child's output and exit status, with its input written first.
async fn collect(
    child: &mut tokio::process::Child,
    (stdout, stderr, input): (ChildStdout, ChildStderr, Option<ChildStdin>),
    options: &CommandOptions,
    tail: bool,
) -> Result<Output> {
    let (stdout, stderr, (), status) = tokio::try_join!(
        drain(stdout, options.stdout_limit, tail),
        drain(stderr, options.stderr_limit, tail),
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
    if let Some(accepted) = &options.accepted_exit_codes
        && !status.code().is_some_and(|c| accepted.contains(&c))
    {
        return Err(AsyncProcessError::Failed {
            status,
            stderr: String::from_utf8_lossy(&stderr).into_owned(),
        });
    }
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

/// Filesystem-only repository identity; call on a native filesystem worker.
/// Linked checkouts share their canonical common Git directory.
#[must_use]
pub fn git_key(cwd: &std::path::Path) -> String {
    use std::io::Read;
    #[cfg(unix)]
    use std::os::unix::fs::OpenOptionsExt;
    let read = |path: &std::path::Path| -> Option<String> {
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        // Avoid blocking on FIFOs; Windows has no equivalent flag.
        #[cfg(unix)]
        options.custom_flags(libc::O_NONBLOCK);
        let file = options.open(path).ok()?;
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
#[path = "async_process_tests.rs"]
mod tests;
