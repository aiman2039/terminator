//! Package layout and archive contents from a fake staged tree; no real
//! release, install or publish.
use super::*;
use std::io::Read;

fn staged(dir: &Path) -> PathBuf {
    let app = dir.join("terminator");
    std::fs::create_dir_all(app.join("radio")).unwrap();
    for name in ["terminator", "terminator-daemon", "terminator-hook"] {
        std::fs::write(app.join(name), format!("#!/bin/sh\necho {name}\n")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(app.join(name), std::fs::Permissions::from_mode(0o755))
                .unwrap();
        }
    }
    std::fs::write(app.join("terminator.png"), b"png").unwrap();
    std::fs::write(app.join("README.md"), b"readme").unwrap();
    std::fs::write(app.join("radio/stations.json"), b"[]").unwrap();
    app
}

fn meta() -> Meta {
    Meta {
        version: "1.2.3".into(),
        arch: "x86_64".into(),
    }
}

#[test]
fn package_layout_keeps_the_three_binaries_side_by_side() {
    let dir = tempfile::tempdir().unwrap();
    let entries = layout(&staged(dir.path())).unwrap();
    let paths: Vec<&str> = entries.iter().map(Entry::path).collect();
    for path in [
        "/usr/lib/terminator-workspace/terminator",
        "/usr/lib/terminator-workspace/terminator-daemon",
        "/usr/lib/terminator-workspace/terminator-hook",
        "/usr/lib/terminator-workspace/radio/stations.json",
        "/usr/bin/terminator-workspace",
        "/usr/bin/terminator-workspace-hook",
        "/usr/share/applications/terminator-workspace.desktop",
        "/usr/share/icons/hicolor/512x512/apps/terminator-workspace.png",
    ] {
        assert!(paths.contains(&path), "{path} missing from {paths:?}");
    }
    // The emulator package owns /usr/bin/terminator; this one never does.
    assert!(!paths.contains(&"/usr/bin/terminator"));
    #[cfg(unix)]
    assert!(entries.iter().any(|entry| matches!(
        entry,
        Entry::File { path, mode: 0o755, .. } if path.ends_with("/terminator-daemon")
    )));
    assert!(desktop_entry().contains("Exec=/usr/lib/terminator-workspace/terminator\n"));
}

#[test]
fn package_layout_refuses_a_tree_without_the_daemon() {
    let dir = tempfile::tempdir().unwrap();
    let app = staged(dir.path());
    std::fs::remove_file(app.join("terminator-daemon")).unwrap();
    assert!(layout(&app).is_err());
}

fn untar(bytes: &[u8]) -> Vec<(String, u32, Vec<u8>)> {
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(bytes));
    archive
        .entries()
        .unwrap()
        .map(|entry| {
            let mut entry = entry.unwrap();
            // The tar crate stores `./usr/...` as `usr/...`; dpkg takes both.
            let path = entry.path().unwrap().to_string_lossy().into_owned();
            let path = path.trim_start_matches("./").to_owned();
            let mode = entry.header().mode().unwrap();
            let mut data = Vec::new();
            entry.read_to_end(&mut data).unwrap();
            (path, mode, data)
        })
        .collect()
}

#[test]
fn deb_holds_control_and_data_in_dpkg_order() {
    let dir = tempfile::tempdir().unwrap();
    let entries = layout(&staged(dir.path())).unwrap();
    let output = dir.path().join(deb::file_name(&meta()));
    assert!(output.ends_with("terminator-workspace_1.2.3_amd64.deb"));
    deb::write_deb(&entries, &meta(), &output).unwrap();
    let mut archive = ar::Archive::new(std::fs::File::open(&output).unwrap());
    let mut members = Vec::new();
    while let Some(member) = archive.next_entry() {
        let mut member = member.unwrap();
        let name = String::from_utf8(member.header().identifier().to_vec()).unwrap();
        let mut data = Vec::new();
        member.read_to_end(&mut data).unwrap();
        members.push((name, data));
    }
    let names: Vec<&str> = members.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, ["debian-binary", "control.tar.gz", "data.tar.gz"]);
    assert_eq!(members[0].1, b"2.0\n");
    let control = untar(&members[1].1);
    let text =
        String::from_utf8(control.iter().find(|e| e.0 == "control").unwrap().2.clone()).unwrap();
    assert!(
        text.starts_with("Package: terminator-workspace\nVersion: 1.2.3\nArchitecture: amd64\n")
    );
    let data = untar(&members[2].1);
    let hook = data
        .iter()
        .find(|e| e.0 == "usr/lib/terminator-workspace/terminator-hook")
        .unwrap();
    assert_eq!(hook.1 & 0o777, 0o755);
    // Parent folders come before their files.
    let position = |name: &str| data.iter().position(|e| e.0 == name).unwrap();
    assert!(
        position("usr/lib/terminator-workspace/")
            < position("usr/lib/terminator-workspace/terminator")
    );
    assert!(data.iter().any(|e| e.0 == "usr/bin/terminator-workspace"));
    assert!(
        data.iter()
            .all(|e| !e.0.is_empty() && !e.0.starts_with('/'))
    );
}

#[test]
fn rpm_lists_the_same_files() {
    let dir = tempfile::tempdir().unwrap();
    let entries = layout(&staged(dir.path())).unwrap();
    let output = dir.path().join(rpm_package::file_name(&meta()));
    assert!(output.ends_with("terminator-workspace-1.2.3-1.x86_64.rpm"));
    rpm_package::write_rpm(&entries, &meta(), &output).unwrap();
    let package = rpm::Package::open(&output).unwrap();
    assert_eq!(package.metadata.get_name().unwrap(), "terminator-workspace");
    let files: Vec<String> = package
        .metadata
        .get_file_paths()
        .unwrap()
        .iter()
        .map(|path| path.display().to_string())
        .collect();
    let mut expected: Vec<String> = entries.iter().map(|e| e.path().to_owned()).collect();
    let mut found = files.clone();
    expected.sort();
    found.sort();
    assert_eq!(found, expected);
}
