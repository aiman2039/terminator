//! Log filters: `git log` arguments, the typing pause, replies, Go to
//! Hash, graph lanes under filters, and filtered loads from a real Git.
use super::*;
use std::process::Command;
use std::time::{Duration, Instant};

fn args(filter: &LogFilter) -> Vec<String> {
    filter.log_args().unwrap()
}

/// The arguments before `--`: options and revisions only.
fn options(args: &[String]) -> &[String] {
    let end = args.iter().position(|arg| arg == "--").unwrap();
    &args[..end]
}

#[test]
fn each_filter_maps_to_its_git_log_argument() {
    let all = args(&LogFilter::default());
    assert_eq!(&all[..3], ["log", "--exclude=refs/stash", "--all"]);
    for want in ["--date-order", "--decorate=full", "--max-count=1000"] {
        assert!(all.iter().any(|arg| arg == want), "{want}: {all:?}");
    }
    assert_eq!(all.last().map(String::as_str), Some("--"));
    assert!(!all.iter().any(|arg| arg.starts_with("--grep")
        || arg.starts_with("--author")
        || arg.starts_with("--since")
        || arg == "--parents"));

    let text = args(&LogFilter {
        text: "  fix(parser) [x]+ ".into(),
        ..LogFilter::default()
    });
    assert!(text.contains(&"--regexp-ignore-case".to_owned()));
    assert!(text.contains(&"--extended-regexp".to_owned()));
    assert!(text.contains(&r"--grep=fix\(parser\) \[x\]\+".to_owned()));

    let head = args(&LogFilter {
        branch: LogBranch::Head,
        ..LogFilter::default()
    });
    assert_eq!(&head[..2], ["log", "HEAD"]);
    assert!(!head.contains(&"--all".to_owned()));
    let branch = args(&LogFilter {
        branch: LogBranch::Ref("refs/remotes/origin/main".into()),
        ..LogFilter::default()
    });
    assert_eq!(&branch[..2], ["log", "refs/remotes/origin/main"]);

    let user = args(&LogFilter {
        author: Some("Ada L.".into()),
        ..LogFilter::default()
    });
    assert!(
        user.contains(&r"--author=^Ada L\. <".to_owned()),
        "{user:?}"
    );
    assert!(options(&user).contains(&"--no-use-mailmap".to_owned()));

    let week = args(&LogFilter {
        date: LogDate::Week,
        ..LogFilter::default()
    });
    assert!(week.contains(&"--since=7.days.ago".to_owned()));

    let path = args(&LogFilter {
        path: " src/lib.rs ".into(),
        ..LogFilter::default()
    });
    assert!(options(&path).contains(&"--parents".to_owned()));
    assert!(path.ends_with(&["--".to_owned(), "src/lib.rs".to_owned()]));
}

#[test]
fn option_like_values_stay_values() {
    let filter = LogFilter {
        text: "--output=/tmp/x".into(),
        author: Some("--all".into()),
        path: "-rf".into(),
        branch: LogBranch::Ref("refs/heads/-x".into()),
        ..LogFilter::default()
    };
    let args = args(&filter);
    let before = options(&args);
    // Each typed value sits inside its own `--name=` argument or after `--`.
    assert!(before.contains(&"--grep=--output=/tmp/x".to_owned()));
    assert!(before.contains(&"--author=^--all <".to_owned()));
    assert!(before.contains(&"refs/heads/-x".to_owned()));
    assert!(
        !before
            .iter()
            .any(|arg| arg == "--output=/tmp/x" || arg == "--all")
    );
    assert!(!before.iter().any(|arg| arg == "-rf"));
    assert_eq!(args.last().map(String::as_str), Some("-rf"));
    // A branch that is not a full branch ref name never reaches Git.
    for name in ["--all", "main", "refs/tags/v1", "refs/heads/a b"] {
        let filter = LogFilter {
            branch: LogBranch::Ref(name.into()),
            ..LogFilter::default()
        };
        assert!(filter.log_args().is_err(), "{name}");
    }
}

