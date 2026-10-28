use super::*;

#[test]
fn relaunch_waits_for_delayed_process_output() {
    let root = tempfile::tempdir().unwrap();
    let marker = root.path().join("relaunched");
    let writer = thread::spawn(move || {
        thread::sleep(Duration::from_millis(100));
        fs::write(marker, "data=isolated\nruntime=isolated/run\nsession=\n").unwrap();
    });
    let result = wait_relaunch_marker(root.path(), Duration::from_secs(5));
    writer.join().unwrap();
    assert_eq!(
        result.unwrap(),
        "data=isolated\nruntime=isolated/run\nsession=\n"
    );
}

#[test]
fn relaunch_rejects_missing_or_incomplete_output_and_reports_child_log() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("restart.log"), "stub could not start").unwrap();
    for output in [None, Some("data=isolated\n")] {
        if let Some(output) = output {
            fs::write(root.path().join("relaunched"), output).unwrap();
        }
        let error = wait_relaunch_marker(root.path(), Duration::ZERO).unwrap_err();
        assert!(error.to_string().contains("Relaunch stub did not complete"));
        assert!(error.to_string().contains("stub could not start"));
    }
}
