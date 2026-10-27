#[cfg(unix)]
use super::*;
#[cfg(unix)]
#[test]
fn missing_non_executable_and_directory_helpers_are_unavailable() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("helper");
    assert!(!executable_available(&file));
    assert!(!executable_available(dir.path()));
    fs::write(&file, "#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(!executable_available(&file));
    fs::set_permissions(&file, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(executable_available(&file));
    fs::remove_file(&file).unwrap();
    assert!(!executable_available(&file));
}
