use super::command::{Runner, cmd, git};
use super::{
    LinuxImageTagInput, Options, checks, linux_image_tag, linux_platform, rust_components, version,
};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, path::Path, sync::Mutex, time::Instant};

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
        self.run_with_steps(name, command, None)
    }
    /// Run a stage and record its result, log, and the per-step records the
    /// release check wrote (in `steps_dir`, or the stage directory).
    fn run_with_steps(
        &self,
        name: &str,
        command: &mut std::process::Command,
        steps_dir: Option<&Path>,
    ) -> Result<()> {
        let destination = self.artifacts.join(name);
        fs::create_dir(&destination)?;
        command
            .current_dir(self.source)
            .env("RUNNER_TEMP", &destination)
            .env("CARGO_HUSKY_DONT_INSTALL_HOOKS", "1");
        println!("==> {name}: {command:?}");
        let started = Instant::now();
        let log = destination.join("run.log");
        let result = self.runner.logged(command, &log);
        let steps = read_steps(steps_dir.unwrap_or(&destination));
        self.record(
            name,
            json!({
                "status": if result.is_ok() {"passed"} else {"failed"},
                "seconds": started.elapsed().as_secs_f64(),
                "log": log,
                "steps": steps,
            }),
        )?;
        result.with_context(|| format!("{name} failed. See {}", log.display()))
    }
    /// One line per stage, plus the first error line of each failed step.
    fn summary(&self) -> Result<String> {
        let results = self
            .results
            .lock()
            .map_err(|_| anyhow::anyhow!("Release results lock poisoned"))?;
        let mut lines = vec!["Release summary:".to_owned()];
        for (stage, value) in results.iter() {
            let Some(status) = value["status"].as_str() else {
                continue;
            };
            let seconds = value["seconds"]
                .as_f64()
                .map(|s| format!(" ({s:.0} s)"))
                .unwrap_or_default();
            lines.push(format!("  {stage}: {status}{seconds}"));
            for step in value["steps"].as_array().into_iter().flatten() {
                if step["status"] == "failed" {
                    let error = step["error"]
                        .as_str()
                        .and_then(|e| e.lines().next())
                        .unwrap_or("failed");
                    lines.push(format!(
                        "    - {} failed: {error}",
                        step["name"].as_str().unwrap_or("?")
                    ));
                }
            }
            if status == "failed"
                && let Some(log) = value["log"].as_str()
            {
                lines.push(format!("    log: {log}"));
            }
        }
        Ok(lines.join("\n"))
    }
}

/// The step records a release check wrote into `dir`, if any.
fn read_steps(dir: &Path) -> Option<Vec<checks::Step>> {
    serde_json::from_slice(&fs::read(dir.join(checks::STEPS_FILE)).ok()?).ok()
}

/// The failed steps of a stage when all of them are fixture steps that a
/// rerun with `--only` can repeat. `None` when a rerun cannot help.
fn retryable_failures(dir: &Path) -> Option<Vec<String>> {
    let failed: Vec<String> = read_steps(dir)?
        .into_iter()
        .filter(|step| step.status == "failed")
        .map(|step| step.name)
        .collect();
    (!failed.is_empty()
        && failed
            .iter()
            .all(|name| checks::FIXTURE_STEPS.contains(&name.as_str())))
    .then_some(failed)
}

pub(super) struct ValidateInput<'a, R> {
    pub(super) source: &'a Path,
    pub(super) artifacts: &'a Path,
    pub(super) options: &'a Options,
    pub(super) runner: &'a R,
}

pub(super) fn validate(input: ValidateInput<'_, impl Runner>) -> Result<()> {
    let ValidateInput {
        source,
        artifacts,
        options,
        runner,
    } = input;
    let stages = Stages {
        source,
        artifacts,
        runner,
        results: Mutex::new(BTreeMap::from([
            ("host".into(), json!({"status":"pending"})),
            (
                "linux".into(),
                json!({"status":if options.runs_linux() {"pending"} else {"skipped; CI runs Linux checks"}}),
            ),
            (
                "windows".into(),
                json!({"status":if options.skip_windows {"skipped explicitly; macOS cross-check only"} else {"pending"}}),
            ),
        ])),
    };
    stages.save()?;
    let linux_plan = if options.runs_linux() {
        let platform = stages.linux_platform(options)?;
        Some((platform, stages.linux_image(options, platform)?))
    } else {
        None
    };
    let result = stages.run_platforms(linux_plan).and_then(|()| {
        if options.skip_windows {
            Ok(())
        } else {
            stages.windows(options)
        }
    });
    println!("{}", stages.summary()?);
    result
}

