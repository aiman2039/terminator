//! One-time move of saved bindings whose default chord changed.
use super::parse;
use std::collections::BTreeMap;

/// Defaults that moved when Cmd+E became Recent files in every pane, as
/// `(action, old default)`. Each one moves to the current default.
pub const CMD_E_MOVES: &[(&str, &str)] = &[
    ("toggle_ide_mode", "command+E"),
    ("recent_files", "command+alt+E"),
];

/// Rename terminal left Cmd+Shift+R to Replace in Files.
pub const RENAME_TERMINAL_MOVES: &[(&str, &str)] = &[("rename_terminal", "command+shift+R")];

/// Every batch of moved defaults, oldest first.
const MOVED_DEFAULTS: [&[(&str, &str)]; 2] = [CMD_E_MOVES, RENAME_TERMINAL_MOVES];

/// Whether `value` is an old default chord of `action` (see
/// [`MOVED_DEFAULTS`]).
pub(super) fn old_default(action: &str, value: &str) -> bool {
    MOVED_DEFAULTS
        .iter()
        .flat_map(|moves| moves.iter())
        .any(|&(moved, old)| moved == action && parse(old) == parse(value))
}

/// Gives each Cmd+E action the map lacks its old default
/// ([`CMD_E_MOVES`]) when no other action holds that chord. A map without
/// the action predates it, when the old chord applied, so
/// [`migrate_moved_defaults`] then moves it like a saved one.
pub fn fill_moved_defaults(map: &mut BTreeMap<String, String>) {
    for &(action, old) in CMD_E_MOVES {
        let chord = parse(old);
        if !map.contains_key(action) && !map.values().any(|bound| parse(bound) == chord) {
            map.insert(action.to_owned(), old.to_owned());
        }
    }
}

/// Moves each binding of `moves` still at its old default to the current
/// default. A binding the user changed stays as it is. A move is undone
/// when its chord would clash with another action, so no custom chord is
/// taken. Returns whether `map` changed.
pub fn migrate_moved_defaults(map: &mut BTreeMap<String, String>, moves: &[(&str, &str)]) -> bool {
    let defaults = terminator_core::Settings::default().keybindings;
    let mut next = map.clone();
    let mut moved = Vec::new();
    for &(action, old) in moves {
        if let Some(new) = defaults.get(action)
            && new != old
            && next.get(action).is_some_and(|value| value == old)
        {
            next.insert(action.to_owned(), new.clone());
            moved.push((action, old));
        }
    }
    // Undoing one move can make another clash; repeat until none does.
    while let Some(index) = moved.iter().position(|(action, _)| clashes(&next, action)) {
        let (action, old) = moved.remove(index);
        next.insert(action.to_owned(), old.to_owned());
    }
    if moved.is_empty() {
        return false;
    }
    *map = next;
    true
}

/// Another action uses the same chord as `action`.
fn clashes(map: &BTreeMap<String, String>, action: &str) -> bool {
    let chord = map.get(action).and_then(|value| parse(value));
    chord.is_some()
        && map
            .iter()
            .any(|(other, value)| other != action && parse(value) == chord)
}
