//! Codex conversations: `sessions/YYYY/MM/DD/rollout-<time>-<id>.jsonl`,
//! the folder in the first (session meta) line, thread names in
//! `session_index.jsonl` next to `sessions`.
use super::{
    Budget, CODEX_HEAD, Conversation, INDEX_TAIL, clean, head_lines, modified, prompt_text,
    tail_text, valid_id,
};
use serde_json::Value;
use std::{collections::HashMap, fs::File, path::Path};

/// The id of a session meta line for one of `folders`. Sub-agent threads
/// (`source: {"subagent": …}`) are skipped.
fn meta(line: &str, folders: &[String]) -> Option<String> {
    let value: Value = serde_json::from_str(line).ok()?;
    if value.get("type")?.as_str()? != "session_meta" {
        return None;
    }
    let payload = value.get("payload")?;
    let cwd = payload.get("cwd")?.as_str()?;
    if !folders.iter().any(|folder| folder == cwd)
        || payload.get("source").is_some_and(Value::is_object)
    {
        return None;
    }
    let id = payload.get("id")?.as_str()?;
    valid_id(id).then(|| id.to_owned())
}

/// The text of a typed user message event.
fn prompt(line: &str) -> Option<String> {
    if !line.contains("\"event_msg\"") {
        return None;
    }
    let value: Value = serde_json::from_str(line).ok()?;
    if value.get("type")?.as_str()? != "event_msg" {
        return None;
    }
    let payload = value.get("payload")?;
    match payload.get("type")?.as_str()? {
        "user_message" => prompt_text(payload.get("message")?.as_str()?),
        "item_completed" => {
            let item = payload.get("item")?;
            (item.get("type")?.as_str()? == "UserMessage").then_some(())?;
            let text = item
                .get("content")?
                .as_array()?
                .iter()
                .find_map(|part| part.get("text")?.as_str())?;
            prompt_text(text)
        }
        _ => None,
    }
}

/// Id, first prompt and change time of one session file for `folders`.
fn read(
    path: &Path,
    folders: &[String],
    budget: &mut Budget<'_>,
) -> Option<(String, Option<String>, u64)> {
    let file = File::open(path).ok()?;
    let updated = modified(&file);
    let mut id = None;
    let mut first = None;
    head_lines(&file, CODEX_HEAD, budget, |line| {
        if id.is_none() {
            // The first line is the session meta; stop when it is not ours.
            id = meta(line, folders);
            return id.is_some();
        }
        first = prompt(line);
        first.is_none()
    });
    Some((id?, first, updated))
}

/// Thread names Codex keeps in `session_index.jsonl`; a later line wins.
fn thread_names(index: &Path, budget: &mut Budget<'_>) -> HashMap<String, String> {
    let mut names = HashMap::new();
    if !budget.open() {
        return names;
    }
    let Ok(file) = File::open(index) else {
        return names;
    };
    for line in tail_text(&file, 0, INDEX_TAIL, budget).lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let id = value.get("id").and_then(Value::as_str);
        let name = value
            .get("thread_name")
            .and_then(Value::as_str)
            .and_then(clean);
        if let (Some(id), Some(name)) = (id, name) {
            names.insert(id.to_owned(), name);
        }
    }
    names
}

/// Conversations of `folders` under `sessions`, at most `listed`.
pub(super) fn conversations(
    sessions: &Path,
    folders: &[String],
    budget: &mut Budget<'_>,
    listed: usize,
) -> Vec<Conversation> {
    // Codex names folders `YYYY/MM/DD` and files `rollout-<time>-<id>`, so a
    // walk in reverse name order meets the newest first. Links are not
    // followed.
    let files = walkdir::WalkDir::new(sessions)
        .max_depth(4)
        .sort_by(|a, b| b.file_name().cmp(a.file_name()))
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with("rollout-") && name.ends_with(".jsonl"))
        });
    let mut found = Vec::new();
    for entry in files {
        if found.len() >= listed || !budget.open() {
            break;
        }
        found.extend(read(entry.path(), folders, budget));
    }
    if found.is_empty() {
        return Vec::new();
    }
    let names = sessions
        .parent()
        .map(|home| thread_names(&home.join("session_index.jsonl"), budget))
        .unwrap_or_default();
    found
        .into_iter()
        .filter_map(|(id, first, updated)| {
            let title = names.get(&id).cloned().or(first)?;
            Some(Conversation {
                agent: "codex",
                id,
                title,
                updated,
            })
        })
        .collect()
}