#[test]
fn text_is_a_hash_only_with_seven_to_64_hex_digits() {
    let text = |text: &str| LogFilter {
        text: text.into(),
        ..LogFilter::default()
    };
    assert_eq!(text(" ABCDEF1 ").hash_text().as_deref(), Some("abcdef1"));
    assert_eq!(text(&"a".repeat(64)).hash_text(), Some("a".repeat(64)));
    let long = "a".repeat(65);
    for no in ["abcdef", "decaf", "abcdefg", "", long.as_str()] {
        assert_eq!(text(no).hash_text(), None, "{no}");
    }
    assert!(text("  ").is_empty());
    assert!(!text("x").is_empty());
    assert!(text("x").hides_commits());
    let date = LogFilter {
        date: LogDate::Day,
        ..LogFilter::default()
    };
    assert!(
        !date.hides_commits(),
        "older history goes on below the list"
    );
}

#[test]
fn typing_waits_for_a_pause_and_each_change_loads_once() {
    let start = Instant::now();
    let at = |ms: u64| start.checked_add(Duration::from_millis(ms)).unwrap();
    let mut view = LogView::default();
    let first = view.reload();
    assert!(view.loading && first.filter.is_empty());
    view.draft.text = "fi".into();
    view.typed(at(0));
    assert_eq!(view.due(at(0)), None);
    assert_eq!(view.wait(at(100)), Some(Duration::from_millis(300)));
    view.draft.text = "fix".into();
    view.typed(at(300));
    assert_eq!(view.due(at(600)), None, "each keystroke restarts the pause");
    let typed = view.due(at(700)).expect("the pause is over");
    assert_eq!(typed.filter.text, "fix");
    assert!(typed.generation > first.generation);
    assert_eq!(view.due(at(800)), None, "a change loads once");
    assert_eq!(view.wait(at(800)), None);
    // Enter and menu picks load at once.
    view.draft.path = "src".into();
    view.typed(at(900));
    view.apply();
    assert_eq!(view.due(at(900)).map(|q| q.filter.path), Some("src".into()));
    view.draft.date = LogDate::Month;
    view.apply();
    assert!(view.due(at(900)).is_some());
    view.clear();
    assert!(
        view.due(at(900))
            .is_some_and(|query| query.filter.is_empty())
    );
}

fn reply(query: &LogQuery) -> CommitLog {
    CommitLog {
        query: Box::new(query.clone()),
        ..CommitLog::default()
    }
}

#[test]
fn only_the_newest_reply_shows_and_a_failed_filter_keeps_the_shown_log() {
    let mut view = LogView::default();
    let old = view.reload();
    view.draft.author = Some("Ada".into());
    view.apply();
    let new = view.due(Instant::now()).unwrap();
    assert_eq!(view.answer(&reply(&old), true), Answer::Stale);
    assert!(view.loading, "the newest query still runs");
    assert!(!view.ended(old.generation, None), "a replaced job ended");
    let failed = new.failed("git log: bad".into());
    assert_eq!(view.answer(&failed, true), Answer::KeepShown);
    assert_eq!(view.error.as_deref(), Some("git log: bad"));
    assert!(!view.loading);
    // With no log on screen, the error shows in its place.
    let retry = view.reload();
    assert!(view.error.is_none());
    assert_eq!(
        view.answer(&retry.failed("Git root: no".into()), false),
        Answer::Show(None)
    );
    let last = view.reload();
    assert!(view.ended(last.generation, Some("Log query stopped".into())));
    assert_eq!(view.error.as_deref(), Some("Log query stopped"));
    assert!(!view.loading);
}

#[test]
fn go_to_hash_reveals_the_found_commit_or_notes_the_miss() {
    let mut view = LogView::default();
    let query = view.find("abc1234".into());
    assert_eq!(query.goto.as_deref(), Some("abc1234"));
    assert_eq!(view.finding.as_deref(), Some("abc1234"));
    let hash = format!("{}abcde", "abc1234".repeat(5));
    let found = CommitLog {
        found: Some(hash.clone()),
        ..reply(&query)
    };
    assert_eq!(view.answer(&found, true), Answer::Show(Some(hash.clone())));
    assert_eq!(view.reveal.as_deref(), Some(hash.as_str()));
    assert!(view.finding.is_none() && view.goto_note.is_none());
    let query = view.find("nope".into());
    let missed = CommitLog {
        missing: Some("No commit matches 'nope'".into()),
        ..reply(&query)
    };
    assert_eq!(view.answer(&missed, true), Answer::Show(None));
    assert_eq!(view.goto_note.as_deref(), Some("No commit matches 'nope'"));
}

