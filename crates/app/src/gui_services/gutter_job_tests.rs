//! The gutter's `HEAD` read also names the file's repository root: its
//! Stage action matches that root with a running commit. (Unix only: the
//! test reaches the file through a symlink.)
#![cfg(unix)]
use super::submit::Services;
use crate::{Job, Update};
use eframe::egui;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

fn git(root: &Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

/// The next `GitBase` reply: the `HEAD` text and the repository root.
fn base_reply(owner: &mut super::Owner) -> (Option<String>, Option<PathBuf>) {
    let started = Instant::now();
    loop {
        if let Some(mut completion) = owner.supervisor.try_recv()
            && let Some(Ok(updates)) = completion.result.take()
        {
            for update in updates {
                if let Update::GitBase(_, 7, base, root) = update {
                    return (base, root);
                }
            }
        }
        assert!(started.elapsed() < Duration::from_secs(20), "no reply");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn the_gutter_head_read_names_the_canonical_repository_root() {
    let dir = tempfile::tempdir().unwrap();
    let (repo, hooks) = (dir.path().join("repo"), dir.path().join("hooks"));
    std::fs::create_dir(&repo).unwrap();
    std::fs::create_dir(&hooks).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.email", "gutter@example.invalid"]);
    git(&repo, &["config", "user.name", "Gutter"]);
    git(&repo, &["config", "commit.gpgsign", "false"]);
    // An empty hooks folder: local config wins over a global one.
    let hooks = hooks.to_string_lossy();
    git(&repo, &["config", "core.hooksPath", &hooks]);
    std::fs::write(repo.join("a.txt"), "base\n").unwrap();
    git(&repo, &["add", "a.txt"]);
    git(&repo, &["commit", "-qm", "base"]);
    // An Explorer path can reach the file through a symlink.
    let link = dir.path().join("link");
    std::os::unix::fs::symlink(&repo, &link).unwrap();
    let (updates, _) = std::sync::mpsc::channel();
    let (service, mut owner) = Services::new(
        terminator_core::Paths::at(dir.path().join("state")),
        egui::Context::default(),
        updates,
    )
    .unwrap();
    service.send(Job::GitBase(link.join("a.txt"), 7)).unwrap();
    let (base, root) = base_reply(&mut owner);
    assert_eq!(base.as_deref(), Some("base\n"));
    assert_eq!(root, Some(repo.canonicalize().unwrap()));
}
