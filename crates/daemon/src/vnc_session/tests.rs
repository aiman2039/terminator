use super::*;
use super::*;
use flate2::read::ZlibDecoder;
use std::io::Read;

#[test]
fn vnc_accepts_ssh_project_ids_and_rejects_unknown_projects() {
    let mut state = terminator_core::State::default();
    state
        .remote_projects
        .push(terminator_core::remote::RemoteProject {
            id: "ssh-project".into(),
            name: "Remote".into(),
            ssh: terminator_core::ssh::SshOptions {
                host: "fixture".into(),
                ..Default::default()
            },
            root: "/workspace".into(),
            layout: serde_json::Value::Null,
            group: None,
            forwards: Vec::new(),
        });
    assert!(state.projects.is_empty());
    assert!(validate_project(&state, "ssh-project").is_ok());
    assert!(validate_project(&state, "").is_ok());
    assert!(validate_project(&state, "unknown").is_err());
}

fn runtime() -> VncRuntime {
    let (input_tx, _input_rx) = mpsc::channel(256);
    VncRuntime::new(false, input_tx)
}

fn pixels(encoded: &str) -> Vec<u8> {
    let compressed = B64.decode(encoded).expect("base64 payload");
    let mut out = Vec::new();
    ZlibDecoder::new(compressed.as_slice())
        .read_to_end(&mut out)
        .expect("zlib payload");
    out
}

fn one_pixel(runtime: &mut VncRuntime, x: u16, y: u16, rgba: [u8; 4]) {
    runtime.blit_rgba(
        Rect {
            x,
            y,
            width: 1,
            height: 1,
        },
        &rgba,
    );
}

#[test]
fn a_client_at_the_encoded_revision_takes_the_changed_region_only() {
    let mut rt = runtime();
    rt.resize(4, 4);
    let full = rt.encode_frame("s", 0, true).expect("full frame");
    assert_eq!(full.region, None, "nothing encoded yet: the whole frame");
    assert_eq!(pixels(&full.zlib_rgba).len(), 4 * 4 * 4);
    assert_eq!((full.width, full.height), (4, 4));

    one_pixel(&mut rt, 1, 2, [9, 8, 7, 255]);
    let partial = rt
        .encode_frame("s", full.revision, true)
        .expect("partial frame");
    assert_eq!(
        partial.region,
        Some(VncRegion {
            x: 1,
            y: 2,
            width: 1,
            height: 1
        })
    );
    assert_eq!(pixels(&partial.zlib_rgba), vec![9, 8, 7, 255]);
    assert_eq!(
        (partial.width, partial.height),
        (4, 4),
        "the record keeps describing the whole remote screen"
    );

    // The region was consumed: a poll with no new damage answers with
    // the full frame rather than repeating the one rectangle.
    let next = rt
        .encode_frame("s", partial.revision, true)
        .expect("next frame");
    assert_eq!(next.region, None);
}

#[test]
fn a_client_behind_the_encoded_revision_gets_the_whole_frame() {
    let mut rt = runtime();
    rt.resize(2, 2);
    let full = rt.encode_frame("s", 0, true).expect("full frame");
    one_pixel(&mut rt, 0, 0, [1, 2, 3, 255]);
    // The client never took the last full frame, so a rectangle cut
    // from it would land on the wrong texture.
    let stale = rt
        .encode_frame("s", full.revision.saturating_sub(1), true)
        .expect("stale frame");
    assert_eq!(stale.region, None);
    assert_eq!(pixels(&stale.zlib_rgba).len(), 2 * 2 * 4);
}

#[test]
fn a_client_that_never_asked_for_regions_gets_the_whole_frame() {
    let mut rt = runtime();
    rt.resize(2, 2);
    let full = rt.encode_frame("s", 0, false).expect("full frame");
    one_pixel(&mut rt, 1, 1, [4, 5, 6, 255]);
    let frame = rt
        .encode_frame("s", full.revision, false)
        .expect("legacy frame");
    assert_eq!(frame.region, None);
    assert_eq!(pixels(&frame.zlib_rgba).len(), 2 * 2 * 4);
}

#[test]
fn a_resize_invalidates_incremental_updates() {
    let mut rt = runtime();
    rt.resize(2, 2);
    let full = rt.encode_frame("s", 0, true).expect("full frame");
    rt.resize(3, 3);
    let frame = rt
        .encode_frame("s", full.revision, true)
        .expect("resized frame");
    assert_eq!(frame.region, None, "every pixel changed");
    assert_eq!(pixels(&frame.zlib_rgba).len(), 3 * 3 * 4);
}

#[test]
fn the_server_refresh_follows_the_last_client_poll() {
    let mut rt = runtime();
    assert!(!rt.polled_recently(), "no viewer has asked yet");
    rt.last_poll = Some(Instant::now());
    assert!(
        rt.polled_recently(),
        "a viewer that just polled keeps the refresh running"
    );
    rt.last_poll = Some(
        Instant::now()
            .checked_sub(CLIENT_ACTIVE + Duration::from_secs(1))
            .expect("instant inside the clock range"),
    );
    assert!(
        !rt.polled_recently(),
        "a viewer that stopped asking costs no traffic"
    );
}
