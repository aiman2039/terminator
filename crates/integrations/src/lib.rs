//! Managed hook configuration. Installers only run on explicit user action.
#![forbid(unsafe_code)]
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};
use terminator_core::{
    AgentState, HookEvent, PROTOCOL_VERSION, Paths, Resume, atomic_write, id, quote,
};
pub const AGENTS: [&str; 6] = ["claude", "codex", "opencode", "muse", "grok", "cursor"];
const MARKER: &str = "terminator-managed:v1";
pub fn config_path(home: &Path, kind: &str) -> Result<PathBuf> {
    Ok(home.join(match kind {
        "claude" => ".claude/settings.json",
        "codex" => ".codex/config.toml",
        "opencode" => ".config/opencode/plugins/terminator.js",
        "muse" => ".muse/hooks.json",
        "grok" => ".grok/hooks/terminator.json",
        "cursor" => ".cursor/hooks.json",
        _ => bail!("Unknown built-in integration"),
    }))
}
#[must_use]
pub fn installed(home: &Path, kind: &str) -> bool {
    let mut paths = Vec::new();
    if let Ok(path) = config_path(home, kind) {
        paths.push(path);
    }
    if kind == "muse" {
        paths.push(muse_settings_path(home));
    }
    paths
        .iter()
        .any(|path| fs::read_to_string(path).is_ok_and(|text| text.contains(MARKER)))
}

/// Settings hover path. Muse Code 1.4 reads user hooks from this file.
#[must_use]
pub fn settings_path(home: &Path, kind: &str) -> PathBuf {
    if kind == "muse" {
        muse_settings_path(home)
    } else {
        config_path(home, kind).unwrap_or_else(|_| home.to_path_buf())
    }
}

fn muse_settings_path(home: &Path) -> PathBuf {
    home.join(".config/muse/settings.json")
}

/// Cursor lifecycle hooks. Permission hooks (`preToolUse`, `beforeShellExecution`,
/// `beforeMCPExecution`, `beforeReadFile`, `subagentStart`) are omitted: exit 0
/// with a body that fails their schema blocks the action.
fn cursor_hook_events() -> &'static [(&'static str, &'static str)] {
    &[
        ("sessionStart", "{}"),
        ("beforeSubmitPrompt", "{\"continue\":true}"),
        ("postToolUse", "{}"),
        ("postToolUseFailure", "{}"),
        ("afterAgentResponse", "{}"),
        ("stop", "{}"),
        ("sessionEnd", "{}"),
    ]
}

