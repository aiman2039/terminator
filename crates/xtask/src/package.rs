use crate::harness::{output, root};
use anyhow::{Context, Result, ensure};
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
    let metadata: Value = serde_json::from_slice(&output(cmd)?)?;
    for package in metadata["packages"]
        .as_array()
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
    let info=metadata["packages"].as_array().unwrap().iter().map(|p|serde_json::json!({"name":p["name"],"version":p["version"],"license":p["license"],"repository":p["repository"]})).collect::<Vec<_>>();
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
    for name in ["terminator", "terminator-daemon", "terminator-hook"] {
        let target = executables.join(name);
        fs::copy(binaries.join(name), &target)?;
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&target, fs::Permissions::from_mode(0o755))?;
        ensure!(
            terminator_core::executable_available(&target),
            "Packaged executable is unavailable: {name}"
        );
    }
    if cfg!(target_os = "macos") {
        let mut info = plist::Dictionary::new();
        let build_number = std::env::var("TERMINATOR_BUILD_NUMBER").unwrap_or_else(|_| "1".into());
        ensure!(
            build_number.parse::<u64>().is_ok_and(|n| n > 0),
            "Build number must be a positive integer"
        );
        for (key, value) in [
            ("CFBundleIdentifier", "dev.terminator.app"),
            ("CFBundleName", "Terminator"),
            ("CFBundleDisplayName", "Terminator"),
            ("CFBundleExecutable", "terminator"),
            ("CFBundleIconFile", "Terminator.icns"),
            ("CFBundlePackageType", "APPL"),
            ("CFBundleVersion", build_number.as_str()),
            ("CFBundleShortVersionString", env!("CARGO_PKG_VERSION")),
            ("LSMinimumSystemVersion", "12.0"),
            (
                "NSDesktopFolderUsageDescription",
                "Access project files you choose on your Desktop.",
            ),
            (
                "NSDocumentsFolderUsageDescription",
                "Access project files you choose in Documents.",
            ),
            (
                "NSDownloadsFolderUsageDescription",
                "Access project files you choose in Downloads.",
            ),
            (
                "NSRemovableVolumesUsageDescription",
                "Access projects you choose on removable volumes.",
            ),
            (
                "NSNetworkVolumesUsageDescription",
                "Access projects you choose on network volumes.",
            ),
        ] {
            info.insert(key.into(), value.into());
        }
        info.insert("NSHighResolutionCapable".into(), true.into());
        plist::Value::Dictionary(info).to_file_xml(app.join("Contents/Info.plist"))?;
        let resources = app.join("Contents/Resources");
        fs::create_dir_all(&resources)?;
        fs::copy(
            root().join("crates/app/assets/branding/terminator.icns"),
            resources.join("Terminator.icns"),
        )?;
        fs::copy(root().join("LICENSE"), resources.join("LICENSE"))?;
        copy_tree(&root().join("docs"), &resources.join("docs"))?;
        copy_tree(
            &root().join("crates/app/assets/radio"),
            &resources.join("radio"),
        )?;
        licenses(&resources.join("licenses"))?;
        let mut sign = Command::new("codesign");
        sign.args(["--force", "--deep", "--sign", "-"]).arg(&app);
        output(sign)?;
        let target = destination.join("Terminator.app");
        let backup = destination.join("Terminator.app.previous");
        ensure!(
            !backup.exists(),
            "Previous package backup already exists: {}",
            backup.display()
        );
        if target.exists() {
            fs::rename(&target, &backup)?;
        }
        if let Err(error) = fs::rename(&app, &target) {
            if backup.exists() {
                fs::rename(&backup, &target)?;
            }
            return Err(error.into());
        }
        if backup.exists() {
            fs::remove_dir_all(backup)?;
        }
        println!("{}", target.display());
    } else {
        fs::write(
            app.join("terminator.desktop"),
            "[Desktop Entry]\nType=Application\nName=Terminator\nExec=terminator\nIcon=terminator\nTerminal=false\nCategories=Development;TerminalEmulator;\n",
        )?;
        fs::copy(
            root().join("crates/app/assets/branding/terminator-512.png"),
            app.join("terminator.png"),
        )?;
        fs::copy(root().join("README.md"), app.join("README.md"))?;
        copy_tree(&root().join("crates/app/assets/radio"), &app.join("radio"))?;
        licenses(&app.join("licenses"))?;
        let file = fs::File::create(staging.path().join("terminator-linux.tar.gz"))?;
        let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut tar = tar::Builder::new(encoder);
        tar.append_dir_all("terminator", &app)?;
        tar.into_inner()?.finish()?;
        let target = destination.join("terminator-linux.tar.gz");
        fs::rename(staging.path().join("terminator-linux.tar.gz"), &target)?;
        println!("{}", target.display());
    }
    Ok(())
}

