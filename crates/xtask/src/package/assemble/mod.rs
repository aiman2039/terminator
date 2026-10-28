#[cfg(test)]
mod tests;

use super::{Context, Path, PathBuf, Result, copy_tree, ensure, fs};
use crate::harness::output;
use std::process::Command;

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
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    for name in ["terminator", "terminator-daemon", "terminator-hook"] {
        let path = app.join("Contents/MacOS").join(name);
        let mut verify = Command::new("lipo");
        verify.arg(&path).args(["-verify_arch", "arm64"]);
        output(verify)?;
        #[cfg(unix)]
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
