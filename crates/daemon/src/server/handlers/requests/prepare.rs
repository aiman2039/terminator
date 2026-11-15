use anyhow::{Context, Result};
use std::sync::Arc;
use std::time::Duration;
use terminator_core::*;

use super::super::super::shared::{Shared, relock};

pub(super) struct PreparedRequest {
    pub coordination: Option<std::fs::File>,
    pub shared_write: bool,
    pub early: Option<Response>,
}

/// How a request interacts with the shared generation catalog.
struct RequestClass {
    /// Writes workspace state shared by every generation.
    shared_write: bool,
    /// Only the active generation may run it.
    admission: bool,
    /// Runs under the cross-generation coordination lock.
    coordinated: bool,
}

impl RequestClass {
    fn of(request: &Request) -> Self {
        let shared_write = matches!(
            request,
            Request::AddProject { .. }
                | Request::AddRemoteProject { .. }
                | Request::RemoveRemoteProject { .. }
                | Request::SetRemoteProjectGroup { .. }
                | Request::UpdateRemoteProject { .. }
                | Request::SetRemoteProjectForwards { .. }
                | Request::SaveLayout { .. }
                | Request::SelectProject { .. }
                | Request::SetInitCommand { .. }
                | Request::Settings(_)
                | Request::WorktreeAdd { .. }
                | Request::WorktreeRemove { .. }
                | Request::VncProfileSave { .. }
                | Request::VncProfileRemove { .. }
                | Request::VncForgetCredential { .. }
        );
        let admission = shared_write
            || matches!(
                request,
                Request::Create { .. }
                    | Request::CreateReview { .. }
                    | Request::VncConnect { .. }
                    | Request::RemoteAppsConnect { .. }
            );
        let coordinated = admission
            || matches!(
                request,
                Request::Cwd { .. }
                    | Request::Shutdown
                    | Request::ShutdownIfIdle
                    | Request::RetireIfDraining
            );
        Self {
            shared_write,
            admission,
            coordinated,
        }
    }

    /// Only admission checks and coordinated refreshes read the catalog.
    fn reads_catalog(&self) -> bool {
        self.admission || self.coordinated
    }
}

impl Shared {
    pub(super) fn prepare_request(self: &Arc<Self>, request: &Request) -> Result<PreparedRequest> {
        let class = RequestClass::of(request);
        let coordination = self.coordinate(class.coordinated)?;
        let early = self.admit_and_refresh(&class)?;
        Ok(PreparedRequest {
            coordination,
            shared_write: class.shared_write,
            early,
        })
    }

    /// Coordinates a shared workspace write that ran its slow part
    /// without the lock: admission and catalog refresh run again.
    pub(super) fn prepare_shared_write(self: &Arc<Self>) -> Result<PreparedRequest> {
        let class = RequestClass {
            shared_write: true,
            admission: true,
            coordinated: true,
        };
        let coordination = self.coordinate(class.coordinated)?;
        let early = self.admit_and_refresh(&class)?;
        Ok(PreparedRequest {
            coordination,
            shared_write: class.shared_write,
            early,
        })
    }

    fn coordinate(&self, coordinated: bool) -> Result<Option<std::fs::File>> {
        if !coordinated {
            return Ok(None);
        }
        // Bounded wait: the client answers inside a 3 s deadline, so a
        // contended lock must fail fast (certain, retryable) rather than
        // hang the handler past the deadline (uncertain mutation).
        self.catalog_paths
            .as_ref()
            .map(|paths| generations::coordinate_timeout(paths, Duration::from_secs(2)))
            .transpose()
            .map_err(Into::into)
    }

    /// Rejects requests this generation may not run, then refreshes
    /// coordinated requests from the shared catalog. Requests that need
    /// neither (hooks, Heartbeat, History, VNC frames…) touch no SQLite.
    /// The catalog I/O runs outside the state lock; only applying the
    /// result takes it. Coordination serializes catalog writers, so the
    /// read cannot be overtaken by another write before it is applied.
    fn admit_and_refresh(self: &Arc<Self>, class: &RequestClass) -> Result<Option<Response>> {
        if !class.reads_catalog() {
            return Ok(None);
        }
        let Some(paths) = &self.catalog_paths else {
            return Ok(None);
        };
        let catalog = generations::Catalog::open(paths)?;
        let generation = relock(&self.state).generation.clone();
        let active = catalog.active()?;
        if let Some(early) = Self::admission_reply(&catalog, active.as_deref(), &generation, class)?
        {
            return Ok(Some(early));
        }
        if class.coordinated {
            let workspace = catalog.workspace()?;
            let legacy = workspace
                .vnc_profiles
                .iter()
                .any(|p| !p.password.is_empty());
            workspace.apply(&mut relock(&self.state));
            if legacy && active.as_deref() == Some(generation.as_str()) {
                // Keychain access may wait on a user prompt: never inside this
                // short-deadline request or under the state lock.
                crate::vnc_session::credentials::finish_activation(Arc::downgrade(self));
            }
        }
        Ok(None)
    }

    fn admission_reply(
        catalog: &generations::Catalog,
        active: Option<&str>,
        generation: &str,
        class: &RequestClass,
    ) -> Result<Option<Response>> {
        if !class.admission {
            return Ok(None);
        }
        if active != Some(generation) {
            return Ok(Some(Response::Redirect {
                generation: active.context("No active generation")?.to_owned(),
            }));
        }
        if !class.shared_write && !catalog.admitted(generation)? {
            return Ok(Some(Response::Error(
                "Creation rejected before execution: recovery is in progress".into(),
            )));
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn high_rate_requests_do_not_open_the_catalog() {
        for request in [
            Request::Heartbeat { focused: true },
            Request::Focus {
                session: "s".into(),
            },
            Request::History {
                session: "s".into(),
            },
            Request::Snapshot,
        ] {
            assert!(!RequestClass::of(&request).reads_catalog(), "{request:?}");
        }
    }

    #[test]
    fn creation_and_cwd_still_refresh_from_the_catalog() {
        for request in [
            Request::Cwd {
                session: "s".into(),
                path: "/".into(),
            },
            Request::Shutdown,
            Request::SelectProject {
                project: "p".into(),
            },
        ] {
            assert!(RequestClass::of(&request).reads_catalog(), "{request:?}");
        }
    }
}
