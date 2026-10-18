pub mod checks;
mod command;
#[cfg(test)]
mod tests;
pub mod version;
pub mod windows;

use anyhow::{Context, Result, ensure};
use clap::Args;
use command::{Runner, System, cmd, git};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
    time::Instant,
};

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
    execute(&root, &options, &System, |source, artifacts| {
        validate(source, artifacts, &options, &System)
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
fn export_source(
    root: &Path,
    temporary: &Path,
    source: &Path,
    marker: &Path,
    original: &str,
) -> Result<(String, String)> {
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

fn execute(
    root: &Path,
    options: &Options,
    runner: &impl Runner,
    validate: impl FnOnce(&Path, &Path) -> Result<()>,
) -> Result<()> {
    let head = git(root, &["rev-parse", "HEAD"])?;
    let branch = git(root, &["symbolic-ref", "--short", "HEAD"])?;
    if !options.check_only {
        ensure!(
            branch == "master",
            "Release must run from master. Use --check-only on other branches."
        );
        runner.run(&mut cmd(root, "gh", &["auth", "status"]))?;
        runner.run(&mut cmd(
            root,
            "git",
            &["fetch", "--no-tags", "origin", "master"],
        ))?;
        let remote = git(root, &["rev-parse", "refs/remotes/origin/master"])?;
        runner.run(&mut cmd(
            root,
            "git",
            &["merge-base", "--is-ancestor", &remote, &head],
        ))?;
    }
    let parent = root.join(".artifacts/releases");
    fs::create_dir_all(&parent)?;
    let artifacts = tempfile::Builder::new()
        .prefix("run-")
        .tempdir_in(parent)?
        .keep();
    println!("Release logs: {}", artifacts.display());
    let temporary = tempfile::Builder::new()
        .prefix("terminator-release-")
        .tempdir()?;
    let original = source_tree(root, temporary.path())?;
    let (source, marker) = export_paths(root);
    let (tag, validated) =
        if let Some((tag, validated)) = reusable_export(&source, &marker, &original) {
            println!(
                "Reusing the validated source for {tag}: {}",
                source.display()
            );
            (tag, validated)
        } else {
            export_source(root, temporary.path(), &source, &marker, &original)?
        };
    if !options.check_only {
        ensure!(
            git(root, &["tag", "--list", &tag])?.is_empty()
                && git(
                    root,
                    &["ls-remote", "--tags", "origin", &format!("refs/tags/{tag}")]
                )?
                .is_empty(),
            "Tag {tag} already exists. No files changed."
        );
    }
    fs::write(
        artifacts.join("source.json"),
        serde_json::to_vec_pretty(&json!({
            "base_commit":head, "original_tree":original, "validated_tree":validated, "tag":tag,
        }))?,
    )?;
    validate(&source, &artifacts)?;
    ensure!(
        git(&source, &["status", "--porcelain"])?.is_empty()
            && git(&source, &["rev-parse", "HEAD^{tree}"])? == validated,
        "Checks changed source files. Review the logs; no release was made."
    );
    ensure!(
        git(root, &["rev-parse", "HEAD"])? == head
            && git(root, &["symbolic-ref", "--short", "HEAD"])? == branch
            && source_tree(root, temporary.path())? == original,
        "Source changed during validation. Run the checks again."
    );
    if options.check_only {
        println!(
            "Selected checks passed for {tag}. Version, index, commits and remote are unchanged."
        );
        return Ok(());
    }
    for name in ["Cargo.toml", "Cargo.lock"] {
        fs::copy(source.join(name), root.join(name))?;
    }
    git(root, &["add", "--all"])?;
    ensure!(
        git(root, &["write-tree"])? == validated,
        "Staged source differs from the validated source. Nothing pushed."
    );
    runner.run(
        cmd(root, "git", &["commit", "-m", &format!("Release {tag}")])
            .env("TERMINATOR_RELEASE_VALIDATED_TREE", &validated),
    )?;
    ensure!(
        git(root, &["rev-parse", "HEAD^{tree}"])? == validated,
        "Commit hooks changed the validated source. Nothing pushed."
    );
    let revision = git(root, &["rev-parse", "HEAD"])?;
    git(root, &["tag", &tag, &revision])?;
    runner.run(&mut cmd(
        root,
        "git",
        &[
            "push",
            "--atomic",
            "origin",
            &format!("{revision}:refs/heads/master"),
            &format!("refs/tags/{tag}"),
        ],
    ))?;
    runner.run(&mut cmd(
        root,
        "gh",
        &[
            "workflow",
            "run",
            "release.yaml",
            "--ref",
            "master",
            "-f",
            &format!("tag={tag}"),
        ],
    ))
    .with_context(|| format!("{tag} was pushed, but workflow dispatch failed. Retry only: gh workflow run release.yaml --ref master -f tag={tag} (if master has moved, use --ref {tag} without the tag input)"))?;
    println!(
        "Started Release for {tag} ({revision}). Logs: {}",
        artifacts.display()
    );
    Ok(())
}

struct Stages<'a, R> {
    source: &'a Path,
    artifacts: &'a Path,
    runner: &'a R,
    results: Mutex<BTreeMap<String, Value>>,
}
impl<R: Runner> Stages<'_, R> {
    fn save(&self) -> Result<()> {
        let results = self
            .results
            .lock()
            .map_err(|_| anyhow::anyhow!("Release results lock poisoned"))?;
        fs::write(
            self.artifacts.join("results.json"),
            serde_json::to_vec_pretty(&*results)?,
        )?;
        Ok(())
    }
    fn record(&self, name: &str, value: Value) -> Result<()> {
        let mut results = self
            .results
            .lock()
            .map_err(|_| anyhow::anyhow!("Release results lock poisoned"))?;
        results.insert(name.into(), value);
        fs::write(
            self.artifacts.join("results.json"),
            serde_json::to_vec_pretty(&*results)?,
        )?;
        Ok(())
    }
    fn run(&self, name: &str, command: &mut std::process::Command) -> Result<()> {
        let destination = self.artifacts.join(name);
        fs::create_dir(&destination)?;
        command
            .current_dir(self.source)
            .env("RUNNER_TEMP", &destination)
            .env("CARGO_HUSKY_DONT_INSTALL_HOOKS", "1");
        println!("==> {name}: {command:?}");
        let started = Instant::now();
        let result = self.runner.logged(command, &destination.join("run.log"));
        self.record(
            name,
            json!({"status":if result.is_ok() {"passed"} else {"failed"}, "seconds":started.elapsed().as_secs_f64()}),
        )?;
        result.with_context(|| {
            format!(
                "{name} failed. See {}",
                destination.join("run.log").display()
            )
        })
    }
}

fn validate(
    source: &Path,
    artifacts: &Path,
    options: &Options,
    runner: &impl Runner,
) -> Result<()> {
    let stages = Stages {
        source,
        artifacts,
        runner,
        results: Mutex::new(BTreeMap::from([
            ("host".into(), json!({"status":"pending"})),
            (
                "linux".into(),
                json!({"status":if options.skip_linux {"skipped explicitly"} else {"pending"}}),
            ),
            (
                "windows".into(),
                json!({"status":if options.skip_windows {"skipped explicitly; macOS cross-check only"} else {"pending"}}),
            ),
        ])),
    };
    stages.save()?;
    let platform = if options.skip_linux {
        None
    } else {
        let platform = linux_platform(&runner.output(&mut cmd(
            source,
            "docker",
            &["info", "--format", "{{.Architecture}}"],
        ))?)?;
        if let Some(image) = &options.linux_image {
            let arch = runner
                .output(&mut cmd(
                    source,
                    "docker",
                    &["image", "inspect", "--format", "{{.Architecture}}", image],
                ))
                .with_context(|| {
                    format!("Selected Linux image {image} is not available locally")
                })?;
            ensure!(
                format!("linux/{arch}") == platform,
                "Selected Linux image {image} uses {arch}, but Docker requires native {platform}. Omit --linux-image to build the native image; emulation cannot verify shell identity"
            );
        }
        println!("Linux runtime platform: {platform} (native Docker architecture)");
        stages.record("linux-platform", json!({"platform":platform}))?;
        Some(platform)
    };
    let target = artifacts
        .parent()
        .and_then(Path::parent)
        .context("Missing artifact parent")?
        .join("release-target");
    let linux_plan = if !options.skip_linux {
        let platform = platform.context("Missing Linux platform")?;
        let toolchain = version::document(&source.join("rust-toolchain.toml"))?;
        let rust = version::field(&toolchain, &["toolchain", "channel"])?
            .as_str()
            .context("Missing Rust toolchain")?
            .to_owned();
        let components = rust_components(&toolchain)?;
        let image = if let Some(image) = &options.linux_image {
            image.clone()
        } else {
            linux_image_tag(
                &fs::read(source.join("scripts/release-linux.Dockerfile"))?,
                &rust,
                &components,
                platform,
            )
        };
        let context = artifacts.join("linux-build-context");
        fs::create_dir(&context)?;
        let cached = if options.rebuild_linux_image {
            false
        } else if options.linux_image.is_some() {
            // Already inspected above during platform architecture check.
            true
        } else {
            runner
                .output(&mut cmd(
                    source,
                    "docker",
                    &["image", "inspect", "--format", "{{.Id}}", &image],
                ))
                .is_ok()
        };
        if cached {
            let destination = artifacts.join("linux-image");
            fs::create_dir(&destination)?;
            fs::write(
                destination.join("run.log"),
                format!(
                    "Reusing local image {image}; selection: {}.\n",
                    if options.linux_image.is_some() {
                        "explicit --linux-image override"
                    } else {
                        "matching configuration"
                    }
                ),
            )?;
            println!("Reusing local Linux image {image}");
            stages.record("linux-image", json!({"status":"cached", "image":image}))?;
        } else {
            stages.run(
                "linux-image",
                &mut cmd(
                    source,
                    "docker",
                    &[
                        "build",
                        "--platform",
                        platform,
                        "-t",
                        &image,
                        "--build-arg",
                        &format!("RUST_VERSION={rust}"),
                        "--build-arg",
                        &format!("RUST_COMPONENTS={components}"),
                        "-f",
                        "scripts/release-linux.Dockerfile",
                        &context.to_string_lossy(),
                    ],
                ),
            )?;
        }
        Some((platform, image))
    } else {
        None
    };
    let stages_ref = &stages;
    let target_ref = &target;
    std::thread::scope(|scope| -> Result<()> {
        let host_task = scope.spawn(move || {
            stages_ref.run(
                "host",
                cmd(source, "bash", &["scripts/release-check.sh"])
                    .env("CARGO_TARGET_DIR", target_ref),
            )
        });
        let linux_task = linux_plan.map(|(platform, image)| {
            scope.spawn(move || -> Result<()> {
                let output = artifacts.join(platform.replace('/', "-"));
                let target_volume = format!(
                    "type=volume,src=terminator-release-linux-target-{},dst=/build",
                    platform.replace('/', "-")
                );
                fs::create_dir(&output)?;
                stages_ref.run(
                    "linux",
                    &mut cmd(
                        source,
                        "docker",
                        &[
                            "run",
                            "--rm",
                            "--pull",
                            "never",
                            "--platform",
                            platform,
                            "--mount",
                            &format!("type=bind,src={},dst=/snapshot,readonly", source.display()),
                            "--mount",
                            &format!("type=bind,src={},dst=/artifacts", output.display()),
                            "--mount",
                            &target_volume,
                            "--mount",
                            "type=volume,src=terminator-release-linux-registry,dst=/root/.cargo/registry",
                            "--mount",
                            "type=volume,src=terminator-release-linux-git,dst=/root/.cargo/git",
                            "-e",
                            "CARGO_TARGET_DIR=/build",
                            "-e",
                            "RUNNER_TEMP=/artifacts",
                            &image,
                            "bash",
                            "-euo",
                            "pipefail",
                            "-c",
                            "cp -a --preserve=timestamps --no-preserve=ownership /snapshot/. /src/; bash scripts/release-check.sh",
                        ],
                    ),
                )
            })
        });
        let host_result = host_task
            .join()
            .map_err(|_| anyhow::anyhow!("Host release check panicked"))?;
        let linux_result = match linux_task {
            Some(task) => task
                .join()
                .map_err(|_| anyhow::anyhow!("Linux release check panicked"))?,
            None => Ok(()),
        };
        host_result?;
        linux_result?;
        Ok(())
    })?;
    if !options.skip_windows {
        let output = artifacts.join("windows-runtime");
        fs::create_dir(&output)?;
        let mut command = if let Some(program) = &options.windows_runner {
            let mut command = cmd(source, program, &[]);
            command.arg(source).arg(&output);
            command
        } else {
            let mut command = cmd(source, std::env::current_exe()?, &["release-windows"]);
            command
                .arg(source)
                .arg(&output)
                .arg("--host")
                .arg(
                    options
                        .windows_host
                        .as_deref()
                        .context("Missing Windows host")?,
                )
                .arg("--bash")
                .arg(&options.windows_bash);
            command
        };
        stages.run("windows", &mut command)?;
    }
    Ok(())
}

fn linux_platform(architecture: &str) -> Result<&'static str> {
    match architecture.trim() {
        "aarch64" | "arm64" => Ok("linux/arm64"),
        "x86_64" | "amd64" => Ok("linux/amd64"),
        other => anyhow::bail!("Unsupported Docker architecture: {other}"),
    }
}

fn linux_image_tag(dockerfile: &[u8], rust: &str, components: &str, platform: &str) -> String {
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
