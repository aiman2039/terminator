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
#[cfg(unix)]
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
    write_update_plist(WriteUpdatePlistInput {
        app: &app,
        info,
        build_number: 25,
        public_key: "public-key".into(),
    })
    .unwrap();
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