impl<R: Runner> Stages<'_, R> {
    /// Docker's native Linux platform, checked against an explicit image.
    fn linux_platform(&self, options: &Options) -> Result<&'static str> {
        let Self { source, runner, .. } = *self;
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
        self.record("linux-platform", json!({"platform":platform}))?;
        Ok(platform)
    }
    /// Reuse or build the Linux check image; returns its tag.
    fn linux_image(&self, options: &Options, platform: &str) -> Result<String> {
        let Self {
            source,
            artifacts,
            runner,
            ..
        } = *self;
        let toolchain = version::document(&source.join("rust-toolchain.toml"))?;
        let rust = version::field(&toolchain, &["toolchain", "channel"])?
            .as_str()
            .context("Missing Rust toolchain")?
            .to_owned();
        let components = rust_components(&toolchain)?;
        let image = if let Some(image) = &options.linux_image {
            image.clone()
        } else {
            linux_image_tag(LinuxImageTagInput {
                dockerfile: &fs::read(source.join("scripts/release-linux.Dockerfile"))?,
                rust: &rust,
                components: &components,
                platform,
            })
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
            self.reuse_image(options, &image)?;
        } else {
            self.run(
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
        Ok(image)
    }
    fn reuse_image(&self, options: &Options, image: &str) -> Result<()> {
        let destination = self.artifacts.join("linux-image");
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
        self.record("linux-image", json!({"status":"cached", "image":image}))
    }
}

impl<R: Runner + Sync> Stages<'_, R> {
    /// Run the host and Linux checks in parallel. The host's real-PTY and
    /// GUI fixtures use short timed waits that the Linux container's build
    /// can starve, so after Linux ends the failed fixture steps (only those)
    /// run once more. Other failures repeat every time and are not rerun.
    fn run_platforms(&self, linux_plan: Option<(&'static str, String)>) -> Result<()> {
        let parallel = linux_plan.is_some();
        let (host, linux) = self.run_platforms_together(linux_plan)?;
        let host = match host {
            Err(first) if parallel => match retryable_failures(&self.artifacts.join("host")) {
                Some(only) => {
                    let only = only.join(",");
                    println!("Retrying host fixture steps {only} alone after a parallel failure");
                    self.run(
                        "host-retry",
                        &mut cmd(
                            self.source,
                            "bash",
                            &["scripts/release-check.sh", "--only", &only],
                        ),
                    )
                }
                None => Err(first),
            },
            host => host,
        };
        host?;
        linux
    }
    fn run_platforms_together(
        &self,
        linux_plan: Option<(&'static str, String)>,
    ) -> Result<(Result<()>, Result<()>)> {
        let source = self.source;
        std::thread::scope(|scope| {
            let host_task = scope.spawn(move || {
                self.run(
                    "host",
                    &mut cmd(source, "bash", &["scripts/release-check.sh"]),
                )
            });
            let linux_task = linux_plan
                .map(|(platform, image)| scope.spawn(move || self.linux(platform, &image)));
            let host_result = host_task
                .join()
                .map_err(|_| anyhow::anyhow!("Host release check panicked"))?;
            let linux_result = match linux_task {
                Some(task) => task
                    .join()
                    .map_err(|_| anyhow::anyhow!("Linux release check panicked"))?,
                None => Ok(()),
            };
            Ok((host_result, linux_result))
        })
    }
    fn linux(&self, platform: &str, image: &str) -> Result<()> {
        let Self {
            source, artifacts, ..
        } = *self;
        let output = artifacts.join(platform.replace('/', "-"));
        let target_volume = format!(
            "type=volume,src=terminator-release-linux-target-{},dst=/build",
            platform.replace('/', "-")
        );
        // nextest writes its store under `<workspace>/target/nextest` and has
        // no override, so `/src/target` gets a writable volume over the
        // read-only source (`target/` exists: the host built this xtask).
        let nextest_volume = format!(
            "type=volume,src=terminator-release-linux-nextest-{},dst=/src/target",
            platform.replace('/', "-")
        );
        // Linked worktrees use a .git file with an absolute path outside /src.
        let common = git(
            source,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )?;
        fs::create_dir(&output)?;
        self.run_with_steps(
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
                    &format!("type=bind,src={},dst=/src,readonly", source.display()),
                    "--mount",
                    &format!("type=bind,src={common},dst={common},readonly"),
                    "--mount",
                    &format!("type=bind,src={},dst=/artifacts", output.display()),
                    "--mount",
                    &target_volume,
                    "--mount",
                    &nextest_volume,
                    "--mount",
                    "type=volume,src=terminator-release-linux-registry,dst=/root/.cargo/registry",
                    "--mount",
                    "type=volume,src=terminator-release-linux-git,dst=/root/.cargo/git",
                    "-e",
                    "CARGO_TARGET_DIR=/build",
                    "-e",
                    "RUNNER_TEMP=/artifacts",
                    // Passes the color choice `logged` makes; unset stays unset.
                    "-e",
                    "CARGO_TERM_COLOR",
                    image,
                    "bash",
                    "-euo",
                    "pipefail",
                    "-c",
                    "git config --global --add safe.directory /src; bash scripts/release-check.sh",
                ],
            ),
            Some(&output),
        )
    }
}

impl<R: Runner> Stages<'_, R> {
    fn windows(&self, options: &Options) -> Result<()> {
        let Self {
            source, artifacts, ..
        } = *self;
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
        self.run("windows", &mut command)
    }
}
