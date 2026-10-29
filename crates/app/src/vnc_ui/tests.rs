use super::*;
use crate::vnc_ui::VncScreenInput;

#[test]
fn embedded_ports_cannot_override_the_visible_vnc_port() {
    for host in [
        "server:66",
        "server:5966",
        "server::66",
        "[::1]:66",
        "vnc://server:66",
    ] {
        assert!(connection_endpoint(host, "5966").is_err(), "{host}");
    }
    for host in ["server", "127.0.0.1", "::1", "2001:db8::66"] {
        assert_eq!(
            connection_endpoint(host, "5966").unwrap(),
            (host.into(), 5966)
        );
    }
    assert_eq!(
        connection_endpoint(" [::1] ", "5966").unwrap(),
        ("::1".into(), 5966)
    );
}

#[test]
fn vnc_display_and_port_conversion_rejects_invalid_values() {
    for (display, port) in [("0", "5900"), ("66", "5966"), ("59635", "65535")] {
        assert_eq!(port_for_display(display), port);
        assert_eq!(display_for_port(port), display);
    }
    for display in ["", "-1", "59636", "65536", "oops"] {
        assert!(port_for_display(display).is_empty());
    }
    for port in ["", "0", "66", "5899", "65536", "oops"] {
        assert!(display_for_port(port).is_empty());
    }
}

#[test]
fn vnc_forms_start_at_display_66_and_preserve_saved_ports() {
    let view = VncViewState::default();
    let mut sidebar = VncSidebarState::default();
    assert_eq!(view.draft_port, "5966");
    assert_eq!(view.draft_display, "66");
    assert_eq!(sidebar.port, "5966");
    assert_eq!(sidebar.display, "66");
    let profile = VncProfile {
        port: 80,
        id: "saved".into(),
        name: "Saved host".into(),
        host: "localhost".into(),
        username: String::new(),
        password: String::new(),
        shared: true,
        view_only: false,
        last_connected: 0,
        options: terminator_core::vnc::VncOptions::default(),
        credential_saved: false,
        labels: Vec::new(),
    };
    sidebar.begin_edit(&profile);
    assert_eq!(sidebar.port, "80");
    assert!(sidebar.display.is_empty());
    sidebar.begin_new();
    assert_eq!(sidebar.port, "5966");
    assert_eq!(sidebar.display, "66");
}

fn compressed(pixels: &[u8]) -> String {
    use flate2::{Compression, write::ZlibEncoder};
    use std::io::Write;
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(pixels).expect("compress pixels");
    B64.encode(encoder.finish().expect("finish compression"))
}

fn frame(region: Option<VncRegion>, pixels: &[u8]) -> VncFrame {
    VncFrame {
        cursor: None,
        monitors: Vec::new(),
        resize_supported: false,
        session: "s".into(),
        width: 2,
        height: 2,
        revision: 3,
        status: VncSessionStatus::Connected,
        error: None,
        zlib_rgba: compressed(pixels),
        region,
        clipboard: None,
        shm: None,
        cursor_serial: 0,
    }
}

#[test]
fn a_partial_frame_decodes_to_region_pixels_only() {
    let region = VncRegion {
        x: 1,
        y: 0,
        width: 1,
        height: 2,
    };
    let pixels = [0u8, 255, 0, 255, 0, 0, 255, 255];
    let decoded = decode_frame(frame(Some(region), &pixels), std::path::Path::new("/"))
        .expect("region frame decodes");
    assert_eq!(decoded.region, Some(region));
    assert_eq!(decoded.width, 2, "the record keeps the remote size");
    assert_eq!(decoded.height, 2);
    let image = decoded.image.expect("region pixels decode");
    assert_eq!(image.size, [1, 2], "the image covers the region only");
    assert_eq!(
        image.pixels[0],
        Color32::from_rgba_unmultiplied(0, 255, 0, 255)
    );
    assert_eq!(
        image.pixels[1],
        Color32::from_rgba_unmultiplied(0, 0, 255, 255)
    );
}

#[test]
fn a_frame_without_a_region_decodes_at_the_remote_size() {
    let pixels = [1u8, 2, 3, 255, 4, 5, 6, 255, 7, 8, 9, 255, 10, 11, 12, 255];
    let decoded =
        decode_frame(frame(None, &pixels), std::path::Path::new("/")).expect("full frame decodes");
    assert_eq!(decoded.region, None);
    let image = decoded.image.expect("full pixels decode");
    assert_eq!(image.size, [2, 2]);
}

