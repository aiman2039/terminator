//! New task from an issue or pull request. A fake `gh`/`glab` runs in a
//! temporary repository; no real remote or account is used.
use super::fetch::{Reference, fetch, parse_reference};
use super::*;
use std::path::{Path, PathBuf};
use terminator_core::{CommandOptions, git, run_command};

fn git_run(cwd: &Path, args: &[&str]) {
    let options = CommandOptions {
        timeout: std::time::Duration::from_secs(20),
        ..Default::default()
    };
    run_command(git::command(cwd, args), options).unwrap();
}

fn repo(dir: &Path, host: &str) -> PathBuf {
    let root = dir.join("repo");
    std::fs::create_dir_all(&root).unwrap();
    git_run(&root, &["init", "-q", "-b", "main"]);
    git_run(
        &root,
        &[
            "remote",
            "add",
            "origin",
            &format!("https://{host}/o/r.git"),
        ],
    );
    root
}

/// A fake CLI: `issue view` fails (the number is a pull request), the
/// pull or merge request view prints `view`. Every call is logged.
#[cfg(unix)]
fn fake_cli(dir: &Path, view: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let log = dir.join("calls");
    let script = format!(
        "#!/bin/sh\necho \"$*\" >> '{}'\nif [ \"$1\" = issue ]; then echo 'not an issue' >&2; exit 1; fi\nprintf '%s\\n' '{view}'\n",
        log.display()
    );
    let path = dir.join("fake-cli");
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

#[test]
fn issue_references_accept_numbers_and_urls_only() {
    let issue = |number| Reference { number, pull: None };
    assert_eq!(parse_reference(" 12 "), Some(issue(12)));
    assert_eq!(parse_reference("#7"), Some(issue(7)));
    assert_eq!(
        parse_reference("!3"),
        Some(Reference {
            number: 3,
            pull: Some(true)
        })
    );
    assert_eq!(
        parse_reference("https://github.com/o/r/pull/42"),
        Some(Reference {
            number: 42,
            pull: Some(true)
        })
    );
    assert_eq!(
        parse_reference("https://gitlab.example.com/g/r/-/merge_requests/5"),
        Some(Reference {
            number: 5,
            pull: Some(true)
        })
    );
    assert_eq!(
        parse_reference("https://github.com/o/r/issues/9"),
        Some(Reference {
            number: 9,
            pull: Some(false)
        })
    );
    for bad in [
        "",
        "0",
        "-5",
        "--repo=x",
        "12 13",
        "file:///issues/1",
        "https://x/o/r/tree/1",
    ] {
        assert_eq!(parse_reference(bad), None, "{bad}");
    }
}

#[cfg(unix)]
#[test]
fn a_bare_number_falls_back_to_the_pull_request_and_fills_the_form() {
    let dir = tempfile::tempdir().unwrap();
    let root = repo(dir.path(), "github.com");
    let cli = fake_cli(
        dir.path(),
        r#"{"number":42,"title":"Fix the login page","body":"Steps\nto fix","url":"https://github.com/o/r/pull/42","headRefName":"fix-login"}"#,
    );
    let issue = fetch(&root, "42", Some(&cli)).unwrap();
    let calls = std::fs::read_to_string(dir.path().join("calls")).unwrap();
    assert_eq!(
        calls.lines().collect::<Vec<_>>(),
        [
            "issue view 42 --json number,title,body,url",
            "pr view 42 --json number,title,body,url,headRefName"
        ]
    );
    assert_eq!(issue.head_ref.as_deref(), Some("refs/pull/42/head"));
    let mut draft = WorktreeDraft {
        start: "HEAD".into(),
        open_terminal: true,
        ..Default::default()
    };
    draft.issue.root = Some(root.clone());
    draft.apply_issue(issue, Some(&root));
    assert_eq!(draft.task, "42-fix-the-login-page");
    assert_eq!(draft.branch, "terminator/42-fix-the-login-page");
    assert_eq!(draft.start, "PR #42 head");
    assert_eq!(
        draft.issue.head_to_fetch(&draft.start),
        Some((root, "refs/pull/42/head".to_owned()))
    );
    // A Start from the user changed afterwards is kept: nothing is fetched.
    assert_eq!(draft.issue.head_to_fetch("main"), None);
}

#[cfg(unix)]
#[test]
fn gitlab_merge_requests_use_glab() {
    let dir = tempfile::tempdir().unwrap();
    let root = repo(dir.path(), "gitlab.com");
    let cli = fake_cli(
        dir.path(),
        r#"{"iid":5,"title":"Speed up","description":"Body","web_url":"https://gitlab.com/g/r/-/merge_requests/5","source_branch":"fast"}"#,
    );
    let issue = fetch(&root, "!5", Some(&cli)).unwrap();
    let calls = std::fs::read_to_string(dir.path().join("calls")).unwrap();
    assert_eq!(calls.trim(), "mr view 5 --output json");
    assert_eq!(issue.title, "Speed up");
    assert_eq!(
        issue.head_ref.as_deref(),
        Some("refs/merge-requests/5/head")
    );
}

#[test]
fn create_fetches_the_pull_request_head_without_changing_refs() {
    let dir = tempfile::tempdir().unwrap();
    let origin = dir.path().join("origin");
    std::fs::create_dir_all(&origin).unwrap();
    git_run(&origin, &["init", "-q", "-b", "main"]);
    git_run(&origin, &["config", "user.name", "Fixture"]);
    git_run(
        &origin,
        &["config", "user.email", "fixture@example.invalid"],
    );
    git_run(&origin, &["commit", "-q", "--allow-empty", "-m", "base"]);
    git_run(&origin, &["update-ref", "refs/pull/7/head", "HEAD"]);
    let root = dir.path().join("checkout");
    std::fs::create_dir_all(&root).unwrap();
    git_run(&root, &["init", "-q", "-b", "main"]);
    git_run(
        &root,
        &["remote", "add", "origin", &origin.display().to_string()],
    );
    let hash = fetch_head(&root, "refs/pull/7/head").unwrap();
    assert_eq!(hash.len(), 40);
    let refs = run_command(
        git::command(&root, &["for-each-ref", "--format=%(refname)"]),
        CommandOptions::default(),
    )
    .unwrap();
    assert!(refs.stdout.is_empty(), "the fetch created refs");
    assert!(fetch_head(&root, "--upload-pack=x").is_err());
}

fn fetched_issue() -> Issue {
    Issue {
        number: 12,
        title: "Crash on start".into(),
        body: "It's broken\nsee log".into(),
        url: "https://github.com/o/r/issues/12".into(),
        head_ref: None,
    }
}

#[test]
fn issue_text_is_typed_only_when_ticked_and_never_with_enter() {
    let mut draft = WorktreeDraft {
        start: "HEAD".into(),
        open_terminal: true,
        agent_command: "claude".into(),
        ..Default::default()
    };
    draft.apply_issue(fetched_issue(), None);
    assert_eq!(draft.start, "HEAD");
    assert_eq!(draft.issue.head_to_fetch("HEAD"), None);
    assert_eq!(draft.task_start().typed_input(), "claude");
    draft.issue.type_prompt = true;
    let typed = draft.task_start().typed_input();
    assert_eq!(
        typed,
        r"claude 'Crash on start (#12): It'\''s broken see log'"
    );
    assert!(!typed.contains('\n') && !typed.contains('\r'));
    // Without an agent command there is nothing to type it after.
    draft.agent_command.clear();
    assert_eq!(draft.task_start().typed_input(), "");
}
