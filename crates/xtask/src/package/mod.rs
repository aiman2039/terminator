mod assemble;
mod dmg;
mod platform;

use crate::harness::root;
use anyhow::{Context, Result, anyhow, ensure};
pub use assemble::{AssembleInput, assemble};
pub use dmg::{LocalDmgInput, dmg, local_dmg};
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
struct StageTimer(&'static str, std::time::Instant);
impl StageTimer {
    fn new(name: &'static str) -> Self {
        Self(name, std::time::Instant::now())
    }
}
impl Drop for StageTimer {
    fn drop(&mut self) {
        println!(
            "{} duration: {:.2}s",
            self.0,
            self.1.elapsed().as_secs_f64()
        );
    }
}

fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let target = destination.join(entry.file_name());
        if entry.file_type()?.is_symlink() {
            #[cfg(unix)]
            std::os::unix::fs::symlink(fs::read_link(entry.path())?, &target)?;
            #[cfg(not(unix))]
            anyhow::bail!("Symlink-preserving packaging requires Unix");
        } else if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else if entry.file_type()?.is_file() {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}
fn licenses(destination: &Path) -> Result<()> {
    fs::create_dir_all(destination)?;
    let vendor = root().join("vendor/codediff.nvim");
    let target = destination.join("codediff.nvim");
    fs::create_dir_all(&target)?;
    for name in [
        "LICENSE",
        "ATTRIBUTION.md",
        "UPSTREAM.md",
        "libvscode-diff/vendor/utf8proc_LICENSE.md",
    ] {
        fs::copy(
            vendor.join(name),
            target.join(Path::new(name).file_name().unwrap()),
        )?;
    }
    copy_tree(
        &root().join("crates/app/assets/fonts"),
        &destination.join("fonts"),
    )?;
    copy_tree(
        &root().join("crates/app/assets/icons"),
        &destination.join("icons"),
    )?;
    let mut cmd = Command::new("cargo");
    cmd.current_dir(root())
        .args(["metadata", "--format-version", "1", "--locked"]);
    // Metadata resolves the full dependency graph, including packages that
    // the native build did not need. Cold CI caches can exceed the fixture
    // helper's 20-second deadline while Cargo downloads those manifests.
    let metadata = terminator_core::run_command(
        cmd,
        terminator_core::CommandOptions {
            timeout: std::time::Duration::from_mins(5),
            stdout_limit: 8 * 1024 * 1024,
            ..Default::default()
        },
    )
    .context("Collect package licenses with cargo metadata")?;
    let metadata: Value = serde_json::from_slice(&metadata.stdout)?;
    for package in metadata
        .get("packages")
        .and_then(Value::as_array)
        .context("Cargo packages missing")?
    {
        let manifest = Path::new(package["manifest_path"].as_str().unwrap());
        for entry in fs::read_dir(manifest.parent().unwrap())? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if entry.file_type()?.is_file()
                && ["LICENSE", "LICENCE", "COPYING", "NOTICE"]
                    .iter()
                    .any(|prefix| name.starts_with(prefix))
            {
                let target = destination.join(format!(
                    "{}-{}",
                    package["name"].as_str().unwrap(),
                    package["version"].as_str().unwrap()
                ));
                fs::create_dir_all(&target)?;
                fs::copy(entry.path(), target.join(name.as_ref()))?;
            }
        }
    }
    let info = metadata
        .get("packages")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("Cargo packages missing"))?
        .iter()
        .map(|p| serde_json::json!({"name":p["name"],"version":p["version"],"license":p["license"],"repository":p["repository"]}))
        .collect::<Vec<_>>();
    fs::write(
        destination.join("dependencies.json"),
        serde_json::to_vec_pretty(&info)?,
    )?;
    Ok(())
}
pub fn run(debug: bool, timings: bool, output_dir: Option<PathBuf>) -> Result<()> {
    let build_timer = StageTimer::new("Build");
    let shell = xshell::Shell::new()?;
    let _cwd = shell.push_dir(root());
    // xtask is already running; an optimized copy is not part of the package.
    let release_flag = (!debug).then_some("--release");
    let timing_flag = timings.then_some("--timings");
    xshell::cmd!(shell, "cargo build --locked -p terminator -p terminator-daemon -p terminator-hook {release_flag...} {timing_flag...}").run()?;
    drop(build_timer);
    let _package_timer = StageTimer::new("Package");
    let binaries = std::env::var_os("CARGO_TARGET_DIR")
        .map_or_else(|| root().join("target"), PathBuf::from)
        .join(if debug { "debug" } else { "release" });
    let destination = output_dir.unwrap_or_else(|| root().join("target/package"));
    fs::create_dir_all(&destination)?;
    let staging = tempfile::Builder::new()
        .prefix(".package-")
        .tempdir_in(&destination)?;
    let app = staging.path().join(if cfg!(target_os = "macos") {
        "Terminator.app"
    } else {
        "terminator"
    });
    let executables = if cfg!(target_os = "macos") {
        app.join("Contents/MacOS")
    } else {
        app.clone()
    };
    fs::create_dir_all(&executables)?;
    copy_executables(&binaries, &executables)?;
    if cfg!(target_os = "macos") {
        platform::macos(&app, &destination)?;
    } else if cfg!(target_os = "windows") {
        platform::windows(&app, staging.path(), &destination)?;
    } else {
        platform::linux(&app, staging.path(), &destination)?;
    }
    Ok(())
}

fn copy_executables(binaries: &Path, executables: &Path) -> Result<()> {
    for name in [
        terminator_core::exe_name("terminator"),
        terminator_core::exe_name("terminator-daemon"),
        terminator_core::exe_name("terminator-hook"),
    ] {
        let target = executables.join(&name);
        fs::copy(binaries.join(&name), &target)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&target, fs::Permissions::from_mode(0o755))?;
        }
        ensure!(
            terminator_core::executable_available(&target),
            "Packaged executable is unavailable: {name}"
        );
    }
    Ok(())
}
