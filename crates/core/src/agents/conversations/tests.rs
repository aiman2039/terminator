//! Agent sessions read from Claude Code and Codex fixture files in a
//! temporary folder; the user's own agent files are never read.
use super::*;
use serde_json::{Value, json};

const CODEX_ID: &str = "01a1df77-e352-7740-8432-39622de34a19";

struct Fixture {
    _dir: tempfile::TempDir,
    roots: Roots,
    folder: PathBuf,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("project");
    std::fs::create_dir_all(&folder).unwrap();
    let roots = Roots {
        claude_projects: Some(dir.path().join("claude").join("projects")),
        codex_sessions: Some(dir.path().join("codex").join("sessions")),
    };
    Fixture {
        _dir: dir,
        roots,
        folder,
    }
}

fn lines(values: &[Value]) -> String {
    let mut text = String::new();
    for value in values {
        text.push_str(&value.to_string());
        text.push('\n');
    }
    text
}

/// Writes `text` and sets the file time to `age` seconds after the epoch.
fn write(path: &Path, text: &str, age: u64) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
    let time = UNIX_EPOCH.checked_add(Duration::from_secs(age)).unwrap();
    File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(time)
        .unwrap();
}

impl Fixture {
    fn folder_text(&self) -> String {
        self.folder.to_string_lossy().into_owned()
    }

    fn claude(&self, (folder, id): (&str, &str), records: &[Value], age: u64) {
        let dir = self
            .roots
            .claude_projects
            .as_ref()
            .unwrap()
            .join(claude_dir_name(folder));
        write(&dir.join(format!("{id}.jsonl")), &lines(records), age);
    }

    fn codex(&self, name: &str, records: &[Value], age: u64) {
        let dir = self.roots.codex_sessions.as_ref().unwrap();
        write(&dir.join("2026/11/15").join(name), &lines(records), age);
    }

    fn list(&self, limits: Limits) -> Listing {
        list(&self.roots, &self.folder, limits, &|| false)
    }
}

fn claude_records(cwd: &str, prompt: &str) -> Vec<Value> {
    vec![
        json!({"type": "permission-mode", "permissionMode": "auto"}),
        json!({"type": "user", "isMeta": true, "cwd": cwd,
            "message": {"role": "user", "content": "<local-command-caveat>x</local-command-caveat>"}}),
        json!({"type": "user", "cwd": cwd,
            "message": {"role": "user", "content": [{"type": "text", "text": prompt}]}}),
    ]
}

fn codex_records(id: &str, cwd: &str, source: Value, prompt: &str) -> Vec<Value> {
    vec![
        json!({"type": "session_meta", "payload": {"id": id, "cwd": cwd, "source": source}}),
        json!({"type": "response_item", "payload": {"type": "message", "role": "user",
            "content": [{"type": "input_text", "text": "# AGENTS.md instructions"}]}}),
        json!({"type": "event_msg", "payload": {"type": "item_completed",
            "item": {"type": "UserMessage", "content": [{"type": "text", "text": prompt}]}}}),
    ]
}

#[test]
fn claude_conversation_lists_its_title_else_the_first_typed_prompt() {
    let fixture = fixture();
    let cwd = fixture.folder_text();
    let mut titled = claude_records(&cwd, "Fix the reconnect bug");
    titled.push(json!({"type": "ai-title", "aiTitle": "Reconnect after GUI exit"}));
    fixture.claude((&cwd, "11111111-aaaa"), &titled, 200);
    fixture.claude(
        (&cwd, "22222222-bbbb"),
        &claude_records(&cwd, "  Add a\n  sidebar   row "),
        100,
    );
    let listing = fixture.list(Limits::default());
    assert!(!listing.stopped);
    let rows: Vec<_> = listing
        .conversations
        .iter()
        .map(|c| (c.agent, c.id.as_str(), c.title.as_str(), c.updated))
        .collect();
    assert_eq!(
        rows,
        [
            ("claude", "11111111-aaaa", "Reconnect after GUI exit", 200),
            ("claude", "22222222-bbbb", "Add a sidebar row", 100),
        ]
    );
}

#[test]
fn claude_title_after_a_long_head_is_read_from_the_tail() {
    let fixture = fixture();
    let cwd = fixture.folder_text();
    let mut records = claude_records(&cwd, "first prompt");
    records.push(json!({"type": "api-request-blob", "blob": "x".repeat(300 * 1024)}));
    records.push(json!({"type": "custom-title", "customTitle": "Named by the user"}));
    records.push(json!({"type": "ai-title", "aiTitle": "Later AI title"}));
    fixture.claude((&cwd, "33333333"), &records, 10);
    let listing = fixture.list(Limits::default());
    assert_eq!(listing.conversations[0].title, "Named by the user");
}

