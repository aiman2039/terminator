#![expect(
    clippy::arithmetic_side_effects,
    reason = "Fixture sizes are a few pixels."
)]

use std::num::NonZeroUsize;

use fallible_iterator::IteratorExt;
use wprs::buffer_pointer::BufferPointer;
use wprs::serialization::geometry::{Point, Rectangle, Size};
use wprs::serialization::wayland::{
    Buffer, BufferAssignment, BufferData, BufferFormat, BufferMetadata, Role, SubSurfaceState,
    SubsurfacePosition, SurfaceRequest, SurfaceRequestPayload, SurfaceState, WlSurfaceId,
};
use wprs::serialization::xdg_shell::{
    ToplevelRequest, ToplevelRequestPayload, XdgPopupId, XdgPopupState, XdgPositioner,
    XdgToplevelId, XdgToplevelState,
};
use wprs::serialization::{ClientId, RecvType, Request};
use wprs::sharding_compression::{CompressedShard, ShardingCompressor, ShardingDecompressor};

use super::input::{Input, toplevel_configure};
use super::state::State;
use crate::protocol::{ClientMessage, Framed, HostMessage, Rect, WindowId, WindowRole};

const CLIENT: ClientId = ClientId(7);
const RED: [u8; 4] = [0, 0, 255, 255];
const BLUE: [u8; 4] = [255, 0, 0, 255];

/// What wprsd sends in a raw-buffer message for these BGRA bytes.
fn filtered(bgra: &[u8]) -> Vec<u8> {
    let ptr = bgra.as_ptr();
    // SAFETY: `ptr` points at `bgra`, which outlives the pointer's use here.
    let data = unsafe { BufferPointer::new(&ptr, bgra.len()) };
    let mut compressor = ShardingCompressor::new(NonZeroUsize::MIN, 1).unwrap();
    let shards = wprs::filtering::filter_and_compress(data, &mut compressor);
    let mut decompressor = ShardingDecompressor::new(NonZeroUsize::MIN).unwrap();
    decompressor
        .decompress_to_owned(
            &shards.indices(),
            bgra.len(),
            shards
                .shards
                .into_iter()
                .map(Ok::<CompressedShard, anyhow::Error>)
                .transpose_into_fallible(),
        )
        .unwrap()
}

fn toplevel(title: &str) -> Role {
    Role::XdgToplevel(XdgToplevelState {
        id: XdgToplevelId(1),
        parent: None,
        title: Some(title.to_owned()),
        app_id: Some("xclock".to_owned()),
        decoration_mode: None,
        maximized: None,
        fullscreen: None,
    })
}

fn commit(
    id: u64,
    size: (i32, i32),
    role: Option<Role>,
    children: &[(u64, i32, i32)],
) -> RecvType<Request> {
    let mut z = children
        .iter()
        .map(|&(child, x, y)| SubsurfacePosition {
            id: WlSurfaceId(child),
            position: Point { x, y },
        })
        .collect::<Vec<_>>();
    z.push(SubsurfacePosition {
        id: WlSurfaceId(id),
        position: Point { x: 0, y: 0 },
    });
    let buffer = Buffer {
        metadata: BufferMetadata {
            width: size.0,
            height: size.1,
            stride: size.0 * 4,
            format: BufferFormat::Argb8888,
        },
        data: BufferData::External,
    };
    RecvType::Object(Request::Surface(SurfaceRequest {
        client: CLIENT,
        surface: WlSurfaceId(id),
        payload: SurfaceRequestPayload::Commit(SurfaceState {
            client: CLIENT,
            id: WlSurfaceId(id),
            buffer: Some(BufferAssignment::New(buffer)),
            role,
            buffer_scale: 1,
            buffer_transform: None,
            opaque_region: None,
            input_region: None,
            z_ordered_children: z,
            damage: None,
            output_ids: Vec::new(),
            viewport_state: None,
            xdg_surface_state: None,
        }),
    }))
}

/// One request, then everything it causes.
fn step(state: &mut State, request: RecvType<Request>) -> Vec<Framed<HostMessage>> {
    state.handle(request);
    state.flush(true)
}

fn send_pixels(state: &mut State, pixel: [u8; 4], count: usize) {
    state.handle(RecvType::RawBuffer(filtered(&pixel.repeat(count))));
}

fn with_damage(mut request: RecvType<Request>, rect: (i32, i32, i32, i32)) -> RecvType<Request> {
    if let RecvType::Object(Request::Surface(SurfaceRequest {
        payload: SurfaceRequestPayload::Commit(state),
        ..
    })) = &mut request
    {
        state.damage = Some(vec![Rectangle {
            loc: Point {
                x: rect.0,
                y: rect.1,
            },
            size: Size {
                w: rect.2,
                h: rect.3,
            },
        }]);
    }
    request
}