fn muse_hook_events() -> &'static [&'static str] {
    &[
        "SessionStart",
        "UserPromptSubmit",
        "PreToolUse",
        "PostToolUse",
        "PermissionRequest",
        "Stop",
        "SubagentStart",
        "SubagentStop",
        "SessionEnd",
    ]
}
pub fn install(home: &Path, kind: &str, helper: &Path, remove: bool) -> Result<PathBuf> {
    install_at(home, kind, helper, remove, &Paths::discover()?)
}
pub fn install_at(
    home: &Path,
    kind: &str,
    helper: &Path,
    remove: bool,
    paths: &Paths,
) -> Result<PathBuf> {
    let path = config_path(home, kind)?;
    let before = fs::read_to_string(&path).or_else(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            Ok(String::new())
        } else {
            Err(e)
        }
    })?;
    ensure!(
        !path.is_symlink(),
        "Refusing to rewrite a symlinked agent configuration"
    );
    let invocation = format!(
        "{} event {} --data-dir {} --runtime-dir {}",
        quote(&helper.to_string_lossy()),
        kind,
        quote(&paths.data.to_string_lossy()),
        quote(&paths.runtime.to_string_lossy())
    );
    let command = format!("{invocation} # {MARKER}");
    let after = if kind == "opencode" {
        ensure!(
            before.is_empty() || before.contains(MARKER),
            "Existing plugin is not managed by Terminator"
        );
        if remove {
            String::new()
        } else {
            opencode_plugin(helper)
        }
    } else if kind == "codex" {
        codex_config(&before, &command, remove)?
    } else if kind == "cursor" {
        if remove && before.trim().is_empty() {
            String::new()
        } else {
            cursor_config(&before, &invocation, remove)?
        }
    } else {
        let mut doc: Value = if before.trim().is_empty() {
            json!({})
        } else {
            serde_json::from_str(&before)
                .context("Agent configuration is invalid JSON; left untouched")?
        };
        ensure!(doc.is_object(), "Expected config object");
        if doc.get("hooks").is_none() {
            doc.as_object_mut()
                .context("Expected config object")?
                .insert("hooks".to_owned(), json!({}));
        }
        let hooks = doc
            .get_mut("hooks")
            .and_then(Value::as_object_mut)
            .context("hooks is not an object")?;
        for groups in hooks.values_mut() {
            clean_json_groups(groups)?;
        }
        if !remove {
            let events = if kind == "muse" {
                muse_hook_events().to_vec()
            } else {
                vec![
                    "SessionStart",
                    "UserPromptSubmit",
                    "PreToolUse",
                    "PostToolUse",
                    "PermissionRequest",
                    "Notification",
                    "Stop",
                    "StopFailure",
                    "SessionEnd",
                ]
            };
            for event in events {
                let list = hooks
                    .entry(event)
                    .or_insert(json!([]))
                    .as_array_mut()
                    .context("Hook groups must be arrays")?;
                list.push(json!({"hooks":[{"type":"command","command":command,"timeout":2}]}));
            }
        }
        serde_json::to_string_pretty(&doc)? + "\n"
    };
    if !before.is_empty() && before != after {
        atomic_write(&backup_path(&path), before.as_bytes())?;
    }
    if remove && kind == "opencode" {
        if path.exists() {
            fs::remove_file(&path)?;
        }
    } else if !(after.is_empty() && !path.exists()) {
        write_config(&path, after.as_bytes())?;
    }
    if kind == "muse" {
        sync_muse_settings(home, &command, remove)?;
    }
    Ok(path)
}

/// A backup name no earlier backup uses: two installs within one second
/// must not overwrite the backup of the user's original configuration.
fn backup_path(path: &Path) -> PathBuf {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis());
    let base = format!("terminator-backup-{millis}");
    let mut candidate = path.with_extension(&base);
    let mut n = 1u32;
    while candidate.exists() {
        candidate = path.with_extension(format!("{base}-{n}"));
        n = n.saturating_add(1);
    }
    candidate
}

/// Replace an agent's configuration file, keeping the permissions the user
/// gave it instead of the private mode used for new files.
fn write_config(path: &Path, bytes: &[u8]) -> Result<()> {
    let permissions = fs::metadata(path).ok().map(|meta| meta.permissions());
    atomic_write(path, bytes)?;
    if let Some(permissions) = permissions {
        fs::set_permissions(path, permissions)?;
    }
    Ok(())
}

