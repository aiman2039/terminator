use super::command::{Runner, cmd};
use super::{
    LinuxImageTagInput, Options, linux_image_tag, linux_platform, rust_components, version,
};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
    time::Instant,
};

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
        Some(stages.linux_platform(options)?)
    };
    let target = artifacts
        .parent()
        .and_then(Path::parent)
        .context("Missing artifact parent")?
        .join("release-target");
    let linux_plan = if options.skip_linux {
        None
    } else {
        let platform = platform.context("Missing Linux platform")?;
        Some((platform, stages.linux_image(options, platform)?))
    };
    stages.run_platforms(&target, linux_plan)?;
    if !options.skip_windows {
        stages.windows(options)?;
    }
    Ok(())
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
    /// Run the host and Linux checks in parallel.
    fn run_platforms(
        &self,
        target: &PathBuf,
        linux_plan: Option<(&'static str, String)>,
    ) -> Result<()> {
        let source = self.source;
        std::thread::scope(|scope| -> Result<()> {
            let host_task = scope.spawn(move || {
                self.run(
                    "host",
                    cmd(source, "bash", &["scripts/release-check.sh"])
                        .env("CARGO_TARGET_DIR", target),
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
            host_result?;
            linux_result?;
            Ok(())
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
        fs::create_dir(&output)?;
        self.run(
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
                    image,
                    "bash",
                    "-euo",
                    "pipefail",
                    "-c",
                    "cp -a --preserve=timestamps --no-preserve=ownership /snapshot/. /src/; bash scripts/release-check.sh",
                ],
            ),
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
