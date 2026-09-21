//! Unix access revocation in a disposable project. This is not a TCC policy test.
use super::{Duration, Harness, Options, PathBuf, Result, capture, ensure, fs, json, thread};
use std::os::unix::fs::PermissionsExt;

struct RestoreAccess(PathBuf);
impl Drop for RestoreAccess {
    fn drop(&mut self) {
        let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o755));
    }
}
pub fn run(o: &Options) -> Result<()> {
    let h = Harness::new()?;
    h.setup()?;
    let project = h.project("folder-recovery")?;
    let path = PathBuf::from(project["path"].as_str().unwrap());
    fs::write(path.join("retained.rs"), "// Public fixture\n")?;
    let shell = h.shell(&project)?;
    h.layout(&project, std::slice::from_ref(&shell))?;
    let _restore = RestoreAccess(path.clone());
    let logs = capture(
        &h,
        o,
        "recovered",
        json!([{"at_ms":5500,"target":"directory-retry"}]),
        7000,
        |_| {
            let deadline = std::time::Instant::now() + Duration::from_secs(4);
            while !fs::read_to_string(h.root.join("recovered.log"))
                .unwrap_or_default()
                .contains("Directory refresh succeeded: entries=1")
            {
                ensure!(
                    std::time::Instant::now() < deadline,
                    "Initial listing did not load"
                );
                thread::sleep(Duration::from_millis(50));
            }
            fs::set_permissions(&path, fs::Permissions::from_mode(0o000))?;
            thread::sleep(Duration::from_millis(1100));
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755))?;
            Ok(())
        },
    )?;
    ensure!(
        logs.contains("Directory refresh failed: kind=PermissionDenied cached_entries=1"),
        "Denied read did not preserve cached content: {logs}"
    );
    ensure!(
        fs::read_to_string(path.join("retained.rs"))? == "// Public fixture\n",
        "Recovery changed project content"
    );
    fs::write(o.output.join("access-evidence.log"), logs)?;
    h.assert_pids(&[shell])?;
    Ok(())
}