#[test]
fn codex_conversation_lists_its_thread_name_else_the_first_typed_prompt() {
    let fixture = fixture();
    let cwd = fixture.folder_text();
    let named = codex_records(CODEX_ID, &cwd, json!("vscode"), "hello");
    fixture.codex(
        &format!("rollout-2026-11-15T15-48-59-{CODEX_ID}.jsonl"),
        &named,
        300,
    );
    let other = "01a1df56-b457-7cd3-8d8d-c217aedb6841";
    let plain = codex_records(other, &cwd, json!("cli"), "Explain the daemon");
    fixture.codex(
        &format!("rollout-2026-11-15T15-12-43-{other}.jsonl"),
        &plain,
        50,
    );
    let index = fixture
        .roots
        .codex_sessions
        .as_ref()
        .unwrap()
        .with_file_name("session_index.jsonl");
    let names = [
        json!({"id": CODEX_ID, "thread_name": "Old name"}),
        json!({"id": CODEX_ID, "thread_name": "Respond to greeting"}),
    ];
    write(&index, &lines(&names), 1);
    let rows: Vec<_> = fixture
        .list(Limits::default())
        .conversations
        .into_iter()
        .map(|c| (c.agent, c.id, c.title))
        .collect();
    assert_eq!(
        rows,
        [
            (
                "codex",
                CODEX_ID.to_owned(),
                "Respond to greeting".to_owned()
            ),
            ("codex", other.to_owned(), "Explain the daemon".to_owned()),
        ]
    );
}

#[test]
fn conversations_of_other_folders_sub_agents_and_empty_files_are_not_listed() {
    let fixture = fixture();
    let cwd = fixture.folder_text();
    let elsewhere = format!("{cwd}-other");
    // Same Claude folder name, other recorded folder (the name is lossy).
    let lossy = format!("{cwd}.");
    fixture.claude((&cwd, "44444444"), &claude_records(&lossy, "not ours"), 1);
    fixture.claude(
        (&elsewhere, "55555555"),
        &claude_records(&elsewhere, "x"),
        1,
    );
    fixture.claude((&cwd, "66666666"), &[json!({"type": "mode"})], 1);
    let sub = codex_records(
        "01a1c4a5-d760-70f0-a814-3fc4e58dc5a1",
        &cwd,
        json!({"subagent": {"other": "guardian"}}),
        "review",
    );
    fixture.codex("rollout-2026-11-15T10-00-00-sub.jsonl", &sub, 1);
    let far = codex_records("01a1c4a5-0000", &elsewhere, json!("cli"), "y");
    fixture.codex("rollout-2026-11-15T09-00-00-far.jsonl", &far, 1);
    let listing = fixture.list(Limits::default());
    assert!(listing.conversations.is_empty(), "{listing:?}");
    assert!(!listing.stopped);
}

#[test]
fn listing_stops_at_the_file_limit_and_keeps_what_it_read() {
    let fixture = fixture();
    let cwd = fixture.folder_text();
    for (index, id) in ["aaaa", "bbbb", "cccc"].into_iter().enumerate() {
        let age = 10_u64.saturating_add(u64::try_from(index).unwrap());
        fixture.claude((&cwd, id), &claude_records(&cwd, id), age);
    }
    let limits = Limits {
        files: 2,
        ..Limits::default()
    };
    let listing = fixture.list(limits);
    assert!(listing.stopped);
    let ids: Vec<_> = listing
        .conversations
        .iter()
        .map(|c| c.id.as_str())
        .collect();
    assert_eq!(ids, ["cccc", "bbbb"], "the newest files are read first");
    let shown = fixture.list(Limits {
        listed: 1,
        ..Limits::default()
    });
    assert_eq!(shown.conversations.len(), 1);
    let stopped = list(&fixture.roots, &fixture.folder, Limits::default(), &|| true);
    assert!(stopped.stopped && stopped.conversations.is_empty());
}

#[test]
fn ids_with_shell_characters_are_rejected_and_not_listed() {
    for good in [CODEX_ID, "abc_DEF-123"] {
        assert!(valid_id(good), "{good}");
    }
    let long = "a".repeat(129);
    for bad in [
        "",
        "-rf",
        "a b",
        "a;rm",
        "$(x)",
        "a'b",
        "a/b",
        "é",
        long.as_str(),
    ] {
        assert!(!valid_id(bad), "{bad}");
    }
    let fixture = fixture();
    let cwd = fixture.folder_text();
    fixture.claude((&cwd, "bad;id"), &claude_records(&cwd, "x"), 1);
    let meta = codex_records("$(touch x)", &cwd, json!("cli"), "y");
    fixture.codex("rollout-2026-11-15T08-00-00-bad.jsonl", &meta, 1);
    assert!(fixture.list(Limits::default()).conversations.is_empty());
}

#[test]
fn long_titles_are_cut_to_one_line() {
    let title = clean(&format!("a\n{}", "b".repeat(500))).unwrap();
    assert_eq!(title.chars().count(), TITLE_CHARS);
    assert!(title.starts_with("a b") && title.ends_with('…'));
    assert_eq!(
        claude_dir_name("/Users/me/my.proj_x"),
        "-Users-me-my-proj-x"
    );
}
