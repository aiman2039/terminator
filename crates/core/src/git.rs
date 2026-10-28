//! Shared Git command setup: `git -C <dir>`, never taking optional locks.
//!
//! This lives in core rather than `terminator-git` because core's own metadata
//! refresh uses it, and a core → git → core dependency would be a cycle.
//! `terminator-git` re-exports these runners alongside the status, ignore, and
//! snapshot vocabulary. Every helper takes the caller's [`CommandOptions`];
//! timeouts and stdout limits stay per caller.
use crate::{CommandOptions, ProcessError, run_command};
#[cfg(feature = "async-client")]
use std::ffi::OsString;
use std::{ffi::OsStr, path::Path, process::Command};

/// `git -C <cwd> <args>` without optional locks.
pub fn command<A: AsRef<OsStr>>(cwd: &Path, args: &[A]) -> Command {
    command_with(OsStr::new("git"), cwd, args)
}

/// Same as [`command`], with an explicit program (e.g. a resolved absolute
/// path instead of a `PATH` lookup at spawn time).
pub fn command_with<A: AsRef<OsStr>>(program: &OsStr, cwd: &Path, args: &[A]) -> Command {
    let mut command = Command::new(program);
    command
        .env("GIT_OPTIONAL_LOCKS", "0")
        .arg("-C")
        .arg(cwd)
        .args(args);
    command
}

/// Run a read-only Git command to completion on the calling thread.
pub fn run_blocking<A: AsRef<OsStr>>(
    cwd: &Path,
    args: &[A],
    options: CommandOptions,
) -> Result<Vec<u8>, ProcessError> {
    Ok(run_command(command(cwd, args), options)?.stdout)
}

/// Run a read-only Git command through the shared process pool. The
/// repository key is computed on the filesystem pool, as before.
#[cfg(feature = "async-client")]
pub struct RunInput<'a> {
    pub cwd: &'a Path,
    pub args: Vec<OsString>,
    pub options: CommandOptions,
}

#[cfg(feature = "async-client")]
pub async fn run(
    processes: &crate::async_process::Processes,
    files: &crate::async_service::NativePool,
    input: RunInput<'_>,
) -> Result<Vec<u8>, crate::async_process::AsyncProcessError> {
    let RunInput { cwd, args, options } = input;
    let command = command(cwd, &args);
    let directory = cwd.to_owned();
    let key = files
        .run(&crate::async_service::CancellationToken::new(), move || {
            Ok::<_, crate::async_service::Failure>(crate::async_process::git_key(&directory))
        })
        .await?;
    Ok(processes.run(command, options, Some(key)).await?.stdout)
}
