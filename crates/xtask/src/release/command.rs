use anyhow::{Context, Result, ensure};
use std::{
    fs::File,
    io::{BufRead, BufReader, Read, Write},
    path::Path,
    process::{Command, Stdio},
    sync::Mutex,
};

pub(super) trait Runner {
    fn run(&self, command: &mut Command) -> Result<()>;
    fn output(&self, command: &mut Command) -> Result<String>;
    fn logged(&self, command: &mut Command, path: &Path) -> Result<()>;
}

pub(super) struct System;
impl Runner for System {
    fn run(&self, command: &mut Command) -> Result<()> {
        let status = command
            .status()
            .with_context(|| format!("Could not start {command:?}"))?;
        ensure!(status.success(), "Command failed ({status}): {command:?}");
        Ok(())
    }

    fn output(&self, command: &mut Command) -> Result<String> {
        let output = command
            .output()
            .with_context(|| format!("Could not start {command:?}"))?;
        ensure!(
            output.status.success(),
            "Command failed: {command:?}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(String::from_utf8(output.stdout)?.trim().to_owned())
    }

    fn logged(&self, command: &mut Command, path: &Path) -> Result<()> {
        let log = Mutex::new(File::create(path)?);
        let mut process = crate::harness::Process(
            command
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()?,
        );
        let stdout = process.0.stdout.take().context("Missing command stdout")?;
        let stderr = process.0.stderr.take().context("Missing command stderr")?;
        std::thread::scope(|scope| -> Result<()> {
            let errors = scope.spawn(|| pump(stderr, &log));
            pump(stdout, &log)?;
            errors
                .join()
                .map_err(|_| anyhow::anyhow!("Command log reader panicked"))??;
            Ok(())
        })?;
        let status = process.0.wait()?;
        ensure!(
            status.success(),
            "Command failed ({status}). See {}",
            path.display()
        );
        Ok(())
    }
}

fn pump(reader: impl Read, log: &Mutex<File>) -> Result<()> {
    let mut reader = BufReader::new(reader);
    let mut line = Vec::new();
    loop {
        line.clear();
        if reader.read_until(b'\n', &mut line)? == 0 {
            return Ok(());
        }
        let mut log = log
            .lock()
            .map_err(|_| anyhow::anyhow!("Command log lock poisoned"))?;
        log.write_all(&line)?;
        std::io::stdout().lock().write_all(&line)?;
    }
}

pub(super) fn cmd(root: &Path, program: impl AsRef<std::ffi::OsStr>, args: &[&str]) -> Command {
    let mut command = Command::new(program);
    command.current_dir(root).args(args);
    command
}

pub(super) fn git(root: &Path, args: &[&str]) -> Result<String> {
    System.output(&mut cmd(root, "git", args))
}
