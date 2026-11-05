//! One-time move of saved bindings whose default chord changed.
use super::parse;
use std::collections::BTreeMap;

/// Defaults that moved when Cmd+E became Recent files in every pane, as
/// `(action, old default)`. Each one moves to the current default.
const MOVED_DEFAULTS: &[(&str, &str)] = &[
    ("toggle_ide_mode", "command+E"),
    ("recent_files", "command+alt+E"),
];

/// Gives each moved action the map lacks its old default
/// ([`MOVED_DEFAULTS`]) when no other action holds that chord. A map
/// without the action predates it, when the old chord applied, so
/// [`migrate_moved_defaults`] then moves it like a saved one.
pub fn fill_moved_defaults(map: &mut BTreeMap<String, String>) {
    for &(action, old) in MOVED_DEFAULTS {
        let chord = parse(old);
        if !map.contains_key(action) && !map.values().any(|bound| parse(bound) == chord) {
            map.insert(action.to_owned(), old.to_owned());
        }
    }
}

/// Moves each binding still at its old default ([`MOVED_DEFAULTS`]) to the
/// current default. A binding the user changed stays as it is. A move is
/// undone when its chord would clash with another action, so no custom
/// chord is taken. Returns whether `map` changed.
pub fn migrate_moved_defaults(map: &mut BTreeMap<String, String>) -> bool {
    let defaults = terminator_core::Settings::default().keybindings;
    let mut next = map.clone();
    let mut moved = Vec::new();
    for &(action, old) in MOVED_DEFAULTS {
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