#[test]
fn modifier_transitions_report_each_changed_key_once() {
    assert_eq!(
        modifier_transitions(0, 0b0011),
        vec![(0xFFE1, true), (0xFFE3, true)],
        "newly pressed modifiers go down in keysym order"
    );
    assert_eq!(
        modifier_transitions(0b0101, 0),
        vec![(0xFFE1, false), (0xFFE9, false)],
        "every released modifier goes up"
    );
    assert!(modifier_transitions(0b1010, 0b1010).is_empty());
    assert_eq!(
        modifier_transitions(0b0001, 0b1000),
        vec![(0xFFE1, false), (0xFFEB, true)],
        "a swap releases the old key before pressing the new one"
    );
}

#[test]
fn modifier_bits_map_command_to_the_chosen_key_without_double_counting_ctrl() {
    use terminator_core::vnc::CommandKey::{Control, Super};
    let none = egui::Modifiers::default();
    assert_eq!(modifier_bits(none, Control), 0);
    let shift_alt = egui::Modifiers {
        shift: true,
        alt: true,
        ..none
    };
    assert_eq!(modifier_bits(shift_alt, Control), 1 | 4);
    // macOS Command is `command` without `ctrl`.
    let command = egui::Modifiers {
        command: true,
        mac_cmd: true,
        ..none
    };
    assert_eq!(modifier_bits(command, Super), 8);
    assert_eq!(modifier_bits(command, Control), 2, "Cmd+A acts as Ctrl+A");
    // Elsewhere `command` mirrors Ctrl: one Control key, not two.
    let ctrl = egui::Modifiers {
        ctrl: true,
        command: true,
        ..none
    };
    assert_eq!(modifier_bits(ctrl, Super), 2);
}

#[test]
fn accelerator_keys_fall_back_to_keycodes_for_letters() {
    assert_eq!(accelerator_keysym(egui::Key::C), Some(u32::from('c')));
    assert_eq!(accelerator_keysym(egui::Key::V), Some(u32::from('v')));
    assert_eq!(accelerator_keysym(egui::Key::F5), None);
}
#[test]
fn focused_vnc_clipboard_commands_send_remote_shortcuts() {
    let ctx = egui::Context::default();
    let mut view = VncViewState {
        width: 800,
        height: 600,
        ..Default::default()
    };
    let texture = ctx.load_texture(
        "clipboard-test",
        ColorImage::filled([800, 600], Color32::BLACK),
        egui::TextureOptions::NEAREST,
    );
    let mut paint = |mut events: Vec<egui::Event>, modifiers: egui::Modifiers, view_only| {
        events.insert(0, egui::Event::ModifiersChanged(modifiers));
        let mut result = None;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 600.0),
                )),
                focused: true,
                events,
                ..Default::default()
            },
            |ui| {
                result = Some(vnc_screen(VncScreenInput {
                    ui,
                    view: &mut view,
                    texture: &texture,
                    view_only,
                    paste_shortcut: VncPasteShortcut::ShiftInsert,
                }));
            },
        );
        output.textures_delta.clear();
        result.unwrap().events
    };
    let none = egui::Modifiers::NONE;
    paint(vec![], none, false);
    let pos = egui::pos2(120.0, 170.0);
    paint(
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: none,
            },
        ],
        none,
        false,
    );
    paint(
        vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: none,
        }],
        none,
        false,
    );
    let command = egui::Modifiers {
        command: true,
        mac_cmd: true,
        ..none
    };
    let mut paste = paint(
        vec![egui::Event::Paste("gui-vnc-paste-ok".into())],
        command,
        false,
    );
    paste.extend(paint(vec![], none, false));
    assert!(
        paste
            .iter()
            .any(|event| matches!(event,VncInputEvent::CopyText(text) if text=="gui-vnc-paste-ok"))
    );
    assert!(paste.iter().any(|event| matches!(
        event,
        VncInputEvent::Key {
            keycode: 0xFF63,
            down: true
        }
    )));
    assert!(matches!(
        paste.last(),
        Some(VncInputEvent::Key {
            keycode: 0xFFEB,
            down: false
        })
    ));
    let mut copy = paint(vec![egui::Event::Copy], command, false);
    copy.extend(paint(vec![], none, false));
    assert!(
        !copy
            .iter()
            .any(|event| matches!(event, VncInputEvent::CopyText(_)))
    );
    assert!(copy.iter().any(|event| matches!(
        event,
        VncInputEvent::Key {
            keycode: 0xFF63,
            down: true
        }
    )));
    let blocked = paint(
        vec![
            egui::Event::Paste("blocked".into()),
            egui::Event::Copy,
            egui::Event::Cut,
        ],
        none,
        true,
    );
    assert!(blocked.is_empty());
    crate::console::Console::out(format_args!(
        "GUI_CLIPBOARD_EVENTS={}",
        serde_json::json!({"paste":paste,"copy":copy})
    ));
}