#[test]
fn a_newer_go_to_hash_of_a_loaded_commit_wins_over_one_that_still_loads() {
    let mut view = LogView::default();
    let older = view.find("abc1234".into());
    // Go to Hash then names a commit that is in the list.
    view.picked("b2");
    assert!(view.finding.is_none() && view.loading);
    assert_eq!(view.reveal.take().as_deref(), Some("b2"));
    let found = CommitLog {
        found: Some("abc1234".repeat(6)),
        ..reply(&older)
    };
    // The reply still shows the log, but it does not select its commit.
    assert_eq!(view.answer(&found, true), Answer::Show(None));
    assert!(view.reveal.is_none() && !view.loading);
    let older = view.find("nope".into());
    view.picked("b2");
    let missed = CommitLog {
        missing: Some("No commit matches 'nope'".into()),
        ..reply(&older)
    };
    assert_eq!(view.answer(&missed, true), Answer::Show(None));
    assert!(
        view.goto_note.is_none(),
        "no note for a search the user left"
    );
}

fn entry(hash: &str, author: &str, parents: &[&str], refs: &[&str]) -> CommitEntry {
    CommitEntry {
        hash: hash.into(),
        short_hash: hash.chars().take(7).collect(),
        author: author.into(),
        email: String::new(),
        date: String::new(),
        timestamp: 0,
        message: String::new(),
        subject: hash.into(),
        refs: refs
            .iter()
            .map(|name| RefLabel {
                name: (*name).into(),
                kind: RefLabelKind::LocalBranch,
            })
            .collect(),
        parents: parents.iter().map(|parent| (*parent).into()).collect(),
        graph: GraphRow::default(),
    }
}

fn reference(full: &str, kind: RefKind) -> RefEntry {
    RefEntry {
        name: full.splitn(3, '/').nth(2).unwrap_or(full).into(),
        full_name: full.into(),
        kind,
        target_hash: String::new(),
        upstream: None,
        ahead: 0,
        behind: 0,
    }
}

#[test]
fn replies_fill_the_user_and_branch_menus() {
    let mut view = LogView::default();
    let query = view.reload();
    let log = CommitLog {
        commits: vec![entry("a1", "Ada", &[], &[]), entry("b2", "Bob", &[], &[])],
        refs: vec![
            reference("refs/remotes/origin/main", RefKind::RemoteBranch),
            reference("refs/remotes/origin/HEAD", RefKind::RemoteBranch),
            reference("refs/heads/main", RefKind::LocalBranch),
            reference("refs/tags/v1", RefKind::Tag),
        ],
        me: Some("Ada".into()),
        ..reply(&query)
    };
    view.answer(&log, false);
    let branches: Vec<&str> = view
        .branches
        .iter()
        .map(|(full, _)| full.as_str())
        .collect();
    assert_eq!(branches, ["refs/heads/main", "refs/remotes/origin/main"]);
    assert_eq!(view.me.as_deref(), Some("Ada"));
    // A User filter lists fewer authors; the menu keeps the others.
    view.draft.author = Some("Bob".into());
    view.apply();
    let query = view.due(Instant::now()).unwrap();
    let only_bob = CommitLog {
        commits: vec![entry("b2", "Bob", &[], &[])],
        ..reply(&query)
    };
    view.answer(&only_bob, true);
    let authors: Vec<&str> = view.authors.iter().map(String::as_str).collect();
    assert_eq!(authors, ["Ada", "Bob"]);
}