pub struct AssembleInput<'a> {
    pub app: &'a Path,
    pub sparkle: &'a Path,
    pub build_number: u64,
    pub destination: &'a Path,
}

/// Native jobs build each daemon and its embedded `CodeDiff` together. Assembly
/// embeds Sparkle into that Apple Silicon app; it never rebuilds or lipos slices.
pub fn assemble(input: AssembleInput<'_>) -> Result<()> {
    let AssembleInput {
        app,
        sparkle,
        build_number,
        destination,
    } = input;
    ensure!(cfg!(target_os = "macos"), "Assembly requires macOS");
    ensure!(build_number > 0, "Build number must be positive");
    // Detect restored/previous output before reading inputs or assembling.
    let target = new_assembly_output(destination)?;
    let public_key = sparkle_public_key()?;
    let app_info = validated_app_info(app)?;
    require_sparkle_version(sparkle)?;
    fs::create_dir_all(destination)?;
    let staging = tempfile::Builder::new()
        .prefix(".assemble-")
        .tempdir_in(destination)?;
    let staged = staging.path().join("Terminator.app");
    copy_tree(app, &staged)?;
    verify_app_executables(&staged)?;
    embed_sparkle(&staged, sparkle)?;
    write_update_plist(&staged, app_info, build_number, public_key)?;
    verify_arm64_macho(&staged)?;
    // Also check at publication time in case another task created the output.
    new_assembly_output(destination)?;
    fs::rename(staged, &target)?;
    println!("{}", target.display());
    Ok(())
}

fn sparkle_public_key() -> Result<String> {
    parse_sparkle_public_key(std::env::var("SPARKLE_PUBLIC_ED_KEY").ok())
}

fn parse_sparkle_public_key(value: Option<String>) -> Result<String> {
    let public_key = value.context("SPARKLE_PUBLIC_ED_KEY must contain the public update key")?;
    use base64::Engine;
    ensure!(
        base64::engine::general_purpose::STANDARD
            .decode(&public_key)?
            .len()
            == 32,
        "Invalid public Ed25519 key"
    );
    Ok(public_key)
}

fn validated_app_info(app: &Path) -> Result<plist::Dictionary> {
    let info = plist::Value::from_file(app.join("Contents/Info.plist"))?;
    for (key, expected) in [
        ("CFBundleIdentifier", "dev.terminator.app"),
        ("LSMinimumSystemVersion", "12.0"),
    ] {
        ensure!(
            info.as_dictionary()
                .and_then(|d| d.get(key))
                .and_then(plist::Value::as_string)
                == Some(expected),
            "Unexpected app metadata: {key}"
        );
    }
    info.into_dictionary().context("Missing app dictionary")
}

fn require_sparkle_version(sparkle: &Path) -> Result<()> {
    let framework_info =
        plist::Value::from_file(sparkle.join("Sparkle.framework/Resources/Info.plist"))?;
    ensure!(
        framework_info
            .as_dictionary()
            .and_then(|d| d.get("CFBundleShortVersionString"))
            .and_then(plist::Value::as_string)
            == Some("2.10.0"),
        "Expected Sparkle 2.10.0"
    );
    Ok(())
}