#[test]
fn damaged_commit_sends_only_its_rect() {
    let mut state = State::default();
    send_pixels(&mut state, RED, 3 * 2);
    step(&mut state, commit(1, (3, 2), Some(toplevel("t")), &[]));
    send_pixels(&mut state, BLUE, 3 * 2);
    let out = step(
        &mut state,
        with_damage(commit(1, (3, 2), Some(toplevel("t")), &[]), (1, 1, 2, 1)),
    );
    let HostMessage::Frame { damage, .. } = &out[0].message else {
        panic!("{out:?}");
    };
    assert_eq!(
        damage,
        &[Rect {
            x: 1,
            y: 1,
            width: 2,
            height: 1
        }]
    );
    assert_eq!(frame_pixels(&out[0]), BLUE.repeat(2));
}

#[test]
fn many_commits_before_a_flush_give_one_frame_with_the_union() {
    let mut state = State::default();
    send_pixels(&mut state, RED, 4 * 4);
    step(&mut state, commit(1, (4, 4), Some(toplevel("t")), &[]));
    for x in 0..3 {
        send_pixels(&mut state, BLUE, 4 * 4);
        state.handle(with_damage(
            commit(1, (4, 4), Some(toplevel("t")), &[]),
            (x, 0, 1, 1),
        ));
    }
    assert!(state.has_frames());
    assert!(state.flush(false).is_empty(), "frames wait for the writer");
    let out = state.flush(true);
    assert_eq!(out.len(), 1);
    assert!(matches!(
        &out[0].message,
        HostMessage::Frame { damage, .. } if damage == &[Rect { x: 0, y: 0, width: 3, height: 1 }]
    ));
    assert!(state.flush(true).is_empty());
}

#[test]
fn far_apart_changes_go_as_separate_rects_in_one_frame() {
    let mut state = State::default();
    send_pixels(&mut state, RED, 100 * 100);
    step(&mut state, commit(1, (100, 100), Some(toplevel("t")), &[]));
    for corner in [0, 99] {
        send_pixels(&mut state, BLUE, 100 * 100);
        state.handle(with_damage(
            commit(1, (100, 100), Some(toplevel("t")), &[]),
            (corner, corner, 1, 1),
        ));
    }
    let out = state.flush(true);
    assert_eq!(out.len(), 1);
    let HostMessage::Frame { damage, .. } = &out[0].message else {
        panic!("not a frame: {:?}", out[0].message);
    };
    assert_eq!(damage.len(), 2, "{damage:?}");
    assert_eq!(frame_pixels(&out[0]), BLUE.repeat(2));
}

#[test]
fn resize_sends_the_whole_window_even_with_small_damage() {
    let mut state = State::default();
    send_pixels(&mut state, RED, 2 * 2);
    step(&mut state, commit(1, (2, 2), Some(toplevel("t")), &[]));
    send_pixels(&mut state, BLUE, 3 * 3);
    let out = step(
        &mut state,
        with_damage(commit(1, (3, 3), Some(toplevel("t")), &[]), (0, 0, 1, 1)),
    );
    assert!(matches!(
        &out[0].message,
        HostMessage::Frame { width: 3, height: 3, damage, .. } if damage[0].width == 3 && damage[0].height == 3
    ));
}

fn frame_pixels(framed: &crate::protocol::Framed<HostMessage>) -> Vec<u8> {
    zstd::bulk::decompress(&framed.payload, 1 << 20).unwrap()
}

#[test]
fn first_toplevel_commit_opens_a_window_and_sends_its_pixels() {
    let mut state = State::default();
    send_pixels(&mut state, RED, 2 * 2);
    let out = step(&mut state, commit(1, (2, 2), Some(toplevel("clock")), &[]));
    assert_eq!(out.len(), 2);
    assert_eq!(
        out[0].message,
        HostMessage::WindowOpened {
            id: WindowId(1),
            role: WindowRole::Toplevel,
            title: "clock".into(),
            app_id: "xclock".into(),
        }
    );
    assert!(matches!(
        out[1].message,
        HostMessage::Frame {
            width: 2,
            height: 2,
            ..
        }
    ));
    assert_eq!(frame_pixels(&out[1]), RED.repeat(4));
}

#[test]
fn title_change_sends_window_changed_once() {
    let mut state = State::default();
    send_pixels(&mut state, RED, 1);
    step(&mut state, commit(1, (1, 1), Some(toplevel("a")), &[]));
    send_pixels(&mut state, RED, 1);
    let out = step(&mut state, commit(1, (1, 1), Some(toplevel("b")), &[]));
    assert!(matches!(&out[0].message, HostMessage::WindowChanged { title, .. } if title == "b"));
    send_pixels(&mut state, RED, 1);
    let out = step(&mut state, commit(1, (1, 1), Some(toplevel("b")), &[]));
    assert!(matches!(out[0].message, HostMessage::Frame { .. }));
}

