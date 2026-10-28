use super::command::{Runner, System, cmd};
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use std::{fs, path::Path, process::Command};

pub(super) fn validate_host(host: &str) -> Result<()> {
    ensure!(
        !host.is_empty() && !host.starts_with('-'),
        "Set TERMINATOR_WINDOWS_HOST to a Windows OpenSSH host (user@host)"
    );
    Ok(())
}

fn literal(value: &str) -> String {
    value.replace('\'', "''")
}

pub(super) fn powershell(root: &Path, host: &str, script: &str) -> Command {
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    cmd(
        root,
        "ssh",
        &[
            "-o",
            "BatchMode=yes",
            "--",
            host,
            "powershell.exe",
            "-NoProfile",
            "-NonInteractive",
            "-EncodedCommand",
            &STANDARD.encode(bytes),
        ],
    )
}

pub struct RunInput<'a> {
    pub source: &'a Path,
    pub artifacts: &'a Path,
    pub host: &'a str,
    pub bash: &'a str,
}

pub fn run(input: RunInput<'_>) -> Result<()> {
    let RunInput {
        source,
        artifacts,
        host,
        bash,
    } = input;
    run_with(
        RunWithInput {
            source,
            artifacts,
            host,
            bash,
        },
        &System,
    )
}

pub(super) struct RunWithInput<'a> {
    pub(super) source: &'a Path,
    pub(super) artifacts: &'a Path,
    pub(super) host: &'a str,
    pub(super) bash: &'a str,
}

/// One remote Windows check run over SSH.
struct Remote<'a, R> {
    runner: &'a R,
    source: &'a Path,
    host: &'a str,
    relative: String,
}

pub(super) fn run_with(input: RunWithInput<'_>, runner: &impl Runner) -> Result<()> {
    let RunWithInput {
        source,
        artifacts,
        host,
        bash,
    } = input;
    validate_host(host)?;
    let source = source.canonicalize()?;
    fs::create_dir_all(artifacts)?;
    let artifacts = artifacts.canonicalize()?;
    let session = Remote {
        runner,
        source: &source,
        host,
        relative: format!(".terminator-release/{}", uuid::Uuid::new_v4().simple()),
    };
    let remote = session.create_directory()?;
    let remote_literal = literal(&remote);
    session.upload_source()?;
    let result = session.run_checks(&remote_literal, bash);
    let downloaded = session.download_artifacts(&remote_literal, &artifacts);
    if result.is_err() && downloaded.is_err() {
        eprintln!("Could not download artifacts. Remote logs: {remote}/artifacts");
    }
    result?;
    downloaded
}

impl<R: Runner> Remote<'_, R> {
    /// Create the remote work directory; returns its path.
    fn create_directory(&self) -> Result<String> {
        let Self {
            runner,
            source,
            host,
            relative,
        } = self;
        let remote = runner.output(&mut powershell(
            source,
            host,
            &format!(
                r"
$ErrorActionPreference = 'Stop'
$base = Join-Path $env:USERPROFILE '{relative}'
New-Item -ItemType Directory -Force -Path (Join-Path $base 'artifacts') | Out-Null
Write-Output ($base.Replace('\', '/'))
"
            ),
        ))?;
        Ok(remote
            .lines()
            .last()
            .filter(|line| !line.trim().is_empty())
            .context("Missing Windows artifact path")?
            .to_owned())
    }
    fn upload_source(&self) -> Result<()> {
        let Self {
            runner,
            source,
            host,
            relative,
        } = self;
        let temporary = tempfile::Builder::new()
            .prefix("windows-release-")
            .tempdir()?;
        let archive = temporary.path().join("source.tar.gz");
        let encoder = flate2::write::GzEncoder::new(
            fs::File::create(&archive)?,
            flate2::Compression::default(),
        );
        let mut bundle = tar::Builder::new(encoder);
        bundle.follow_symlinks(false);
        bundle.append_dir_all("source", source)?;
        bundle.into_inner()?.finish()?;
        runner.run(
            cmd(source, "scp", &["-B"])
                .arg(&archive)
                .arg(format!("{host}:{relative}/source.tar.gz")),
        )?;
        Ok(())
    }
    fn run_checks(&self, remote_literal: &str, bash: &str) -> Result<()> {
        let Self {
            runner,
            source,
            host,
            ..
        } = self;
        runner.run(&mut powershell(
            source,
            host,
            &format!(
                r"
$ErrorActionPreference = 'Stop'
$base = '{remote_literal}'
tar -xzf (Join-Path $base 'source.tar.gz') -C $base
if ($LASTEXITCODE -ne 0) {{ exit $LASTEXITCODE }}
$env:RUNNER_TEMP = (Join-Path $base 'artifacts').Replace('\', '/')
$env:CARGO_TARGET_DIR = (Join-Path $env:USERPROFILE '.terminator-release/target').Replace('\', '/')
$env:CARGO_HUSKY_DONT_INSTALL_HOOKS = '1'
Set-Location (Join-Path $base 'source')
& '{}' 'scripts/release-check.sh'
exit $LASTEXITCODE
",
                literal(bash)
            ),
        ))
    }
    fn download_artifacts(&self, remote_literal: &str, artifacts: &Path) -> Result<()> {
        let Self {
            runner,
            source,
            host,
            relative,
        } = self;
        runner.run(&mut powershell(
            source,
            host,
            &format!(
                r"
$ErrorActionPreference = 'Stop'
tar -czf '{remote_literal}/artifacts.tar.gz' -C '{remote_literal}/artifacts' .
exit $LASTEXITCODE
"
            ),
        ))?;
        runner.run(
            cmd(
                source,
                "scp",
                &["-B", &format!("{host}:{relative}/artifacts.tar.gz")],
            )
            .arg(artifacts.join("artifacts.tar.gz")),
        )
    }
}

#[cfg(test)]
pub(super) fn decoded_script(command: &Command) -> String {
    let encoded = command.get_args().last().unwrap().to_str().unwrap();
    let bytes = STANDARD.decode(encoded).unwrap();
    let words = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes(pair.try_into().unwrap()))
        .collect::<Vec<_>>();
    String::from_utf16(&words).unwrap()
}