fn sync_muse_settings(home: &Path, command: &str, remove: bool) -> Result<()> {
    let path = muse_settings_path(home);
    ensure!(
        !path.is_symlink(),
        "Refusing to rewrite a symlinked Muse settings file"
    );
    let before = fs::read_to_string(&path).or_else(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            Ok(String::new())
        } else {
            Err(error)
        }
    })?;
    if before.is_empty() && remove {
        return Ok(());
    }
    let mut doc: Value = if before.trim().is_empty() {
        json!({"schema_version": 1})
    } else {
        serde_json::from_str(&before).context("Muse settings are invalid JSON; left untouched")?
    };
    ensure!(doc.is_object(), "Muse settings must be an object");
    match doc.get("schema_version") {
        None => {
            doc.as_object_mut()
                .context("Muse settings must be an object")?
                .insert("schema_version".to_owned(), json!(1));
        }
        Some(version) if version.as_u64() == Some(1) => {}
        Some(_) => bail!("Muse settings schema is not version 1; left untouched"),
    }
    if doc.get("hooks").is_none() {
        doc.as_object_mut()
            .context("Muse settings must be an object")?
            .insert("hooks".to_owned(), json!({}));
    }
    let hooks = doc
        .get_mut("hooks")
        .and_then(Value::as_object_mut)
        .context("Muse hooks must be an object")?;
    for groups in hooks.values_mut() {
        clean_json_groups(groups)?;
    }
    if !remove {
        for event in muse_hook_events() {
            let list = hooks
                .entry(*event)
                .or_insert(json!([]))
                .as_array_mut()
                .context("Hook groups must be arrays")?;
            list.push(json!({"hooks":[{"type":"command","command":command,"timeout":2}]}));
        }
    }
    let after = serde_json::to_string_pretty(&doc)? + "\n";
    if !before.is_empty() && before != after {
        atomic_write(&backup_path(&path), before.as_bytes())?;
    }
    if before != after {
        write_config(&path, after.as_bytes())?;
    }
    Ok(())
}
fn clean_json_groups(groups: &mut Value) -> Result<()> {
    let groups = groups
        .as_array_mut()
        .context("Hook groups must be arrays")?;
    for group in groups.iter_mut() {
        if let Some(hooks) = group.get_mut("hooks").and_then(Value::as_array_mut) {
            hooks.retain(|h| {
                !h.get("command")
                    .and_then(Value::as_str)
                    .is_some_and(|c| c.contains(MARKER))
            });
        }
    }
    groups.retain(|g| {
        !g.get("hooks")
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty)
    });
    Ok(())
}
fn cursor_config(before: &str, invocation: &str, remove: bool) -> Result<String> {
    let mut doc: Value = if before.trim().is_empty() {
        json!({"version": 1})
    } else {
        serde_json::from_str(before).context("Cursor hooks.json is invalid JSON; left untouched")?
    };
    ensure!(doc.is_object(), "Cursor hooks.json must be an object");
    match doc.get("version") {
        None => {
            doc.as_object_mut()
                .context("Cursor hooks.json must be an object")?
                .insert("version".to_owned(), json!(1));
        }
        Some(version) if version.as_u64() == Some(1) => {}
        Some(_) => bail!("Cursor hooks.json version is not 1; left untouched"),
    }
    if doc.get("hooks").is_none() {
        doc.as_object_mut()
            .context("Cursor hooks.json must be an object")?
            .insert("hooks".to_owned(), json!({}));
    }
    let hooks = doc
        .get_mut("hooks")
        .and_then(Value::as_object_mut)
        .context("Cursor hooks must be an object")?;
    for &(event, response) in cursor_hook_events() {
        let list = hooks
            .entry(event)
            .or_insert(json!([]))
            .as_array_mut()
            .with_context(|| format!("Cursor {event} hooks must be an array"))?;
        if list.iter().any(|entry| {
            entry.as_object().is_some_and(|object| {
                object.contains_key("hooks") && !object.contains_key("command")
            })
        }) {
            bail!("Unsupported Cursor hook format for {event}; left untouched");
        }
        list.retain(|entry| {
            !entry
                .get("command")
                .and_then(Value::as_str)
                .is_some_and(|command| command.contains(MARKER))
        });
        if !remove {
            list.push(json!({
                "command": format!("{invocation} ; printf '%s\\n' '{response}' # {MARKER}"),
                "timeout": 2
            }));
        }
    }
    if remove {
        hooks.retain(|_, value| value.as_array().is_none_or(|list| !list.is_empty()));
    }
    Ok(serde_json::to_string_pretty(&doc)? + "\n")
}