#[test]
fn subsurface_commit_redraws_its_toplevel_with_the_child_on_top() {
    let mut state = State::default();
    send_pixels(&mut state, RED, 2 * 2);
    step(
        &mut state,
        commit(1, (2, 2), Some(toplevel("t")), &[(2, 1, 1)]),
    );
    send_pixels(&mut state, BLUE, 1);
    let sub = Role::SubSurface(SubSurfaceState {
        parent: WlSurfaceId(1),
        location: Point { x: 1, y: 1 },
        sync: false,
    });
    let out = step(&mut state, commit(2, (1, 1), Some(sub), &[]));
    let frame = out.last().unwrap();
    assert!(matches!(
        frame.message,
        HostMessage::Frame {
            id: WindowId(1),
            ..
        }
    ));
    assert_eq!(frame_pixels(frame), [RED, RED, RED, BLUE].concat());
}

#[test]
fn popup_opens_as_overlay_of_its_parent_window_at_the_anchor() {
    let mut state = State::default();
    send_pixels(&mut state, RED, 4);
    step(&mut state, commit(1, (2, 2), Some(toplevel("t")), &[]));
    let positioner = XdgPositioner {
        width: 1,
        height: 1,
        anchor_rect: Rectangle {
            loc: Point { x: 5, y: 6 },
            size: Size { w: 10, h: 4 },
        },
        anchor_edges: 6,
        gravity: 8,
        constraint_adjustment: 0,
        offset: Point { x: 0, y: 0 },
        reactive: false,
        parent_size: None,
        parent_configure: None,
    };
    let popup = Role::XdgPopup(XdgPopupState {
        id: XdgPopupId(9),
        parent_surface_id: WlSurfaceId(1),
        positioner,
        grab_requested: false,
    });
    send_pixels(&mut state, BLUE, 1);
    let out = step(&mut state, commit(3, (1, 1), Some(popup), &[]));
    assert_eq!(
        out[0].message,
        HostMessage::WindowOpened {
            id: WindowId(3),
            role: WindowRole::Popup {
                parent: WindowId(1),
                x: 5,
                y: 10
            },
            title: String::new(),
            app_id: String::new(),
        }
    );
}

#[test]
fn toplevel_destroy_closes_the_window_and_unknown_destroy_is_ignored() {
    let mut state = State::default();
    send_pixels(&mut state, RED, 1);
    step(&mut state, commit(1, (1, 1), Some(toplevel("t")), &[]));
    let destroy = |surface| {
        RecvType::Object(Request::Toplevel(ToplevelRequest {
            client: CLIENT,
            surface: WlSurfaceId(surface),
            payload: ToplevelRequestPayload::Destroyed,
        }))
    };
    let out = step(&mut state, destroy(1));
    assert_eq!(
        out[0].message,
        HostMessage::WindowClosed { id: WindowId(1) }
    );
    assert!(step(&mut state, destroy(1)).is_empty());
    assert!(state.surface_of(WindowId(1)).is_none());
}

#[test]
fn cursor_surface_pixels_go_out_with_the_hotspot_and_named_cursors_by_name() {
    use wprs::serialization::wayland::{ClientSurface, CursorImage, CursorImageStatus};
    let mut state = State::default();
    let set = |status| RecvType::Object(Request::CursorImage(CursorImage { serial: 1, status }));
    let out = step(
        &mut state,
        set(CursorImageStatus::Surface {
            client_surface: ClientSurface {
                client: CLIENT,
                surface: WlSurfaceId(5),
            },
            hotspot: Point { x: 1, y: 2 },
        }),
    );
    assert!(out.is_empty(), "no pixels yet");
    send_pixels(&mut state, BLUE, 4);
    let out = step(
        &mut state,
        commit(5, (2, 2), Some(Role::Cursor(Point { x: 1, y: 2 })), &[]),
    );
    assert!(matches!(
        out[0].message,
        HostMessage::Cursor {
            width: 2,
            height: 2,
            hotspot_x: 1,
            hotspot_y: 2,
            name: None
        }
    ));
    assert_eq!(out[0].payload, BLUE.repeat(4));
    let out = step(&mut state, set(CursorImageStatus::Named("text".into())));
    assert!(matches!(&out[0].message, HostMessage::Cursor { name: Some(n), .. } if n == "text"));
}

#[test]
fn window_id_is_the_surface_id_so_a_new_bridge_keeps_it() {
    for _ in 0..2 {
        let mut state = State::default();
        send_pixels(&mut state, RED, 1);
        let out = step(&mut state, commit(42, (1, 1), Some(toplevel("t")), &[]));
        assert!(matches!(
            out[0].message,
            HostMessage::WindowOpened {
                id: WindowId(42),
                ..
            }
        ));
    }
}

