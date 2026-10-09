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
        linux_platform(&System.output(&mut cmd(
            &root,
            "docker",
            &["info", "--format", "{{.Architecture}}"],
        ))?)?;
    }
    for plugin in ["audit", "deny"] {
        System.run(&mut cmd(&root, "cargo", &[plugin, "--version"]))?;
    }
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
    let source = temporary.path().join("source");
    fs::create_dir(&source)?;
    let prefix = format!("--prefix={}/", source.display());
    System.run(
        cmd(root, "git", &["checkout-index", "--all", &prefix])
            .env("GIT_INDEX_FILE", temporary.path().join("index")),
    )?;
    let tag = format!("v{}", version::bump(&source)?);
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
    let object_format = git(root, &["rev-parse", "--show-object-format"])?;
    git(
        &source,
        &[
            "init",
            "--quiet",
            &format!("--object-format={object_format}"),
        ],
    )?;
    git(&source, &["add", "--force", "--all"])?;
    git(
        &source,
        &[
            "-c",
            "user.name=Release validation",
            "-c",
            "user.email=release@localhost",
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--quiet",
            "-m",
            &format!("Validate {tag}"),
        ],
    )?;
    let validated = git(&source, &["rev-parse", "HEAD^{tree}"])?;
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
    runner.run(&mut cmd(root, "gh", &["workflow", "run", "release.yaml", "--ref", &tag]))
        .with_context(|| format!("{tag} was pushed, but workflow dispatch failed. Retry only: gh workflow run release.yaml --ref {tag}"))?;
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
    results: BTreeMap<String, Value>,
}
impl<R: Runner> Stages<'_, R> {
    fn save(&self) -> Result<()> {
        fs::write(
            self.artifacts.join("results.json"),
            serde_json::to_vec_pretty(&self.results)?,
        )?;
        Ok(())
    }
    fn run(&mut self, name: &str, command: &mut std::process::Command) -> Result<()> {
        let destination = self.artifacts.join(name);
        fs::create_dir(&destination)?;
        command
            .current_dir(self.source)
            .env("RUNNER_TEMP", &destination)
            .env("CARGO_HUSKY_DONT_INSTALL_HOOKS", "1");
        println!("==> {name}: {command:?}");
        let started = Instant::now();
        let result = self.runner.logged(command, &destination.join("run.log"));
        self.results.insert(name.into(), json!({"status":if result.is_ok() {"passed"} else {"failed"}, "seconds":started.elapsed().as_secs_f64()}));
        self.save()?;
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
    let mut stages = Stages {
        source,
        artifacts,
        runner,
        results: BTreeMap::from([
            ("host".into(), json!({"status":"pending"})),
            (
                "linux".into(),
                json!({"status":if options.skip_linux {"skipped explicitly"} else {"pending"}}),
            ),
            (
                "windows".into(),
                json!({"status":if options.skip_windows {"skipped explicitly; macOS cross-check only"} else {"pending"}}),
            ),
        ]),
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
        stages
            .results
            .insert("linux-platform".into(), json!({"platform":platform}));
        stages.save()?;
        Some(platform)
    };
    let target = artifacts
        .parent()
        .and_then(Path::parent)
        .context("Missing artifact parent")?
        .join("release-target");
    stages.run(
        "release-flow-tests",
        cmd(
            source,
            "cargo",
            &["test", "-p", "xtask", "--locked", "release::"],
        )
        .env("CARGO_TARGET_DIR", &target),
    )?;
    stages.run(
        "host",
        cmd(source, "bash", &["scripts/release-check.sh"]).env("CARGO_TARGET_DIR", &target),
    )?;
    if !options.skip_linux {
        let platform = platform.context("Missing Linux platform")?;
        let toolchain = version::document(&source.join("rust-toolchain.toml"))?;
        let rust = version::field(&toolchain, &["toolchain", "channel"])?
            .as_str()
            .context("Missing Rust toolchain")?;
        let components = rust_components(&toolchain)?;
        let image = if let Some(image) = &options.linux_image {
            image.clone()
        } else {
            linux_image_tag(
                &fs::read(source.join("scripts/release-linux.Dockerfile"))?,
                rust,
                &components,
                platform,
            )
        };
        let context = artifacts.join("linux-build-context");
        fs::create_dir(&context)?;
        let cached = if options.rebuild_linux_image {
            false
        } else {
            let inspected = runner.output(&mut cmd(
                source,
                "docker",
                &["image", "inspect", "--format", "{{.Id}}", &image],
            ));
            if options.linux_image.is_some() {
                inspected.with_context(|| {
                    format!("Selected Linux image {image} is not available locally")
                })?;
                true
            } else {
                inspected.is_ok()
            }
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
            stages.results.insert(
                "linux-image".into(),
                json!({"status":"cached", "image":image}),
            );
            stages.save()?;
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
        let output = artifacts.join(platform.replace('/', "-"));
        let target_volume = format!(
            "type=volume,src=terminator-release-linux-target-{},dst=/build",
            platform.replace('/', "-")
        );
        fs::create_dir(&output)?;
        stages.run("linux", &mut cmd(source, "docker", &["run", "--rm", "--pull", "never", "--platform", platform,
            "--mount", &format!("type=bind,src={},dst=/snapshot,readonly", source.display()),
            "--mount", &format!("type=bind,src={},dst=/artifacts", output.display()),
            "--mount", &target_volume,
            "--mount", "type=volume,src=terminator-release-linux-registry,dst=/root/.cargo/registry",
            "--mount", "type=volume,src=terminator-release-linux-git,dst=/root/.cargo/git",
            "-e", "CARGO_TARGET_DIR=/build", "-e", "RUNNER_TEMP=/artifacts", &image,
            "bash", "-euo", "pipefail", "-c", "cp -a --no-preserve=ownership /snapshot/. /src/; bash scripts/release-check.sh"]))?;
    }
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
    format!("terminator-release-check:config-{:x}", hash.finalize())
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
