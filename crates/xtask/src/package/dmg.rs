use super::{Context, Path, PathBuf, Result, StageTimer, ensure, fs, run};
use crate::harness::{output, root};
use std::process::Command;

/// Use create-dmg's Finder layout support; never silently skip presentation in CI.
pub fn dmg(app: &Path, destination: &Path) -> Result<()> {
    ensure!(cfg!(target_os = "macos"), "DMG packaging requires macOS");
    ensure!(!destination.exists(), "DMG output already exists");
    for name in ["terminator", "terminator-daemon", "terminator-hook"] {
        ensure!(
            terminator_core::executable_available(&app.join("Contents/MacOS").join(name)),
            "Missing or non-executable bundle component: {name}"
        );
    }
    let staging = tempfile::Builder::new()
        .prefix("terminator-dmg-")
        .tempdir()?;
    let source = staging.path().join("source");
    fs::create_dir(&source)?;
    // ditto retains the signed bundle's metadata and framework symlinks.
    let mut copy = Command::new("ditto");
    copy.arg(app).arg(source.join("Terminator.app"));
    output(copy)?;
    let background = staging.path().join("background.png");
    dmg_background(&background)?;
    let mut create = Command::new("create-dmg");
    create
        .args([
            "--volname",
            "Terminator",
            "--window-pos",
            "200",
            "120",
            "--window-size",
            "640",
            "440",
            "--icon-size",
            "96",
            "--text-size",
            "14",
            "--icon",
            "Terminator.app",
            "170",
            "195",
            "--hide-extension",
            "Terminator.app",
            "--app-drop-link",
            "470",
            "195",
            "--background",
        ])
        .arg(background)
        .arg(destination)
        .arg(source);
    terminator_core::run_command(create, terminator_core::CommandOptions {
        timeout: std::time::Duration::from_mins(5),
        stdout_limit: 1024 * 1024,
        ..Default::default()
    }).context("DMG creation failed (requires create-dmg and a macOS desktop; install with brew install create-dmg)")?;
    Ok(())
}

fn dmg_background(destination: &Path) -> Result<()> {
    let mut options = resvg::usvg::Options::default();
    options.fontdb_mut().load_system_fonts();
    let tree =
        resvg::usvg::Tree::from_str(include_str!("../../assets/dmg-background.svg"), &options)?;
    let mut pixels =
        resvg::tiny_skia::Pixmap::new(640, 400).context("DMG background allocation failed")?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::identity(),
        &mut pixels.as_mut(),
    );
    pixels.save_png(destination)?;
    Ok(())
}

/// Publish a unique directory only once both the app and compressed image exist.
/// Inputs for `xtask local-dmg`.
pub struct LocalDmgInput {
    pub release: bool,
    pub styled: bool,
    pub destination: Option<PathBuf>,
    pub timings: bool,
}

pub fn local_dmg(input: LocalDmgInput) -> Result<()> {
    let LocalDmgInput {
        release,
        styled,
        destination,
        timings,
    } = input;
    ensure!(cfg!(target_os = "macos"), "local-dmg requires macOS");
    if styled {
        let mut prerequisite = Command::new("create-dmg");
        prerequisite.arg("--version");
        output(prerequisite).context("Styled DMGs require create-dmg: brew install create-dmg")?;
    }
    let destination = destination.unwrap_or_else(|| root().join("target/local-dmg"));
    fs::create_dir_all(&destination)?;
    let staging = tempfile::Builder::new()
        .prefix(".building-")
        .tempdir_in(&destination)?;
    run(!release, timings, Some(staging.path().to_owned()))?;
    let dmg_timer = StageTimer::new("DMG");
    let app = staging.path().join("Terminator.app");
    let image = staging.path().join("Terminator.dmg");
    if styled {
        dmg(&app, &image)?;
    } else {
        plain_dmg(&app, staging.path(), &image)?;
    }
    drop(dmg_timer);
    let mut verify = Command::new("hdiutil");
    verify.arg("verify").arg(&image);
    output(verify)?;
    let mut signature = Command::new("codesign");
    signature.args(["--verify", "--deep", "--strict"]).arg(&app);
    output(signature)?;
    let unique = staging
        .path()
        .file_name()
        .context("Missing staging name")?
        .to_string_lossy()
        .replace(".building-", "build-");
    let final_dir = destination.join(unique);
    ensure!(
        !final_dir.exists(),
        "Output already exists: {}",
        final_dir.display()
    );
    fs::rename(staging.path(), &final_dir)?;
    println!(
        "App: {}\nDMG: {}",
        final_dir.join("Terminator.app").display(),
        final_dir.join("Terminator.dmg").display()
    );
    Ok(())
}

/// A plain compressed DMG holding the app and an Applications link.
fn plain_dmg(app: &Path, staging: &Path, image: &Path) -> Result<()> {
    let source = staging.join("image-source");
    fs::create_dir(&source)?;
    let mut copy = Command::new("ditto");
    copy.arg(app).arg(source.join("Terminator.app"));
    output(copy)?;
    #[cfg(unix)]
    std::os::unix::fs::symlink("/Applications", source.join("Applications"))?;
    let mut create = Command::new("hdiutil");
    create
        .args([
            "create",
            "-volname",
            "Terminator",
            "-format",
            "UDZO",
            "-srcfolder",
        ])
        .arg(&source)
        .arg(image);
    terminator_core::run_command(
        create,
        terminator_core::CommandOptions {
            timeout: std::time::Duration::from_mins(5),
            stdout_limit: 1024 * 1024,
            ..Default::default()
        },
    )
    .context("Plain DMG creation failed")?;
    fs::remove_dir_all(source)?;
    Ok(())
}
