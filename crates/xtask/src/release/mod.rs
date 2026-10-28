pub mod checks;
mod command;
mod publish;
#[cfg(test)]
mod tests;
mod validate;
pub mod version;
pub mod windows;

use anyhow::{Context, Result, ensure};
use clap::Args;
use command::{Runner, System, cmd, git};
use publish::{ExecuteInput, execute};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
#[cfg(test)]
use std::time::Instant;
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};
use validate::{ValidateInput, validate};

#[derive(Args, Debug, Default)]
pub struct Options {
    /// Validate without changing versions, commits or remotes.
    #[arg(long)]
    pub check_only: bool,
    /// Explicitly omit Linux execution.
    #[arg(long)]
    pub skip_linux: bool,
    /// Rebuild the Linux image even when its exact configuration is cached.
    #[arg(long)]
    pub rebuild_linux_image: bool,
    /// Use an existing local Linux image instead of building the repository recipe.
    #[arg(long, conflicts_with = "rebuild_linux_image")]
    pub linux_image: Option<String>,
    /// Explicitly omit Windows execution; keep macOS cross-checks.
    #[arg(long)]
    pub skip_windows: bool,
    /// Custom executable receiving source and artifact directories.
    #[arg(long, env = "TERMINATOR_WINDOWS_RUNNER")]
    pub windows_runner: Option<String>,
    /// Windows OpenSSH host used by the built-in runner.
    #[arg(long, env = "TERMINATOR_WINDOWS_HOST")]
    pub windows_host: Option<String>,
    #[arg(
        long,
        env = "TERMINATOR_WINDOWS_BASH",
        default_value = "C:/Program Files/Git/bin/bash.exe"
    )]
    pub windows_bash: String,
}

struct Lock {
    file: File,
}
impl Lock {
    fn acquire(root: &Path) -> Result<Self> {
        let path = root.join(".artifacts/release.lock");
        fs::create_dir_all(path.parent().unwrap())?;
        ensure!(
            !path.is_dir(),
            "Legacy release lock directory exists: {}. Remove the empty directory only after checking that no older release process is running",
            path.display()
        );
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .with_context(|| format!("Cannot open release lock: {}", path.display()))?;
        if let Err(error) = fs2::FileExt::try_lock_exclusive(&file) {
            if error.raw_os_error() == fs2::lock_contended_error().raw_os_error() {
                anyhow::bail!("Another release is running. Lock: {}", path.display());
            }
            return Err(error)
                .with_context(|| format!("Cannot acquire release lock: {}", path.display()));
        }
        file.set_len(0)?;
        writeln!(file, "pid={}", std::process::id())?;
        // Keep the inode in place: unlinking a lock file allows concurrent
        // callers to lock different files at the same path.
        Ok(Self { file })
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        // Release explicitly before close: another thread may have briefly
        // inherited this descriptor while spawning a child before exec.
        let _ = fs2::FileExt::unlock(&self.file);
    }
}

fn require(program: &str) -> Result<PathBuf> {
    terminator_core::find_executable(program)
        .with_context(|| format!("Missing required tool: {program}"))
}

pub fn run(mut options: Options) -> Result<()> {
    let root = crate::harness::root();
    let _lock = Lock::acquire(&root)?;
    validate_windows_configuration(&options)?;
    require("cargo")?;
    require("nvim")?;
    if cfg!(target_os = "macos") {
        require(&std::env::var("TERMINATOR_ZIG").unwrap_or_else(|_| "zig".into()))?;
    }
    if !options.skip_windows {
        if let Some(program) = &options.windows_runner {
            options.windows_runner = Some(
                require(program)?
                    .canonicalize()?
                    .to_string_lossy()
                    .into_owned(),
            );
        } else {
            windows::validate_host(options.windows_host.as_deref().unwrap())?;
            require("ssh")?;
            require("scp")?;
        }
    }
    if !options.skip_linux {
        require("docker")?;
    }
    std::thread::scope(|scope| -> Result<()> {
        let mut checks = Vec::new();
        for plugin in ["audit", "deny", "nextest"] {
            let root = &root;
            checks.push(
                scope.spawn(move || System.run(&mut cmd(root, "cargo", &[plugin, "--version"]))),
            );
        }
        for check in checks {
            check
                .join()
                .map_err(|_| anyhow::anyhow!("Plugin check panicked"))??;
        }
        Ok(())
    })?;
    execute(ExecuteInput {
        root: &root,
        options: &options,
        runner: &System,
        validate: |source, artifacts| {
            validate(ValidateInput {
                source,
                artifacts,
                options: &options,
                runner: &System,
            })
        },
    })
}

fn validate_windows_configuration(options: &Options) -> Result<()> {
    ensure!(
        options.skip_windows || options.windows_runner.is_some() || options.windows_host.is_some(),
        "Set TERMINATOR_WINDOWS_HOST or TERMINATOR_WINDOWS_RUNNER, or use --skip-windows. Cross-checks do not run Windows tests."
    );
    Ok(())
}

fn source_tree(root: &Path, temporary: &Path) -> Result<String> {
    let index = temporary.join("index");
    if index.exists() {
        fs::remove_file(&index)?;
    }
    for args in [
        ["read-tree", "HEAD"].as_slice(),
        ["add", "--all"].as_slice(),
    ] {
        System.run(cmd(root, "git", args).env("GIT_INDEX_FILE", &index))?;
    }
    System.output(cmd(root, "git", &["write-tree"]).env("GIT_INDEX_FILE", &index))
}