#[test]
fn clipboard_shortcuts_release_local_modifiers_and_use_the_selected_remote_chord() {
    for (shortcut, key, modifier) in [
        (VncPasteShortcut::ShiftInsert, 0xFF63, 0xFFE1),
        (VncPasteShortcut::ControlV, u32::from('v'), 0xFFE3),
        (VncPasteShortcut::ControlShiftV, u32::from('v'), 0xFFE3),
        (VncPasteShortcut::CommandV, u32::from('v'), 0xFFEB),
    ] {
        let events = remote_clipboard_events(&egui::Event::Paste("text".into()), shortcut, 8);
        assert!(matches!(
            events.first(),
            Some(VncInputEvent::Key {
                keycode: 0xFFEB,
                down: false
            })
        ));
        let key_index = events
            .iter()
            .position(|e| matches!(e,VncInputEvent::Key {keycode,down:true} if *keycode==key))
            .unwrap();
        let transfer = events
            .iter()
            .position(|e| matches!(e,VncInputEvent::CopyText(text) if text=="text"))
            .unwrap();
        let modifier_index = events
            .iter()
            .position(|e| matches!(e,VncInputEvent::Key {keycode,down:true} if *keycode==modifier))
            .unwrap();
        assert!(transfer < modifier_index && modifier_index < key_index);
        assert!(matches!(
            events.last(),
            Some(VncInputEvent::Key {
                keycode: 0xFFEB,
                down: true
            })
        ));
    }
}
#[test]
fn a_selected_monitor_maps_pointer_input_to_that_monitor() {
    let ctx = egui::Context::default();
    let mut view = VncViewState {
        width: 800,
        height: 600,
        selected_monitor: Some(2),
        monitors: vec![
            terminator_core::vnc::VncMonitor {
                id: 1,
                x: 0,
                y: 0,
                width: 400,
                height: 600,
            },
            terminator_core::vnc::VncMonitor {
                id: 2,
                x: 400,
                y: 0,
                width: 400,
                height: 600,
            },
        ],
        ..Default::default()
    };
    let texture = ctx.load_texture(
        "monitors",
        ColorImage::filled([800, 600], Color32::BLACK),
        egui::TextureOptions::NEAREST,
    );
    let mut events = Vec::new();
    for pressed in [false, true] {
        let pos = egui::pos2(400.0, 300.0);
        let input = if pressed {
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        } else {
            Vec::new()
        };
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 600.0),
                )),
                focused: true,
                events: input,
                ..Default::default()
            },
            |ui| {
                events = vnc_screen(VncScreenInput {
                    ui,
                    view: &mut view,
                    texture: &texture,
                    view_only: false,
                    paste_shortcut: VncPasteShortcut::ShiftInsert,
                })
                .events;
            },
        );
        output.textures_delta.clear();
    }
    assert!(events.iter().any(|e|matches!(e,VncInputEvent::Pointer{x,y,buttons:1} if (590..=610).contains(x)&&(290..=310).contains(y))));
}

#[test]
fn saving_a_form_connection_keeps_the_password_only_on_secure_daemons() {
    let request = PendingVncConnect {
        profile_id: None,
        host: "localhost".into(),
        port: 7939,
        username: String::new(),
        password: "fixture-password".into(),
        shared: true,
        view_only: false,
        label: None,
        save_profile: true,
        options: Default::default(),
    };
    let (profile, forget) = connect::connect_profile(&request, "dev", true);
    assert_eq!(profile.password, "fixture-password");
    assert!(!forget);
    let (profile, forget) = connect::connect_profile(&request, "dev", false);
    assert!(profile.password.is_empty());
    assert!(forget);
}

