use super::*;
use std::io::Cursor;

fn round_trip<M>(message: &M, payload: &[u8]) -> Framed<M>
where
    M: Serialize + for<'de> Deserialize<'de>,
{
    let mut bytes = Vec::new();
    write_frame(&mut bytes, message, payload).unwrap_or_else(|e| panic!("write: {e}"));
    read_frame(&mut Cursor::new(bytes)).unwrap_or_else(|e| panic!("read: {e}"))
}

#[test]
fn frame_with_pixels_round_trips() {
    let message = HostMessage::Frame {
        id: WindowId(7),
        width: 640,
        height: 480,
        damage: vec![Rect {
            x: 1,
            y: 2,
            width: 3,
            height: 4,
        }],
        encoding: FrameEncoding::ZstdBgra,
    };
    let framed = round_trip(&message, &[1, 2, 3, 255]);
    assert_eq!(framed.message, message);
    assert_eq!(framed.payload, vec![1, 2, 3, 255]);
}

#[test]
fn popup_role_keeps_its_parent_and_offset() {
    let message = HostMessage::WindowOpened {
        id: WindowId(2),
        role: WindowRole::Popup {
            parent: WindowId(1),
            x: 10,
            y: -4,
        },
        title: String::new(),
        app_id: "gvim".into(),
    };
    assert_eq!(round_trip(&message, &[]).message, message);
}

#[test]
fn several_frames_read_in_order_from_one_stream() {
    let mut bytes = Vec::new();
    for id in 0..3 {
        write_frame(
            &mut bytes,
            &ClientMessage::PointerLeave { id: WindowId(id) },
            &[],
        )
        .unwrap_or_else(|e| panic!("{e}"));
    }
    let mut input = Cursor::new(bytes);
    for id in 0..3 {
        let framed: Framed<ClientMessage> =
            read_frame(&mut input).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            framed.message,
            ClientMessage::PointerLeave { id: WindowId(id) }
        );
    }
    assert!(matches!(
        read_frame::<ClientMessage>(&mut input),
        Err(FrameError::Closed)
    ));
}

#[test]
fn stream_cut_inside_a_frame_is_an_error_not_a_clean_close() {
    let mut bytes = Vec::new();
    write_frame(
        &mut bytes,
        &ClientMessage::PointerLeave { id: WindowId(1) },
        &[9; 16],
    )
    .unwrap_or_else(|e| panic!("{e}"));
    bytes.truncate(bytes.len().saturating_sub(4));
    assert!(matches!(
        read_frame::<ClientMessage>(&mut Cursor::new(bytes)),
        Err(FrameError::Io(_))
    ));
}

#[test]
fn oversized_header_length_is_refused_before_allocating() {
    let mut bytes = u32::MAX.to_be_bytes().to_vec();
    bytes.extend_from_slice(b"{}");
    assert!(matches!(
        read_frame::<ClientMessage>(&mut Cursor::new(bytes)),
        Err(FrameError::HeaderTooLarge(_))
    ));
}

#[test]
fn unknown_message_type_is_a_json_error() {
    let header = br#"{"type":"teleport"}"#;
    let mut bytes = u32::try_from(header.len())
        .unwrap_or_else(|e| panic!("{e}"))
        .to_be_bytes()
        .to_vec();
    bytes.extend_from_slice(header);
    bytes.extend_from_slice(&0_u32.to_be_bytes());
    assert!(matches!(
        read_frame::<HostMessage>(&mut Cursor::new(bytes)),
        Err(FrameError::Json(_))
    ));
}

#[test]
fn version_mismatch_names_the_side_to_update() {
    assert!(version_mismatch_message(0, "0.150.0").contains("Update Terminator on the host"));
    assert!(
        version_mismatch_message(PROTOCOL_VERSION + 1, "0.200.0")
            .contains("Update Terminator on this computer")
    );
}

#[test]
fn error_kind_uses_stable_snake_case_names() {
    let json = serde_json::to_string(&HostMessage::Error {
        kind: ErrorKind::WprsdStopped,
        message: "exit 139".into(),
    })
    .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        json,
        r#"{"type":"error","kind":"wprsd_stopped","message":"exit 139"}"#
    );
}
