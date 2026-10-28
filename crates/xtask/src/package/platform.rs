use super::{copy_tree, licenses};
use crate::harness::{output, root};
use anyhow::{Result, ensure};
use std::{fs, path::Path, process::Command};

pub(super) fn macos(app: &Path, destination: &Path) -> Result<()> {
    write_info_plist(app)?;
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
    sign.args(["--force", "--deep", "--sign", "-"]).arg(app);
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
    if let Err(error) = fs::rename(app, &target) {
        if backup.exists() {
            fs::rename(&backup, &target)?;
        }
        return Err(error.into());
    }
    if backup.exists() {
        fs::remove_dir_all(backup)?;
    }
    println!("{}", target.display());
    Ok(())
}

fn write_info_plist(app: &Path) -> Result<()> {
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
    Ok(())
}

pub(super) fn windows(app: &Path, staging: &Path, destination: &Path) -> Result<()> {
    // Portable install: unzip anywhere and run terminator.exe. The zip
    // holds the same tree as the Linux tarball (binaries, README, radio,
    // licenses, icon). A signed MSIX can replace this once the release
    // flow has signing; the updater only needs the asset name below.
    fs::copy(
        root().join("crates/app/assets/branding/terminator-512.png"),
        app.join("terminator.png"),
    )?;
    fs::copy(root().join("README.md"), app.join("README.md"))?;
    copy_tree(&root().join("crates/app/assets/radio"), &app.join("radio"))?;
    licenses(&app.join("licenses"))?;
    let mut entries = Vec::new();
    crate::zip_writer::append_tree(&mut entries, "terminator", app)?;
    let staged = staging.join("terminator-windows.zip");
    crate::zip_writer::finish(&staged, &mut entries)?;
    let target = destination.join("terminator-windows.zip");
    fs::rename(&staged, &target)?;
    println!("{}", target.display());
    Ok(())
}

pub(super) fn linux(app: &Path, staging: &Path, destination: &Path) -> Result<()> {
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
    let file = fs::File::create(staging.join("terminator-linux.tar.gz"))?;
    let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
    let mut tar = tar::Builder::new(encoder);
    tar.append_dir_all("terminator", app)?;
    tar.into_inner()?.finish()?;
    let target = destination.join("terminator-linux.tar.gz");
    fs::rename(staging.join("terminator-linux.tar.gz"), &target)?;
    println!("{}", target.display());
    Ok(())
}
