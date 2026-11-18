//! Claude Code conversations: `~/.claude/projects/<folder name>/<id>.jsonl`.
use super::{
    Budget, CLAUDE_HEAD, Conversation, MAX_DIR_ENTRIES, TITLE_TAIL, clean, head_lines, modified,
    prompt_text, tail_text, unix, valid_id,
};
use serde_json::Value;
use std::{
    fs::File,
    path::{Path, PathBuf},
};

/// Claude Code's folder name for a working directory: every character
/// that is not an ASCII letter or digit becomes `-`.
#[must_use]
pub fn claude_dir_name(folder: &str) -> String {
    folder
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

fn flag(value: &Value, key: &str) -> bool {
    value.get(key).and_then(Value::as_bool).unwrap_or(false)
}

/// A title record: a user-set title wins over an AI title, which wins over
/// an older summary. Higher rank wins.
fn title(value: &Value) -> Option<(u8, String)> {
    let (rank, key) = match value.get("type")?.as_str()? {
        "custom-title" => (3, "customTitle"),
        "ai-title" => (2, "aiTitle"),
        "summary" => (1, "summary"),
        _ => return None,
    };
    Some((rank, clean(value.get(key)?.as_str()?)?))
}

fn better(current: Option<(u8, String)>, new: Option<(u8, String)>) -> Option<(u8, String)> {
    match (current, new) {
        (Some(current), Some(new)) if new.0 < current.0 => Some(current),
        (current, new) => new.or(current),
    }
}

/// The text of a typed user message.
fn prompt(value: &Value) -> Option<String> {
    if value.get("type")?.as_str()? != "user" || flag(value, "isMeta") || flag(value, "isSidechain")
    {
        return None;
    }
    let text = match value.get("message")?.get("content")? {
        Value::String(text) => text.as_str(),
        Value::Array(blocks) => blocks.iter().find_map(|block| {
            (block.get("type")?.as_str()? == "text").then_some(())?;
            block.get("text")?.as_str()
        })?,
        _ => return None,
    };
    prompt_text(text)
}

#[derive(Default)]
struct Head {
    cwd: Option<String>,
    prompt: Option<String>,
    title: Option<(u8, String)>,
}

impl Head {
    fn take(&mut self, line: &str) {
        if !(line.contains("\"user\"") || line.contains("title\"") || line.contains("\"summary\""))
        {
            return;
        }
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            return;
        };
        if self.cwd.is_none() {
            self.cwd = value.get("cwd").and_then(Value::as_str).map(str::to_owned);
        }
        if self.prompt.is_none() {
            self.prompt = prompt(&value);
        }
        self.title = better(self.title.take(), title(&value));
    }
}

/// One conversation file, if it belongs to `folders`.
fn read(path: &Path, folders: &[String], budget: &mut Budget<'_>) -> Option<Conversation> {
    let id = path.file_stem()?.to_str()?;
    if !valid_id(id) {
        return None;
    }
    let file = File::open(path).ok()?;
    let updated = modified(&file);
    let mut head = Head::default();
    let head_bytes = head_lines(&file, CLAUDE_HEAD, budget, |line| {
        head.take(line);
        head.cwd.is_none() || head.prompt.is_none()
    });
    // The folder name is lossy; the recorded folder decides.
    if head.cwd.as_ref().is_some_and(|cwd| !folders.contains(cwd)) {
        return None;
    }
    // The newest title is near the end of the file.
    for line in tail_text(&file, head_bytes, TITLE_TAIL, budget).lines() {
        if let Ok(value) = serde_json::from_str::<Value>(line) {
            head.title = better(head.title.take(), title(&value));
        }
    }
    let title = head.title.map(|(_, title)| title).or(head.prompt)?;
    Some(Conversation {
        agent: "claude",
        id: id.to_owned(),
        title,
        updated,
    })
}

/// `.jsonl` files (links not followed) of one folder, newest first.
fn files(dir: &Path) -> Vec<(u64, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<(u64, PathBuf)> = entries
        .take(MAX_DIR_ENTRIES)
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let meta = entry.metadata().ok()?;
            let path = entry.path();
            (meta.is_file() && path.extension().is_some_and(|x| x == "jsonl")).then_some(())?;
            Some((unix(meta.modified()), path))
        })
        .collect();
    files.sort_by_key(|(updated, _)| std::cmp::Reverse(*updated));
    files
}

/// Conversations of `folders` under `projects`, at most `listed`.
pub(super) fn conversations(
    projects: &Path,
    folders: &[String],
    budget: &mut Budget<'_>,
    listed: usize,
) -> Vec<Conversation> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    for folder in folders {
        let dir = projects.join(claude_dir_name(folder));
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    let mut found = Vec::new();
    for (_, path) in dirs.iter().flat_map(|dir| files(dir)) {
        if found.len() >= listed || !budget.open() {
            break;
        }
        found.extend(read(&path, folders, budget));
    }
    found
}
