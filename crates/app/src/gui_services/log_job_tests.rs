//! Commit Log query jobs: their queue, their busy reply, the local Git
//! they run (never a daemon request), and the Git a replaced query stops.
use super::jobs::{JobGit, JobRouting};
use super::submit::Services;
use crate::git_log::{LogFilter, LogGit, LogQuery};
use crate::{GitView, Job, Tab, Update};
use eframe::egui;
use std::path::Path;
use std::time::{Duration, Instant};
use terminator_core::CommandOptions;
use terminator_core::async_service::{CancellationToken, Policy};

fn query(text: &str, generation: u64) -> LogQuery {
    LogQuery {
        filter: LogFilter {
            text: text.into(),
            ..LogFilter::default()
        },
        goto: None,
        generation,
    }
}

#[test]
fn log_queries_use_a_local_replaceable_queue_not_the_daemon() {
    let tab = Tab::CommitLog {
        cwd: "/repo".into(),
    };
    let job = Job::GitLog(tab.clone(), query("fix", 42));
    let context = JobRouting::context(&job);
    assert_eq!(context.subsystem, crate::git_log::LOG_SUBSYSTEM);
    assert_eq!(context.resource, tab.key());
    assert_eq!(context.policy, Policy::ReplaceableRead);
    assert_eq!(context.generation, 42, "replies match their query");
    // A refused query still answers its tab, so the bar stops its spinner.
    match JobRouting::rejection(&job).as_slice() {
        [Update::GitView(key, GitView::Log(log))] => {
            assert_eq!(key, &tab.key());
            assert_eq!(log.query.generation, 42);
            assert!(log.error.is_some());
        }
        _ => panic!("a refused query answers its Commit Log tab"),
    }
}

fn git(root: &Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

fn service(dir: &Path) -> (Services, super::Owner) {
    let (updates, _) = std::sync::mpsc::channel();
    Services::new(
        terminator_core::Paths::at(dir.join("state")),
        egui::Context::default(),
        updates,
    )
    .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_log_job_runs_the_filtered_query_with_local_git() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.email", "log@example.invalid"]);
    git(&repo, &["config", "user.name", "Log"]);
    git(&repo, &["config", "commit.gpgsign", "false"]);
    for subject in ["first", "fix the parser", "last"] {
        git(&repo, &["commit", "-q", "--allow-empty", "-m", subject]);
    }
    let (service, _owner) = service(dir.path());
    let tab = Tab::CommitLog { cwd: repo.clone() };
    let mut updates = Vec::new();
    let request = (tab.clone(), query("PARSER", 9));
    service
        .git_log(request, CancellationToken::new(), &mut updates)
        .await
        .unwrap();
    match updates.as_slice() {
        [Update::GitView(key, GitView::Log(log))] => {
            assert_eq!(key, &tab.key());
            assert_eq!(log.error, None);
            assert_eq!(log.query.generation, 9);
            let subjects: Vec<&str> = log.commits.iter().map(|c| c.subject.as_str()).collect();
            assert_eq!(subjects, ["fix the parser"]);
        }
        _ => panic!("one Commit Log reply"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cancelled_log_query_stops_its_git_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let (service, _owner) = service(dir.path());
    let cancel = CancellationToken::new();
    let runner = JobGit::new(&service, &cancel);
    let root = dir.path().to_path_buf();
    let started = Instant::now();
    // A Git alias that would run for 30 seconds.
    let slow = std::thread::spawn(move || {
        let options = CommandOptions {
            timeout: Duration::from_mins(1),
            ..CommandOptions::default()
        };
        runner.git(&root, &["-c", "alias.slow=!sleep 30", "slow"], options)
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    cancel.cancel();
    // The other runtime workers keep running while this one waits.
    let result = slow.join().unwrap();
    assert_eq!(result, Err("Operation cancelled".to_owned()));
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "Git was stopped"
    );
}
