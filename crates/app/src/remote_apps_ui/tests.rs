use eframe::egui::{Key, Modifiers};

use super::keys::{LEFT_CTRL, LEFT_SHIFT, evdev, modifier_changes};
use super::tab_title;

#[test]
fn keys_map_to_us_evdev_positions() {
    assert_eq!(evdev(Key::A), Some(30));
    assert_eq!(evdev(Key::Q), Some(16));
    assert_eq!(evdev(Key::Num1), Some(2));
    assert_eq!(evdev(Key::Enter), Some(28));
    assert_eq!(evdev(Key::ArrowLeft), Some(105));
    assert_eq!(evdev(Key::F12), Some(88));
    assert_eq!(evdev(Key::BrowserBack), None);
}

#[test]
fn command_acts_as_control_and_only_changes_are_sent() {
    let cmd = Modifiers {
        mac_cmd: true,
        command: true,
        ..Modifiers::NONE
    };
    assert_eq!(modifier_changes(Modifiers::NONE, cmd), [(LEFT_CTRL, true)]);
    assert!(modifier_changes(cmd, cmd).is_empty());
    let shift = Modifiers::SHIFT;
    assert_eq!(
        modifier_changes(shift, Modifiers::NONE),
        [(LEFT_SHIFT, false)]
    );
}

#[test]
fn untitled_windows_get_a_readable_tab_title() {
    assert_eq!(tab_title(""), "Remote app");
    assert_eq!(tab_title("xclock"), "xclock");
}
