use anyhow::{Context, Result, ensure};
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
                | Request::SetRemoteProjectForwards { .. }
                | Request::SaveLayout { .. }
                | Request::SelectProject { .. }
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
                Request::Create { .. } | Request::CreateReview { .. } | Request::VncConnect { .. }
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
}

impl Shared {
    pub(super) fn prepare_request(&self, request: &Request) -> Result<PreparedRequest> {
        let class = RequestClass::of(request);
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
    /// coordinated requests from the shared catalog.
    fn admit_and_refresh(&self, class: &RequestClass) -> Result<Option<Response>> {
        let Some(paths) = &self.catalog_paths else {
            return Ok(None);
        };
        let catalog = generations::Catalog::open(paths)?;
        let mut state = relock(&self.state);
        if let Some(early) = Self::admission_reply(&catalog, &state, class)? {
            return Ok(Some(early));
        }
        if class.coordinated {
            Self::refresh_from_catalog(paths, &catalog, &mut state)?;
        }
        Ok(None)
    }

    fn admission_reply(
        catalog: &generations::Catalog,
        state: &State,
        class: &RequestClass,
    ) -> Result<Option<Response>> {
        if !class.admission {
            return Ok(None);
        }
        if catalog.active()?.as_deref() != Some(&state.generation) {
            return Ok(Some(Response::Redirect {
                generation: catalog.active()?.context("No active generation")?,
            }));
        }
        if !class.shared_write && !catalog.admitted(&state.generation)? {
            return Ok(Some(Response::Error(
                "Creation rejected before execution: recovery is in progress".into(),
            )));
        }
        Ok(None)
    }

    fn refresh_from_catalog(
        paths: &Paths,
        catalog: &generations::Catalog,
        state: &mut State,
    ) -> Result<()> {
        let mut refreshed = state.clone();
        catalog.refresh(&mut refreshed)?;
        if refreshed
            .vnc_profiles
            .iter()
            .any(|p| !p.password.is_empty())
        {
            ensure!(
                crate::vnc_session::credentials::migrate(paths, &mut refreshed.vnc_profiles) == 0,
                "Unlock the OS credential store to migrate legacy VNC passwords"
            );
            if catalog.active()?.as_deref() == Some(&refreshed.generation) {
                catalog.save_workspace(&refreshed)?;
            }
        }
        *state = refreshed;
        Ok(())
    }
}
