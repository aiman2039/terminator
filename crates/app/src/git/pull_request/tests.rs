//! The PR tab reads `gh`/`glab` output only. These tests run a fake CLI in
//! a temporary repository; no real remote or account is used.
use super::fetch::{PrState, fetch, parse_gh_threads, parse_gh_view, parse_glab_view};
use super::view::checks_text;
use std::path::{Path, PathBuf};
use terminator_core::{CommandOptions, git, run_command};

const VIEW: &str = r#"{"number":42,"title":"Fix login","state":"OPEN","url":"https://github.com/o/r/pull/42","isDraft":false,"reviewDecision":"CHANGES_REQUESTED","statusCheckRollup":[{"__typename":"CheckRun","name":"build","status":"COMPLETED","conclusion":"SUCCESS"},{"__typename":"CheckRun","name":"test","status":"COMPLETED","conclusion":"FAILURE"},{"__typename":"CheckRun","name":"lint","status":"IN_PROGRESS","conclusion":""},{"__typename":"StatusContext","context":"ci/legacy","state":"SUCCESS"}]}"#;

const THREADS: &str = r#"{"data":{"repository":{"pullRequest":{"reviewThreads":{"nodes":[{"isResolved":false,"path":"src/a.rs","line":12,"comments":{"nodes":[{"author":{"login":"ana"},"body":"Rename this\nplease"}]}},{"isResolved":true,"path":"src/b.rs","line":null,"comments":{"nodes":[{"author":{"login":"bo"},"body":"ok"}]}}]}}}}}"#;

fn git_run(cwd: &Path, args: &[&str]) {
    let options = CommandOptions {
        timeout: std::time::Duration::from_secs(20),
        ..Default::default()
    };
    run_command(git::command(cwd, args), options).unwrap();
}

/// A repository whose `origin` is on `host`.
fn repo(dir: &Path, host: &str) -> PathBuf {
    let root = dir.join("repo");
    std::fs::create_dir_all(&root).unwrap();
    git_run(&root, &["init", "-q", "-b", "main"]);
    let url = format!("https://{host}/o/r.git");
    git_run(&root, &["remote", "add", "origin", &url]);
    root
}

/// A fake CLI that prints `view` for `pr view` / `mr view` and `threads`
/// for `api`, or fails with `stderr`.
#[cfg(unix)]
fn fake_cli(dir: &Path, view: &str, threads: &str, stderr: Option<&str>) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let script = match stderr {
        Some(stderr) => format!("#!/bin/sh\necho '{stderr}' >&2\nexit 1\n"),
        None => format!(
            "#!/bin/sh\nif [ \"$1\" = api ]; then\ncat <<'EOF'\n{threads}\nEOF\nelse\ncat <<'EOF'\n{view}\nEOF\nfi\n"
        ),
    };
    let path = dir.join("fake-cli");
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

#[cfg(unix)]
#[test]
fn github_pull_request_shows_checks_and_review_threads() {
    let dir = tempfile::tempdir().unwrap();
    let root = repo(dir.path(), "github.com");
    let cli = fake_cli(dir.path(), VIEW, THREADS, None);
    let PrState::Ready(info) = fetch(&root, Some(&cli)) else {
        panic!("expected a pull request");
    };
    assert_eq!((info.number, info.title.as_str()), (42, "Fix login"));
    assert_eq!(info.review.as_deref(), Some("CHANGES_REQUESTED"));
    assert_eq!(
        checks_text(&info.checks).as_deref(),
        Some("Checks: 2 passed, 1 failed, 1 pending")
    );
    assert_eq!(info.checks.failing, vec!["test".to_owned()]);
    assert_eq!(info.threads.len(), 2);
    assert_eq!(info.threads[0].body, "Rename this");
    assert_eq!(info.threads[0].line, Some(12));
    assert!(info.threads[1].resolved);
}

#[cfg(unix)]
#[test]
fn a_branch_without_a_pull_request_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let root = repo(dir.path(), "github.com");
    let stderr = "no pull requests found for branch \"main\"";
    let cli = fake_cli(dir.path(), "", "", Some(stderr));
    assert_eq!(fetch(&root, Some(&cli)), PrState::NoPr);
}

#[cfg(unix)]
#[test]
fn a_cli_without_a_login_gives_a_quiet_hint() {
    let dir = tempfile::tempdir().unwrap();
    let root = repo(dir.path(), "github.com");
    let stderr = "To get started with GitHub CLI, please run:  gh auth login";
    let cli = fake_cli(dir.path(), "", "", Some(stderr));
    assert!(
        matches!(fetch(&root, Some(&cli)), PrState::Hint(hint) if hint.contains("gh auth login"))
    );
}

#[test]
fn a_missing_cli_gives_a_quiet_hint() {
    let dir = tempfile::tempdir().unwrap();
    let root = repo(dir.path(), "gitlab.example.com");
    let missing = dir.path().join("no-such-glab");
    assert!(matches!(fetch(&root, Some(&missing)), PrState::Hint(hint) if hint.contains("glab")));
}

#[test]
fn gitlab_merge_request_reads_its_pipeline() {
    let view: serde_json::Value = serde_json::from_str(
        r#"{"iid":7,"title":"MR","state":"opened","web_url":"https://gitlab.com/o/r/-/merge_requests/7","draft":true,"head_pipeline":{"status":"failed"}}"#,
    )
    .unwrap();
    let info = parse_glab_view(&view);
    assert_eq!(info.number, 7);
    assert!(info.draft);
    assert_eq!(info.checks.failed, 1);
}

#[test]
fn a_view_with_no_checks_has_no_summary() {
    let info = parse_gh_view(&serde_json::json!({"number": 1, "statusCheckRollup": []}));
    assert_eq!(checks_text(&info.checks), None);
    assert!(parse_gh_threads(&serde_json::json!({})).is_empty());
}

#[test]
fn a_known_pull_request_comes_only_from_reads_already_made_for_its_root_and_branch() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = eframe::egui::Context::default();
    let mut app = crate::App::with_context(&ctx, terminator_core::Paths::at(dir.path().into()));
    let (wt, other) = (Path::new("/w/fix"), Path::new("/w/other"));
    assert_eq!(app.known_pr(wt, Some("fix")), None, "nothing read yet");
    app.metadata = Some(terminator_core::metadata::Metadata {
        root: Some(wt.into()),
        branch: Some("fix".into()),
        pull_request: Some(terminator_core::metadata::PullRequest {
            number: 7,
            title: "Fix".into(),
            url: "https://example.com/7".into(),
            state: "OPEN".into(),
        }),
        ..Default::default()
    });
    assert_eq!(app.known_pr(wt, Some("fix")).map(|pr| pr.0), Some(7));
    assert_eq!(app.known_pr(wt, Some("main")), None, "another branch");
    assert_eq!(app.known_pr(other, Some("fix")), None, "another worktree");
    // The PR tab's answer for its root and branch wins.
    let panel = &mut app.git_extras.pull_request;
    panel.key = Some((other.into(), "topic".into()));
    panel.state = Some(PrState::Ready(super::fetch::PrInfo {
        number: 9,
        ..Default::default()
    }));
    assert_eq!(app.known_pr(other, Some("topic")).map(|pr| pr.0), Some(9));
}