/// Exported source tree, kept between runs. Cargo reuses compiler artifacts
/// only while the source path and the file times stay unchanged, so a retry
/// of the same index tree reuses this export instead of copying the tree
/// again: a fresh copy has new file times and rebuilds every workspace and
/// vendored crate. The path stays outside the repository, because a nested
/// copy of the workspace leaves its excluded vendored crates without a root,
/// and it is derived from the repository path so two checkouts never share
/// one export.
fn hex_bytes(bytes: impl AsRef<[u8]>) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let bytes = bytes.as_ref();
    let mut out = String::with_capacity(bytes.len().saturating_mul(2));
    for &byte in bytes {
        let high = usize::from(byte >> 4);
        let low = usize::from(byte & 0x0f);
        if let Some(&digit) = HEX.get(high) {
            out.push(char::from(digit));
        }
        if let Some(&digit) = HEX.get(low) {
            out.push(char::from(digit));
        }
    }
    out
}

fn export_paths(root: &Path) -> (PathBuf, PathBuf) {
    let digest = hex_bytes(Sha256::digest(root.as_os_str().as_encoded_bytes()));
    let parent = std::env::temp_dir();
    let export = parent.join(format!(
        "terminator-release-source-{}",
        digest.get(..16).unwrap_or(&digest)
    ));
    let marker = parent.join(format!(
        "{}.json",
        export.file_name().unwrap().to_string_lossy()
    ));
    (export, marker)
}

/// Marker of the last completed export: the index tree it came from, the
/// tagged version it was bumped to, and its validated commit tree.
fn reusable_export(source: &Path, marker: &Path, original: &str) -> Option<(String, String)> {
    let marker: Value = serde_json::from_slice(&fs::read(marker).ok()?).ok()?;
    if marker.get("original")?.as_str()? != original {
        return None;
    }
    let validated = marker.get("validated")?.as_str()?.to_owned();
    if !source.is_dir()
        || marker.get("tag")?.as_str().is_none()
        || git(source, &["rev-parse", "HEAD^{tree}"]).ok()? != validated
        || !git(source, &["status", "--porcelain"]).ok()?.is_empty()
    {
        return None;
    }
    Some((marker.get("tag")?.as_str()?.to_owned(), validated))
}

/// Copy the current index tree into the export directory, bump its version,
/// commit it as the validated tree, and record the marker a retry reads.
struct ExportSourceInput<'a> {
    root: &'a Path,
    temporary: &'a Path,
    source: &'a Path,
    marker: &'a Path,
    original: &'a str,
}

fn export_source(input: ExportSourceInput<'_>) -> Result<(String, String)> {
    let ExportSourceInput {
        root,
        temporary,
        source,
        marker,
        original,
    } = input;
    if source.exists() {
        fs::remove_dir_all(source)?;
    }
    fs::create_dir_all(source)?;
    let prefix = format!("--prefix={}/", source.display());
    System.run(
        cmd(root, "git", &["checkout-index", "--all", &prefix])
            .env("GIT_INDEX_FILE", temporary.join("index")),
    )?;
    let tag = format!("v{}", version::bump(source)?);
    let object_format = git(root, &["rev-parse", "--show-object-format"])?;
    git(
        source,
        &[
            "init",
            "--quiet",
            &format!("--object-format={object_format}"),
        ],
    )?;
    // Disable background maintenance and auto-gc so the exported .git/objects
    // directory is never mutated while Docker copies /snapshot in parallel.
    for (key, value) in [("gc.auto", "0"), ("maintenance.auto", "false")] {
        git(source, &["config", key, value])?;
    }
    git(source, &["add", "--force", "--all"])?;
    git(
        source,
        &[
            "-c",
            "user.name=Release validation",
            "-c",
            "user.email=release@localhost",
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "gc.auto=0",
            "-c",
            "maintenance.auto=false",
            "commit",
            "--quiet",
            "-m",
            &format!("Validate {tag}"),
        ],
    )?;
    let validated = git(source, &["rev-parse", "HEAD^{tree}"])?;
    fs::write(
        marker,
        serde_json::to_vec_pretty(&json!({
            "original": original, "validated": validated, "tag": tag,
        }))?,
    )?;
    Ok((tag, validated))
}

fn linux_platform(architecture: &str) -> Result<&'static str> {
    match architecture.trim() {
        "aarch64" | "arm64" => Ok("linux/arm64"),
        "x86_64" | "amd64" => Ok("linux/amd64"),
        other => anyhow::bail!("Unsupported Docker architecture: {other}"),
    }
}

struct LinuxImageTagInput<'a> {
    dockerfile: &'a [u8],
    rust: &'a str,
    components: &'a str,
    platform: &'a str,
}

fn linux_image_tag(input: LinuxImageTagInput<'_>) -> String {
    let LinuxImageTagInput {
        dockerfile,
        rust,
        components,
        platform,
    } = input;
    let mut hash = Sha256::new();
    for part in [
        dockerfile,
        rust.as_bytes(),
        components.as_bytes(),
        platform.as_bytes(),
    ] {
        hash.update((part.len() as u64).to_le_bytes());
        hash.update(part);
    }
    format!(
        "terminator-release-check:config-{}",
        hex_bytes(hash.finalize())
    )
}

fn rust_components(toolchain: &toml_edit::DocumentMut) -> Result<String> {
    let mut components: BTreeSet<String> = ["clippy", "rustfmt", "llvm-tools-preview"]
        .map(str::to_owned)
        .into();
    if let Some(configured) = version::field(toolchain, &["toolchain"])?.get("components") {
        for component in configured
            .as_array()
            .context("Rust components must be an array")?
        {
            let component = component
                .as_str()
                .context("Rust component must be a string")?;
            ensure!(
                !component.is_empty()
                    && !component.starts_with('-')
                    && component
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_')),
                "Invalid Rust component: {component}"
            );
            components.insert(component.to_owned());
        }
    }
    Ok(components.into_iter().collect::<Vec<_>>().join(" "))
}
