use super::*;

pub fn write_frame<T: Serialize>(writer: &mut impl Write, value: &T) -> Result<(), FrameError> {
    let data = serde_json::to_vec(value)?;
    let too_large = || FrameError::TooLarge { bytes: data.len() };
    if data.len() > MAX_FRAME {
        return Err(too_large());
    }
    let len = u32::try_from(data.len()).map_err(|_| too_large())?;
    writer.write_all(&len.to_be_bytes())?;
    writer.write_all(&data)?;
    writer.flush()?;
    Ok(())
}
pub fn read_frame<T: DeserializeOwned>(reader: &mut impl Read) -> Result<T, FrameError> {
    let mut header = [0; 4];
    reader.read_exact(&mut header)?;
    let n = u32::from_be_bytes(header) as usize;
    if n > MAX_FRAME {
        return Err(FrameError::TooLarge { bytes: n });
    }
    let mut bytes = vec![0; n];
    reader.read_exact(&mut bytes)?;
    Ok(serde_json::from_slice(&bytes)?)
}
pub fn connect(
    paths: &Paths,
    request: Request,
    token: Option<String>,
) -> Result<transport::Stream> {
    connect_hint(ConnectHintInput {
        paths,
        request,
        token,
        snapshot_hint: None,
    })
}
struct ConnectHintInput<'a> {
    paths: &'a Paths,
    request: Request,
    token: Option<String>,
    snapshot_hint: Option<SnapshotHint>,
}

fn connect_hint(input: ConnectHintInput<'_>) -> Result<transport::Stream> {
    let ConnectHintInput {
        paths,
        request,
        token,
        snapshot_hint,
    } = input;
    let routed = generations::owner_for(paths, &request)?;
    let paths = &routed;
    let mut s = transport::Stream::connect_ipc(&paths.socket())?;
    s.set_timeouts(request.response_deadline(), Duration::from_secs(3))?;
    write_frame(
        &mut s,
        &Envelope {
            version: PROTOCOL_VERSION,
            auth: match token {
                Some(t) => t,
                None => paths.token()?,
            },
            snapshot_chunks: matches!(request, Request::Snapshot | Request::VncFrame { .. }),
            ssh_errors: true,
            request,
            snapshot_hint,
        },
    )?;
    Ok(s)
}
pub fn rpc(paths: &Paths, request: Request) -> Result<Response> {
    if matches!(request, Request::Snapshot) && generations::exists(paths) {
        return Ok(Response::State(Box::new(generations::snapshot(paths)?)));
    }
    if generations::exists(paths)
        && matches!(
            &request,
            Request::Heartbeat { .. } | Request::ClearHistory { session: None }
        )
    {
        fanout_owners(paths, &request)?;
        return Ok(Response::Ok);
    }
    if generations::exists(paths)
        && let Some(owner) = archived_generation(paths, &request)?
        && !matches!(
            request,
            Request::Archived { .. }
                | Request::Shutdown
                | Request::ShutdownIfIdle
                | Request::Snapshot
        )
    {
        return rpc(
            paths,
            Request::Archived {
                generation: owner,
                request: Box::new(request),
            },
        );
    }
    for _ in 0..3 {
        let mut stream = connect(paths, request.clone(), None)?;
        let response = snapshot::read_response(&mut stream)?;
        if let Response::Redirect { generation } = &response
            && generations::exists(paths)
            && redirect_allowed(&request)
        {
            if !generations::Catalog::open(paths)?
                .generations()?
                .iter()
                .any(|g| &g.id == generation)
            {
                return Err(IpcError::UnregisteredRedirect {
                    generation: generation.clone(),
                });
            }
            continue;
        }
        return response.checked();
    }
    Err(IpcError::Unstable {
        request: request.name(),
    })
}

