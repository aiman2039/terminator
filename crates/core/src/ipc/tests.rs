use super::*;

#[test]
fn worktree_requests_outlast_their_git_timeout() {
    let add = Request::WorktreeAdd {
        project: "p".into(),
        path: "/tmp/w".into(),
        branch: None,
        start: "HEAD".into(),
    };
    let remove = Request::WorktreeRemove {
        project: "p".into(),
    };
    // `worktrees::run` gives Git 30 s; the client must wait longer.
    for request in [add, remove] {
        assert!(request.response_deadline() > Duration::from_secs(30));
    }
    assert_eq!(
        Request::Snapshot.response_deadline(),
        Duration::from_secs(3)
    );
}

#[test]
fn keychain_requests_wait_for_a_permission_prompt() {
    // A macOS keychain prompt needs the user; 3 s reported a save that
    // then succeeded as an uncertain mutation.
    let forget = Request::VncForgetCredential { id: "p".into() };
    assert!(forget.response_deadline() >= Duration::from_mins(1));
}

#[test]
fn ssh_errors_survive_the_wire_as_typed_errors() {
    let error = crate::ssh::SshError {
        failure: crate::ssh::SshFailure::UnknownHostKey,
        message: "The SSH host key is not trusted yet".into(),
    };
    let wire = serde_json::to_vec(&Response::SshError(error.clone())).unwrap();
    let response: Response = serde_json::from_slice(&wire).unwrap();
    let checked = response.checked().unwrap_err();
    assert_eq!(crate::ssh::SshError::find(&checked), Some(&error));
    assert_eq!(
        crate::ErrorChain(&checked).to_string(),
        "The SSH host key is not trusted yet"
    );
}

#[test]
fn oversized_frame_rejected_before_allocation() {
    let mut b = u32::try_from(MAX_FRAME)
        .unwrap_or(u32::MAX)
        .saturating_add(1)
        .to_be_bytes()
        .as_slice()
        .to_vec();
    assert!(read_frame::<Request>(&mut b.as_slice()).is_err());
    b.clear();
}

#[test]
fn envelopes_remain_compatible_in_both_directions() {
    #[derive(Deserialize)]
    struct LegacyEnvelope {
        version: u32,
        auth: String,
        request: Request,
    }
    let hint = State::default().snapshot_hint();
    let new = Envelope {
        version: 1,
        auth: "fixture".into(),
        request: Request::Snapshot,
        snapshot_hint: Some(hint),
        snapshot_chunks: true,
        ssh_errors: true,
    };
    let legacy: LegacyEnvelope =
        serde_json::from_value(serde_json::to_value(&new).unwrap()).unwrap();
    assert_eq!(legacy.version, 1);
    assert_eq!(legacy.auth, "fixture");
    assert!(matches!(legacy.request, Request::Snapshot));
    let new: Envelope = serde_json::from_value(
        serde_json::json!({"version":1,"auth":"fixture","request":"Snapshot"}),
    )
    .unwrap();
    assert!(new.snapshot_hint.is_none());
    assert!(!new.snapshot_chunks);
    assert!(!new.ssh_errors);
}