fn verify_app_executables(app: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    for name in ["terminator", "terminator-daemon", "terminator-hook"] {
        let path = app.join("Contents/MacOS").join(name);
        let mut verify = Command::new("lipo");
        verify.arg(&path).args(["-verify_arch", "arm64"]);
        output(verify)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

fn embed_sparkle(app: &Path, sparkle: &Path) -> Result<()> {
    copy_tree(
        &sparkle.join("Sparkle.framework"),
        &app.join("Contents/Frameworks/Sparkle.framework"),
    )?;
    fs::copy(
        sparkle.join("LICENSE"),
        app.join("Contents/Resources/licenses/Sparkle-LICENSE"),
    )?;
    Ok(())
}

fn write_update_plist(
    app: &Path,
    mut info: plist::Dictionary,
    build_number: u64,
    public_key: String,
) -> Result<()> {
    info.insert("CFBundleVersion".into(), build_number.to_string().into());
    info.insert(
        "SUFeedURL".into(),
        "https://github.com/aiman2039/terminator/releases/latest/download/appcast.xml".into(),
    );
    info.insert("SUPublicEDKey".into(), public_key.into());
    for key in [
        "SUEnableAutomaticChecks",
        "SUAutomaticallyUpdate",
        "SURequireSignedFeed",
        "SUVerifyUpdateBeforeExtraction",
    ] {
        info.insert(key.into(), true.into());
    }
    info.insert("SUShowReleaseNotes".into(), false.into());
    plist::Value::Dictionary(info).to_file_xml(app.join("Contents/Info.plist"))?;
    Ok(())
}

fn new_assembly_output(destination: &Path) -> Result<PathBuf> {
    let target = destination.join("Terminator.app");
    match fs::symlink_metadata(&target) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(target),
        Err(error) => Err(error).context("Cannot inspect assembly output"),
        Ok(_) => anyhow::bail!(
            "Assembly output already exists: {}. Use a fresh output directory outside the Cargo cache",
            target.display()
        ),
    }
}

fn verify_arm64_macho(path: &Path) -> Result<()> {
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        if entry.file_type()?.is_symlink() {
            continue;
        }
        if entry.file_type()?.is_dir() {
            verify_arm64_macho(&entry.path())?;
        } else {
            let mut file = Command::new("file");
            file.arg("-b").arg(entry.path());
            if String::from_utf8(output(file)?)?.contains("Mach-O") {
                let mut verify = Command::new("lipo");
                verify.arg(entry.path()).args(["-verify_arch", "arm64"]);
                output(verify)?;
            }
        }
    }
    Ok(())
}

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
    let tree = resvg::usvg::Tree::from_str(include_str!("../assets/dmg-background.svg"), &options)?;
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
pub fn local_dmg(
    release: bool,
    styled: bool,
    destination: Option<PathBuf>,
    timings: bool,
) -> Result<()> {
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
        let source = staging.path().join("image-source");
        fs::create_dir(&source)?;
        let mut copy = Command::new("ditto");
        copy.arg(&app).arg(source.join("Terminator.app"));
        output(copy)?;
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
            .arg(&image);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restored_empty_app_directory_is_refused_before_reading_inputs() {
        if !cfg!(target_os = "macos") {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("Terminator.app");
        fs::create_dir(&target).unwrap();
        let missing = dir.path().join("missing-input");
        let error = assemble(AssembleInput {
            app: &missing,
            sparkle: &missing,
            build_number: 1,
            destination: dir.path(),
        })
        .unwrap_err();
        assert!(
            error
                .to_string()
                .starts_with("Assembly output already exists:")
        );
        assert!(target.is_dir());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn fresh_assembly_output_leaves_other_packages_intact() {
        let dir = tempfile::tempdir().unwrap();
        let cached = dir.path().join("target/package/Terminator.app");
        fs::create_dir_all(&cached).unwrap();
        fs::write(cached.join("keep"), b"previous package").unwrap();
        let fresh = dir.path().join("runner-temp/package");
        assert_eq!(
            new_assembly_output(&fresh).unwrap(),
            fresh.join("Terminator.app")
        );
        assert_eq!(fs::read(cached.join("keep")).unwrap(), b"previous package");
    }

    #[test]
    fn dangling_app_symlink_is_not_an_empty_destination() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("Terminator.app");
        std::os::unix::fs::symlink(dir.path().join("missing"), &target).unwrap();
        assert!(new_assembly_output(dir.path()).is_err());
        assert!(
            fs::symlink_metadata(target)
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }

    #[test]
    fn sparkle_public_key_must_be_32_byte_ed25519() {
        let key = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string();
        assert_eq!(parse_sparkle_public_key(Some(key.clone())).unwrap(), key);
        assert!(parse_sparkle_public_key(None).is_err());
        assert!(parse_sparkle_public_key(Some("AAAA".into())).is_err());
    }

    #[test]
    fn app_info_requires_terminator_bundle_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let info = dir.path().join("Contents/Info.plist");
        fs::create_dir_all(info.parent().unwrap()).unwrap();
        plist::Value::Dictionary(plist::Dictionary::new())
            .to_file_xml(&info)
            .unwrap();
        assert!(validated_app_info(dir.path()).is_err());
        let mut valid = plist::Dictionary::new();
        valid.insert(
            "CFBundleIdentifier".into(),
            "dev.terminator.app".to_string().into(),
        );
        valid.insert("LSMinimumSystemVersion".into(), "12.0".to_string().into());
        plist::Value::Dictionary(valid).to_file_xml(&info).unwrap();
        assert!(validated_app_info(dir.path()).is_ok());
    }

    #[test]
    fn sparkle_framework_must_be_2_10_0() {
        let dir = tempfile::tempdir().unwrap();
        let info = dir.path().join("Sparkle.framework/Resources/Info.plist");
        fs::create_dir_all(info.parent().unwrap()).unwrap();
        let mut wrong = plist::Dictionary::new();
        wrong.insert(
            "CFBundleShortVersionString".into(),
            "2.9.0".to_string().into(),
        );
        plist::Value::Dictionary(wrong).to_file_xml(&info).unwrap();
        assert!(require_sparkle_version(dir.path()).is_err());
        let mut right = plist::Dictionary::new();
        right.insert(
            "CFBundleShortVersionString".into(),
            "2.10.0".to_string().into(),
        );
        plist::Value::Dictionary(right).to_file_xml(&info).unwrap();
        assert!(require_sparkle_version(dir.path()).is_ok());
    }

    #[test]
    fn assembled_plist_enables_signed_automatic_updates() {
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("Terminator.app");
        fs::create_dir_all(app.join("Contents")).unwrap();
        let mut info = plist::Dictionary::new();
        info.insert("CFBundleIdentifier".into(), "dev.terminator.app".into());
        write_update_plist(&app, info, 25, "public-key".into()).unwrap();
        let written = plist::Value::from_file(app.join("Contents/Info.plist")).unwrap();
        let dict = written.as_dictionary().unwrap();
        assert_eq!(
            dict.get("CFBundleVersion")
                .and_then(plist::Value::as_string),
            Some("25")
        );
        assert_eq!(
            dict.get("SUFeedURL").and_then(plist::Value::as_string),
            Some("https://github.com/aiman2039/terminator/releases/latest/download/appcast.xml")
        );
        assert_eq!(
            dict.get("SUPublicEDKey").and_then(plist::Value::as_string),
            Some("public-key")
        );
        for key in [
            "SUEnableAutomaticChecks",
            "SUAutomaticallyUpdate",
            "SURequireSignedFeed",
            "SUVerifyUpdateBeforeExtraction",
        ] {
            assert_eq!(
                dict.get(key).and_then(plist::Value::as_boolean),
                Some(true),
                "{key}"
            );
        }
        assert_eq!(
            dict.get("SUShowReleaseNotes")
                .and_then(plist::Value::as_boolean),
            Some(false)
        );
    }
}