#[test]
fn loaded_commits_are_found_by_hash_prefix_or_label() {
    let log = CommitLog {
        commits: vec![
            entry("abcdef1234", "Ada", &[], &["main"]),
            entry("abcd999999", "Ada", &[], &["v1"]),
        ],
        ..CommitLog::default()
    };
    let found = |rev: &str| log.find_loaded(rev).map(|c| c.hash.as_str());
    assert_eq!(found("ABCDEF"), Some("abcdef1234"));
    assert_eq!(found("abcd9"), Some("abcd999999"));
    assert_eq!(found(" v1 "), Some("abcd999999"));
    assert_eq!(found("main"), Some("abcdef1234"));
    assert_eq!(found("abc"), None, "a hash needs four digits");
    assert_eq!(found("feature"), None);
}

#[test]
fn filters_that_hide_commits_end_lines_at_hidden_parents() {
    // `a` and `c` match; their parents `b` and `d` are hidden.
    let commits = || vec![entry("a", "", &["b"], &[]), entry("c", "", &["d"], &[])];
    let mut whole = commits();
    assert_eq!(
        query::lay_out(&mut whole, false),
        2,
        "older history goes on"
    );
    let mut filtered = commits();
    assert_eq!(query::lay_out(&mut filtered, true), 1);
    assert!(filtered.iter().all(|c| c.graph.down.is_empty()));
    // A listed parent still joins its child.
    let mut linked = vec![entry("a", "", &["c"], &[]), entry("c", "", &["d"], &[])];
    query::lay_out(&mut linked, true);
    assert_eq!(linked[0].graph.down.len(), 1);
    assert_eq!(linked[1].graph.up.len(), 1);
}

#[test]
fn a_parent_listed_above_its_child_gets_no_line() {
    // A hash match goes first, above its child `c`.
    let mut commits = vec![
        entry("p", "", &["q"], &[]),
        entry("c", "", &["p"], &[]),
        entry("q", "", &[], &[]),
    ];
    let lanes = query::lay_out(&mut commits, true);
    assert!(commits[1].graph.down.is_empty(), "no line runs up to `p`");
    assert_eq!(commits[0].graph.down.len(), 1, "`p` still joins `q`");
    assert_eq!(lanes, 2);
}

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn commit(root: &Path, author: &str, file: &str, subject: &str) {
    std::fs::write(root.join(file), subject).unwrap();
    git(root, &["add", "."]);
    let name = format!("user.name={author}");
    git(root, &["-c", &name, "commit", "-qm", subject]);
}

/// `main`: base, src one, docs (Jim Bob), src two; `feature` from src
/// one: feature work (Ada).
fn history() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git(root, &["init", "-q", "-b", "main"]);
    git(root, &["config", "user.email", "fixture@example.invalid"]);
    git(root, &["config", "user.name", "Bob"]);
    git(root, &["config", "commit.gpgsign", "false"]);
    std::fs::create_dir(root.join("src")).unwrap();
    commit(root, "Bob", "base.txt", "base");
    commit(root, "Bob", "src/a.txt", "src one");
    git(root, &["checkout", "-qb", "feature"]);
    commit(root, "Ada", "f.txt", "feature work (fix)");
    git(root, &["checkout", "-q", "main"]);
    commit(root, "Jim Bob", "docs.txt", "docs");
    commit(root, "Bob", "src/a.txt", "src two");
    dir
}

fn load(root: &Path, filter: LogFilter, goto: Option<&str>) -> CommitLog {
    let query = LogQuery {
        filter,
        goto: goto.map(str::to_owned),
        generation: 7,
    };
    let log = query_log(&BlockingGit, root, &query);
    assert_eq!(log.error, None);
    assert_eq!(*log.query, query, "the reply names its query");
    log
}

fn subjects(log: &CommitLog) -> Vec<&str> {
    log.commits.iter().map(|c| c.subject.as_str()).collect()
}