fn codex_config(before: &str, command: &str, remove: bool) -> Result<String> {
    use toml_edit::{Array, ArrayOfTables, DocumentMut, InlineTable, Item, Table, Value as T};
    let mut doc = before
        .parse::<DocumentMut>()
        .context("Invalid Codex TOML; left untouched")?;
    if doc.get("hooks").is_none() {
        doc.insert("hooks", Item::Table(Table::new()));
    }
    let hooks = doc
        .get_mut("hooks")
        .and_then(Item::as_table_mut)
        .context("Codex hooks must be a table")?;
    let events = [
        "SessionStart",
        "UserPromptSubmit",
        "PreToolUse",
        "PostToolUse",
        "PermissionRequest",
        "Notification",
        "Stop",
        "SessionEnd",
        "Interrupt",
    ];
    for event in events {
        if let Some(item) = hooks.get_mut(event) {
            if let Some(groups) = item.as_array_of_tables_mut() {
                for group in groups.iter_mut() {
                    if let Some(h) = group.get_mut("hooks").and_then(Item::as_array_mut) {
                        h.retain(|v| {
                            !v.as_inline_table()
                                .and_then(|t| t.get("command"))
                                .and_then(T::as_str)
                                .is_some_and(|c| c.contains(MARKER))
                        });
                    }
                }
                groups.retain(|g| {
                    g.get("hooks")
                        .and_then(Item::as_array)
                        .is_none_or(|a| !a.is_empty())
                });
            } else if let Some(groups) = item.as_array_mut() {
                for g in groups.iter_mut() {
                    if let Some(h) = g
                        .as_inline_table_mut()
                        .and_then(|t| t.get_mut("hooks"))
                        .and_then(T::as_array_mut)
                    {
                        h.retain(|v| {
                            !v.as_inline_table()
                                .and_then(|t| t.get("command"))
                                .and_then(T::as_str)
                                .is_some_and(|c| c.contains(MARKER))
                        });
                    }
                }
                groups.retain(|g| {
                    g.as_inline_table()
                        .and_then(|t| t.get("hooks"))
                        .and_then(T::as_array)
                        .is_none_or(|h| !h.is_empty())
                });
            } else {
                bail!("Unsupported Codex hook format for {event}; left untouched")
            }
        }
        if !remove {
            let mut handler = InlineTable::new();
            handler.insert("type", T::from("command"));
            handler.insert("command", T::from(command));
            handler.insert("timeout", T::from(2));
            let mut hs = Array::new();
            hs.push(handler);
            if let Some(arr) = hooks.get_mut(event).and_then(Item::as_array_mut) {
                let mut group = InlineTable::new();
                group.insert("hooks", T::Array(hs));
                arr.push(group);
            } else {
                if hooks.get(event).is_none() {
                    hooks.insert(event, Item::ArrayOfTables(ArrayOfTables::new()));
                }
                let mut group = Table::new();
                group.insert("hooks", Item::Value(T::Array(hs)));
                hooks
                    .get_mut(event)
                    .and_then(Item::as_array_of_tables_mut)
                    .context("Unsupported Codex hook format for {event}; left untouched")?
                    .push(group);
            }
        }
    }
    Ok(doc.to_string())
}
fn opencode_plugin(helper: &Path) -> String {
    let helper = json!(helper.to_string_lossy()).to_string();
    format!(
        r"// {MARKER}
import {{ spawn }} from 'node:child_process';
import {{ randomUUID }} from 'node:crypto';
const TYPES = ['session.created','session.status','session.idle','session.error','session.deleted','permission.asked','permission.replied','permission.v2.asked','permission.v2.replied','question.asked','question.replied','question.rejected','question.v2.asked','question.v2.replied','question.v2.rejected'];
let started = false;
function bind() {{
  if (!process.env.TERMINATOR_SESSION_ID || started) return null;
  started = true;
  return {{ invocation: randomUUID(), seq: {{ n: 0 }} }};
}}
function emit(invocation, seq, event) {{
  const type = event && event.type;
  if (!TYPES.includes(type)) return;
  const payload = {{...event, terminator_invocation: invocation, terminator_sequence: ++seq.n, terminator_event: randomUUID()}};
  const child = spawn({helper}, ['event','opencode'], {{stdio:['pipe','ignore','ignore'], env:process.env}});
  child.on('error', () => {{}}); child.stdin.on('error', () => {{}});
  child.stdin.end(JSON.stringify(payload));
  const timer = setTimeout(() => child.kill(), 1800); timer.unref(); child.on('exit', () => clearTimeout(timer));
}}
async function server() {{
  const s = bind();
  if (!s) return {{}};
  return {{ event: async ({{ event }}) => emit(s.invocation, s.seq, event) }};
}}
export const Terminator = server;
export default {{
  id: 'terminator',
  server,
  async setup(ctx) {{
    const s = bind();
    if (!s) return;
    const c = new AbortController();
    void (async () => {{
      for await (const event of ctx.event.subscribe({{ signal: c.signal }})) emit(s.invocation, s.seq, event);
    }})();
    return () => c.abort();
  }},
}};
"
    )
}

mod normalize;
pub use normalize::normalize;

#[cfg(test)]
mod tests;
