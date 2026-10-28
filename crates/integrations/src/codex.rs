//! Codex `config.toml` hooks: managed handlers are removed, then re-added.
use super::*;
use toml_edit::{Array, ArrayOfTables, DocumentMut, InlineTable, Item, Table, Value as T};

const EVENTS: [&str; 9] = [
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

pub(crate) fn codex_config(before: &str, command: &str, remove: bool) -> Result<String> {
    let mut doc = before
        .parse::<DocumentMut>()
        .map_err(|source| Error::Toml {
            context: "Invalid Codex TOML; left untouched",
            source,
        })?;
    if doc.get("hooks").is_none() {
        doc.insert("hooks", Item::Table(Table::new()));
    }
    let hooks = doc
        .get_mut("hooks")
        .and_then(Item::as_table_mut)
        .ok_or(Error::CodexHooksNotTable)?;
    for event in EVENTS {
        if let Some(item) = hooks.get_mut(event) {
            strip_event(item, event)?;
        }
        if !remove {
            add_hook(hooks, event, command)?;
        }
    }
    Ok(doc.to_string())
}

/// Removes managed handlers, and the groups they leave empty, from one event.
fn strip_event(item: &mut Item, event: &'static str) -> Result<()> {
    if let Some(groups) = item.as_array_of_tables_mut() {
        strip_table_groups(groups);
        return Ok(());
    }
    if let Some(groups) = item.as_array_mut() {
        strip_inline_groups(groups);
        return Ok(());
    }
    Err(Error::UnsupportedCodexFormat { event })
}

fn managed(handler: &T) -> bool {
    handler
        .as_inline_table()
        .and_then(|t| t.get("command"))
        .and_then(T::as_str)
        .is_some_and(|c| c.contains(MARKER))
}

fn strip_table_groups(groups: &mut ArrayOfTables) {
    for group in groups.iter_mut() {
        if let Some(h) = group.get_mut("hooks").and_then(Item::as_array_mut) {
            h.retain(|v| !managed(v));
        }
    }
    groups.retain(|g| {
        g.get("hooks")
            .and_then(Item::as_array)
            .is_none_or(|a| !a.is_empty())
    });
}

fn strip_inline_groups(groups: &mut Array) {
    for g in groups.iter_mut() {
        if let Some(h) = g
            .as_inline_table_mut()
            .and_then(|t| t.get_mut("hooks"))
            .and_then(T::as_array_mut)
        {
            h.retain(|v| !managed(v));
        }
    }
    groups.retain(|g| {
        g.as_inline_table()
            .and_then(|t| t.get("hooks"))
            .and_then(T::as_array)
            .is_none_or(|h| !h.is_empty())
    });
}

/// Appends one managed handler group, in the event's existing table style.
fn add_hook(hooks: &mut Table, event: &'static str, command: &str) -> Result<()> {
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
        return Ok(());
    }
    if hooks.get(event).is_none() {
        hooks.insert(event, Item::ArrayOfTables(ArrayOfTables::new()));
    }
    let mut group = Table::new();
    group.insert("hooks", Item::Value(T::Array(hs)));
    hooks
        .get_mut(event)
        .and_then(Item::as_array_of_tables_mut)
        .ok_or(Error::UnsupportedCodexFormat { event })?
        .push(group);
    Ok(())
}