#[test]
fn filtered_loads_follow_branch_user_text_and_path() {
    let dir = history();
    let root = dir.path();
    let all = load(root, LogFilter::default(), None);
    assert_eq!(all.commits.len(), 5);
    assert_eq!(all.me.as_deref(), Some("Bob"));
    let head = load(
        root,
        LogFilter {
            branch: LogBranch::Head,
            ..LogFilter::default()
        },
        None,
    );
    assert!(!subjects(&head).contains(&"feature work (fix)"));
    let feature = LogFilter {
        branch: LogBranch::Ref("refs/heads/feature".into()),
        ..LogFilter::default()
    };
    assert_eq!(
        subjects(&load(root, feature, None)),
        ["feature work (fix)", "src one", "base"]
    );
    let bob = LogFilter {
        author: Some("bob".into()),
        ..LogFilter::default()
    };
    // `Jim Bob` is another user; case does not matter.
    assert_eq!(
        subjects(&load(root, bob, None)),
        ["src two", "src one", "base"]
    );
    let text = LogFilter {
        text: "WORK (FIX)".into(),
        ..LogFilter::default()
    };
    assert_eq!(subjects(&load(root, text, None)), ["feature work (fix)"]);
    let path = LogFilter {
        path: "src".into(),
        ..LogFilter::default()
    };
    let src = load(root, path, None);
    assert_eq!(subjects(&src), ["src two", "src one"]);
    // Git joins `src two` to `src one` past `docs`: one unbroken lane.
    assert_eq!(src.commits[0].graph.down.len(), 1);
    assert_eq!(src.commits[1].graph.up.len(), 1);
    assert_eq!(src.lanes, 1);
    let recent = LogFilter {
        date: LogDate::Day,
        ..LogFilter::default()
    };
    assert_eq!(load(root, recent, None).commits.len(), 5);
}

#[test]
fn the_user_filter_matches_the_listed_name_also_with_a_mailmap() {
    let dir = history();
    let root = dir.path();
    // With `.mailmap`, Git matches `--author` against the mapped name.
    std::fs::write(
        root.join(".mailmap"),
        "Ada Lovelace <ada@example.invalid> Ada <fixture@example.invalid>\n",
    )
    .unwrap();
    git(root, &["config", "log.mailmap", "true"]);
    let all = load(root, LogFilter::default(), None);
    assert!(
        all.commits.iter().any(|commit| commit.author == "Ada"),
        "the list and the User menu show names as committed"
    );
    let ada = LogFilter {
        author: Some("Ada".into()),
        ..LogFilter::default()
    };
    assert_eq!(subjects(&load(root, ada, None)), ["feature work (fix)"]);
}

fn head_of(root: &Path, rev: &str) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", rev])
        .output()
        .unwrap();
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

#[test]
fn go_to_hash_and_hash_text_list_commits_the_filters_leave_out() {
    let dir = history();
    let root = dir.path();
    let base = head_of(root, "main~3");
    let docs = LogFilter {
        text: "docs".into(),
        ..LogFilter::default()
    };
    let log = load(root, docs.clone(), base.get(..9));
    assert_eq!(subjects(&log), ["docs", "base"], "added at the end");
    assert_eq!(log.found.as_deref(), Some(base.as_str()));
    assert_eq!(log.missing, None);
    let log = load(root, docs, Some("no-such-ref"));
    assert_eq!(log.found, None);
    assert_eq!(
        log.missing.as_deref(),
        Some("No commit matches 'no-such-ref'")
    );
    // A typed hash lists its commit first, next to message matches.
    let by_hash = LogFilter {
        text: base.get(..10).unwrap().to_uppercase(),
        ..LogFilter::default()
    };
    let log = load(root, by_hash, None);
    assert_eq!(subjects(&log), ["base"]);
}

#[test]
fn option_like_filter_values_reach_git_as_values() {
    let dir = history();
    let root = dir.path();
    let marker = root.join("written-by-git");
    let filter = LogFilter {
        text: format!("--output={}", marker.display()),
        author: Some("--all".into()),
        path: "-rf".into(),
        ..LogFilter::default()
    };
    let log = load(root, filter, Some("--all"));
    assert!(log.commits.is_empty());
    assert_eq!(log.missing.as_deref(), Some("'--all' is not a revision"));
    assert!(!marker.exists(), "no option reached Git");
}
