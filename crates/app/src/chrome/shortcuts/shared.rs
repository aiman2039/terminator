//! Chords two actions may share by keyboard focus: a terminal with the
//! keyboard keeps the chord, and the other action takes it everywhere
//! else. `JetBrains` Find in Path is Cmd+Shift+F; on Linux, stored
//! `command` already means Ctrl+Shift, so it is Find in terminal's
//! Ctrl+Shift+F there. Rename terminal and Replace in Files (Cmd+Shift+R)
//! no longer share a default, but a saved keymap may still bind both to
//! one chord. Each action can still be rebound alone; then both chords
//! work everywhere.
use super::{Action, binding, parse};
use std::collections::BTreeMap;

/// `(terminal action, action everywhere else)` pairs that may share a
/// chord.
const SHARED: &[(Action, Action)] = &[
    (Action::RenameTerminal, Action::ReplaceInFiles),
    (Action::FindInTerminal, Action::FindInFiles),
];

/// True when the action ids `a` and `b` are a shared pair, in either
/// order.
pub(super) fn partners(a: &str, b: &str) -> bool {
    SHARED.iter().any(|(terminal, elsewhere)| {
        (terminal.id() == a && elsewhere.id() == b) || (terminal.id() == b && elsewhere.id() == a)
    })
}

/// Drop the second action of each shared pair from `actions` (the ids
/// bound to one chord), so the pair is no conflict.
pub(super) fn drop_partners(actions: &mut Vec<String>) {
    for (terminal, elsewhere) in SHARED {
        if actions.iter().any(|action| action == terminal.id()) {
            actions.retain(|action| action != elsewhere.id());
        }
    }
}

/// True when `action` must let its chord pass this frame: it shares the
/// chord with its partner, and the keyboard is on the partner's side (a
/// terminal for the terminal action, anywhere else for the other).
pub fn yields(map: &BTreeMap<String, String>, action: Action, terminal_focus: bool) -> bool {
    SHARED.iter().any(|&(terminal, elsewhere)| {
        let other_side =
            (action == terminal && !terminal_focus) || (action == elsewhere && terminal_focus);
        other_side && same_chord(map, terminal, elsewhere)
    })
}

/// Both actions are bound, to the same keys.
fn same_chord(map: &BTreeMap<String, String>, a: Action, b: Action) -> bool {
    let chord = parse(&binding(map, a.id()));
    chord.is_some() && chord == parse(&binding(map, b.id()))
}

/// Give each action the map lacks its default chord. A default chord the
/// map already gives another action stays unbound: a new action never
/// takes a chord the user picked. A shared partner on its own default
/// chord, or on an old default the keymap migration moves, shares it
/// instead.
pub fn fill_defaults(map: &mut BTreeMap<String, String>) {
    let defaults = terminator_core::Settings::default().keybindings;
    for (action, value) in &defaults {
        if map.contains_key(action) {
            continue;
        }
        let chord = parse(value);
        let taken = chord.is_some()
            && map.iter().any(|(other, bound)| {
                let sharing = partners(action, other)
                    && (defaults
                        .get(other)
                        .is_some_and(|own| parse(own) == parse(bound))
                        || super::migrate::old_default(other, bound));
                !sharing && parse(bound) == chord
            });
        let value = if taken { String::new() } else { value.clone() };
        map.insert(action.clone(), value);
    }
}