#[test]
fn address_bar_reads_displays_ports_and_bracketed_ipv6() {
    use super::address_book::parse_address;
    assert_eq!(
        parse_address("dc6.example.com:2039"),
        Ok(("dc6.example.com".into(), 7939))
    );
    assert_eq!(parse_address("vnc://host::5901"), Ok(("host".into(), 5901)));
    assert_eq!(parse_address("host"), Ok(("host".into(), 5900)));
    assert_eq!(parse_address("[::1]:1"), Ok(("::1".into(), 5901)));
    for bad in ["", ":1", "host:x", "host::0", "host:70000", "[::1"] {
        assert!(parse_address(bad).is_err(), "{bad}");
    }
}

#[test]
fn labels_are_trimmed_without_repeats() {
    assert_eq!(
        super::profiles::parse_labels(" lab, Team ,lab,, team "),
        vec!["lab".to_owned(), "Team".to_owned()]
    );
}

#[test]
fn dropped_files_upload_into_an_absolute_folder_only() {
    use super::transfer::remote_for_drop;
    let file = std::path::Path::new("/Users/me/report.pdf");
    assert_eq!(
        remote_for_drop("/home/me/Desktop/", file),
        Ok("/home/me/Desktop/report.pdf".to_owned())
    );
    assert_eq!(remote_for_drop("/", file), Ok("/report.pdf".to_owned()));
    assert!(remote_for_drop("", file).is_err());
    assert!(remote_for_drop("~/Desktop", file).is_err());
}

#[test]
fn ctrl_and_cmd_chords_reach_the_remote_as_ctrl_chords() {
    let ctx = egui::Context::default();
    let mut view = VncViewState {
        width: 800,
        height: 600,
        ..Default::default()
    };
    let texture = ctx.load_texture(
        "keys-test",
        ColorImage::filled([800, 600], Color32::BLACK),
        egui::TextureOptions::NEAREST,
    );
    let mut paint = |mut events: Vec<egui::Event>, modifiers: egui::Modifiers| {
        events.insert(0, egui::Event::ModifiersChanged(modifiers));
        let mut result = None;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 600.0),
                )),
                focused: true,
                events,
                ..Default::default()
            },
            |ui| {
                result = Some(vnc_screen(VncScreenInput {
                    ui,
                    view: &mut view,
                    texture: &texture,
                    view_only: false,
                    paste_shortcut: VncPasteShortcut::ShiftInsert,
                }));
            },
        );
        output.textures_delta.clear();
        result.map(|outcome| outcome.events).unwrap_or_default()
    };
    let none = egui::Modifiers::NONE;
    let pos = egui::pos2(120.0, 170.0);
    paint(vec![], none);
    // Click to focus the remote screen, as a user does.
    for pressed in [true, false] {
        paint(
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: none,
                },
            ],
            none,
        );
    }
    let key = |key, pressed, modifiers| egui::Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers,
    };
    let keys = |events: Vec<VncInputEvent>| -> Vec<(u32, bool)> {
        events
            .into_iter()
            .filter_map(|e| match e {
                VncInputEvent::Key { keycode, down } => Some((keycode, down)),
                _ => None,
            })
            .collect()
    };
    const CONTROL: u32 = 0xFFE3;
    let c = u32::from('c');
    let a = u32::from('a');
    // Physical Ctrl+C: Control down, c down/up, Control up.
    let ctrl = egui::Modifiers { ctrl: true, ..none };
    let mut sent = keys(paint(vec![key(egui::Key::C, true, ctrl)], ctrl));
    sent.extend(keys(paint(vec![key(egui::Key::C, false, ctrl)], ctrl)));
    sent.extend(keys(paint(vec![], none)));
    assert_eq!(
        sent,
        vec![(CONTROL, true), (c, true), (c, false), (CONTROL, false)]
    );
    // Mac Cmd+A with the default Command key: also a Ctrl chord.
    let cmd = egui::Modifiers {
        command: true,
        mac_cmd: true,
        ..none
    };
    let mut sent = keys(paint(vec![key(egui::Key::A, true, cmd)], cmd));
    sent.extend(keys(paint(vec![key(egui::Key::A, false, cmd)], cmd)));
    sent.extend(keys(paint(vec![], none)));
    assert_eq!(
        sent,
        vec![(CONTROL, true), (a, true), (a, false), (CONTROL, false)]
    );
}