pub(crate) fn redirect_allowed(request: &Request) -> bool {
    matches!(
        request,
        Request::Create { .. }
            | Request::CreateReview { .. }
            | Request::AddProject { .. }
            | Request::AddRemoteProject { .. }
            | Request::RemoveRemoteProject { .. }
            | Request::SetRemoteProjectGroup { .. }
            | Request::SetRemoteProjectForwards { .. }
            | Request::SaveLayout { .. }
            | Request::SelectProject { .. }
            | Request::Settings(_)
            | Request::WorktreeAdd { .. }
            | Request::WorktreeRemove { .. }
            | Request::VncProfileSave { .. }
            | Request::VncProfileRemove { .. }
            | Request::VncForgetCredential { .. }
            | Request::VncConnect { .. }
    )
}

fn fanout_owners(paths: &Paths, request: &Request) -> Result<()> {
    let catalog = generations::Catalog::open(paths)?;
    let active = catalog.active()?;
    for owner in catalog.generations()? {
        if owner.status == generations::Status::Prepared {
            continue;
        }
        if generations::historical(&owner, active.as_deref()) {
            if matches!(request, Request::ClearHistory { .. }) {
                rpc(
                    paths,
                    Request::Archived {
                        generation: owner.id,
                        request: Box::new(request.clone()),
                    },
                )?;
            }
        } else {
            let result = rpc(&owner.paths(), request.clone());
            // Old owners may outlive their temporary runtime files. Their
            // health is reported by Snapshot; a best-effort focus heartbeat
            // must still reach the active owner and other healthy owners.
            // Explicit history mutations continue to report every failure.
            if !matches!(request, Request::Heartbeat { .. })
                || active.as_deref() == Some(owner.id.as_str())
            {
                result?;
            }
        }
    }
    Ok(())
}

pub(crate) fn archived_generation(paths: &Paths, request: &Request) -> Result<Option<String>> {
    let catalog = generations::Catalog::open(paths)?;
    let active = catalog.active()?;
    let target = generations::owner_for(paths, request)?;
    Ok(catalog
        .generations()?
        .into_iter()
        .find(|g| g.data == target.data && generations::historical(g, active.as_deref()))
        .map(|g| g.id))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotHint {
    pub generation: String,
    pub revision: u64,
    #[serde(default)]
    pub catalog_revision: u64,
    #[serde(default)]
    pub owner_revisions: Vec<(String, u64, Option<String>, generations::Status)>,
}
impl State {
    #[must_use]
    pub fn snapshot_hint(&self) -> SnapshotHint {
        SnapshotHint {
            generation: self.generation.clone(),
            revision: self.revision,
            catalog_revision: self.catalog_revision,
            owner_revisions: self
                .generations
                .iter()
                .map(|g| {
                    (
                        g.owner.id.clone(),
                        g.revision,
                        g.error.clone(),
                        g.owner.status.clone(),
                    )
                })
                .collect(),
        }
    }
}
pub fn conditional_snapshot(paths: &Paths, hint: Option<SnapshotHint>) -> Result<Response> {
    if generations::exists(paths) {
        let state = generations::snapshot(paths)?;
        return if hint.as_ref() == Some(&state.snapshot_hint()) {
            Ok(Response::Unchanged)
        } else {
            Ok(Response::State(Box::new(state)))
        };
    }
    let mut stream = connect_hint(ConnectHintInput {
        paths,
        request: Request::Snapshot,
        token: None,
        snapshot_hint: hint,
    })?;
    snapshot::read_response(&mut stream)?.checked()
}

/// Docking libraries use infinite rectangles before their first layout pass. JSON
/// encodes those as null; replace only coordinate placeholders, never optional IDs.
#[must_use]
pub fn sanitize_layout(mut value: serde_json::Value) -> serde_json::Value {
    fn walk(value: &mut serde_json::Value) {
        match value {
            serde_json::Value::Object(map) => {
                for (key, value) in map {
                    if (key == "x" || key == "y") && value.is_null() {
                        *value = serde_json::json!(0.0);
                    } else {
                        walk(value);
                    }
                }
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    walk(item);
                }
            }
            _ => {}
        }
    }
    walk(&mut value);
    value
}