#[test]
fn commit_without_raw_pixels_keeps_the_window_closed_instead_of_panicking() {
    let mut state = State::default();
    assert!(step(&mut state, commit(1, (2, 2), Some(toplevel("t")), &[])).is_empty());
}

#[test]
fn client_disconnect_closes_all_its_windows() {
    let mut state = State::default();
    send_pixels(&mut state, RED, 1);
    step(&mut state, commit(1, (1, 1), Some(toplevel("t")), &[]));
    let out = step(
        &mut state,
        RecvType::Object(Request::ClientDisconnected(CLIENT)),
    );
    assert_eq!(
        out[0].message,
        HostMessage::WindowClosed { id: WindowId(1) }
    );
}

#[test]
fn configure_mirror_reads_back_as_the_wprs_type() {
    let configure = toplevel_configure(WlSurfaceId(42), (640, 480), 0b1111_1000).unwrap();
    assert_eq!(configure.surface_id, WlSurfaceId(42));
    assert_eq!(configure.new_size.w.map(|w| w.get()), Some(640));
    assert_eq!(configure.new_size.h.map(|h| h.get()), Some(480));
    let zero = toplevel_configure(WlSurfaceId(1), (0, 0), 0).unwrap();
    assert_eq!(zero.new_size.w, None);
}

#[test]
fn first_motion_enters_the_surface_and_focus_enters_the_keyboard_once() {
    let mut input = Input::default();
    let lookup = |id: WindowId| (id == WindowId(1)).then_some(WlSurfaceId(5));
    let motion = ClientMessage::PointerMotion {
        id: WindowId(1),
        x: 3.0,
        y: 4.0,
    };
    let first = input.events(&motion, lookup);
    let second = input.events(&motion, lookup);
    let count = |events: &[wprs::serialization::Event]| match events {
        [wprs::serialization::Event::PointerFrame(frame)] => frame.len(),
        _ => 0,
    };
    assert_eq!(count(&first), 2);
    assert_eq!(count(&second), 1);
    let focus = ClientMessage::Configure {
        id: WindowId(1),
        width: 100,
        height: 80,
        scale: 1,
        focused: true,
    };
    assert_eq!(input.events(&focus, lookup).len(), 2);
    assert_eq!(input.events(&focus, lookup).len(), 1);
    let unknown = ClientMessage::CloseRequested { id: WindowId(9) };
    assert!(input.events(&unknown, lookup).is_empty());
}

#[test]
fn remote_copy_asks_for_text_and_sends_it_to_the_client() {
    use super::clipboard::Clipboard;
    use wprs::serialization::wayland::{
        DataRequest, DataSource, DataSourceRequest, DataToTransfer, SourceMetadata,
    };
    let mut clipboard = Clipboard::default();
    let offer = DataRequest::SourceRequest(DataSourceRequest::SetSelection(
        DataSource::Selection,
        SourceMetadata {
            mime_types: vec!["image/png".into(), "UTF8_STRING".into()],
            dnd_actions: 0,
        },
    ));
    let (events, frames) = clipboard.on_wprsd(offer);
    assert_eq!(events.len(), 1);
    assert!(frames.is_empty());
    let data = DataRequest::TransferData(DataSource::Selection, DataToTransfer(b"hi".to_vec()));
    let (_, frames) = clipboard.on_wprsd(data);
    assert!(matches!(
        &frames[0].message,
        HostMessage::ClipboardData { mime_type } if mime_type == "UTF8_STRING"
    ));
    assert_eq!(frames[0].payload, b"hi");
}

#[test]
fn local_paste_offers_text_and_serves_it_when_an_app_asks() {
    use super::clipboard::Clipboard;
    use wprs::serialization::wayland::{DataDestinationRequest, DataRequest, DataSource};
    let mut clipboard = Clipboard::default();
    let ask = || {
        DataRequest::DestinationRequest(DataDestinationRequest::RequestDataTransfer(
            DataSource::Selection,
            "UTF8_STRING".into(),
        ))
    };
    assert!(
        clipboard.on_wprsd(ask()).0.is_empty(),
        "nothing offered yet"
    );
    let offer = ClientMessage::ClipboardOffer {
        mime_types: vec!["UTF8_STRING".into()],
    };
    assert_eq!(clipboard.on_client(&offer, b"local".to_vec()).len(), 1);
    let (events, _) = clipboard.on_wprsd(ask());
    assert!(matches!(
        &events[0],
        wprs::serialization::Event::Data(wprs::serialization::wayland::DataEvent::TransferData(
            DataSource::Selection,
            data
        )) if data.0 == b"local"
    ));
}
