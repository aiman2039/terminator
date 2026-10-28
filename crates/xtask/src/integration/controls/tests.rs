use super::*;
use std::process::Command;

#[test]
fn readiness_markers_require_execution_and_cannot_match_input_echo() {
    for (command, marker) in [
        (CONTROL_SIGNAL, "CONTROL_SEND_PROOF"),
        (CROSS_PROJECT_SIGNAL, "CROSS_PROJECT_READY"),
        (CHILD_SIGNAL, "CHILD_READY"),
        (CHILD_STOPPED_SIGNAL, "CHILD_STOPPED"),
    ] {
        assert!(!command.contains(marker), "Input echo could match {marker}");
        let output = Command::new("sh").args(["-c", command]).output().unwrap();
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            format!("{marker}\n")
        );
    }
}

#[test]
fn failed_directory_change_does_not_emit_readiness() {
    let dir = tempfile::tempdir().unwrap();
    let command = format!(
        "cd {} && {CROSS_PROJECT_SIGNAL}",
        shell_quote(&dir.path().join("missing").to_string_lossy())
    );
    let output = Command::new("sh").args(["-c", &command]).output().unwrap();
    assert!(!output.status.success());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("CROSS_PROJECT_READY"));
}
