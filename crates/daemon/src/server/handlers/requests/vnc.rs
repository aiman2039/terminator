use anyhow::{Context, Result, ensure};
use std::sync::Arc;
use terminator_core::*;

use super::super::super::shared::{Shared, relock};
use super::Outcome;
use crate::vnc_session::credentials;

impl Shared {
    pub(super) fn on_vnc_profile_save(
        &self,
        mut profile: vnc::VncProfile,
        forget_password: bool,
    ) -> Result<Outcome> {
        Self::normalize_vnc_profile(&mut profile)?;
        let previous = self.vnc_profile(|p| p.id == profile.id);
        Self::ensure_vnc_endpoint_kept(previous.as_ref(), &profile, forget_password)?;
        self.store_vnc_credential(previous.as_ref(), &mut profile, forget_password)?;
        self.upsert_vnc_profile(profile)?;
        Ok(Outcome::Changed)
    }

    pub(super) fn on_vnc_forget_credential(&self, id: &str) -> Result<Outcome> {
        let profile = self
            .vnc_profile(|p| p.id == id)
            .context("Unknown VNC profile")?;
        credentials::forget(self.credential_paths(), &profile)?;
        let mut state = relock(&self.state);
        if let Some(profile) = state.vnc_profiles.iter_mut().find(|p| p.id == id) {
            profile.password.clear();
            profile.credential_saved = false;
        }
        state.revision = state.revision.saturating_add(1);
        Ok(Outcome::Changed)
    }

    pub(super) fn on_vnc_transfer(
        self: &Arc<Self>,
        session: &str,
        transfer: vnc::VncTransfer,
    ) -> Result<Outcome> {
        self.vnc_transfer(session, transfer)?;
        Ok(Outcome::Reply(Response::Ok))
    }

    pub(super) fn on_vnc_transfer_status(&self, session: &str, id: &str) -> Result<Outcome> {
        Ok(Outcome::Reply(Response::VncTransferStatus(
            self.vnc_transfer_status(session, id)?,
        )))
    }

    pub(super) fn on_vnc_profile_remove(&self, id: &str) -> Result<Outcome> {
        if let Some(profile) = self.vnc_profile(|p| p.id == id && p.credential_saved) {
            credentials::forget(self.credential_paths(), &profile)?;
        }
        let mut state = relock(&self.state);
        state.vnc_profiles.retain(|p| p.id != id);
        state.revision = state.revision.saturating_add(1);
        Ok(Outcome::Changed)
    }

    pub(super) fn on_vnc_connect(
        self: &Arc<Self>,
        spec: crate::vnc_session::VncConnectSpec,
    ) -> Result<Outcome> {
        Ok(Outcome::Reply(Response::VncCreated(
            self.vnc_connect(spec)?,
        )))
    }

    pub(super) fn on_vnc_disconnect(&self, session: &str) -> Result<Outcome> {
        self.vnc_disconnect(session)?;
        Ok(Outcome::Changed)
    }

    pub(super) fn on_vnc_input(
        &self,
        session: &str,
        events: Vec<vnc::VncInputEvent>,
    ) -> Result<Outcome> {
        self.vnc_input(session, events)?;
        Ok(Outcome::Reply(Response::Ok))
    }

    pub(super) fn on_vnc_frame(
        &self,
        request: crate::vnc_session::VncFrameRequest<'_>,
    ) -> Result<Outcome> {
        Ok(Outcome::Reply(match self.vnc_frame(request)? {
            Some(frame) => Response::VncFrame(Box::new(frame)),
            None => Response::Unchanged,
        }))
    }

    fn credential_paths(&self) -> &Paths {
        self.catalog_paths.as_ref().unwrap_or(&self.paths)
    }

    fn vnc_profile(&self, matches: impl Fn(&vnc::VncProfile) -> bool) -> Option<vnc::VncProfile> {
        relock(&self.state)
            .vnc_profiles
            .iter()
            .find(|p| matches(p))
            .cloned()
    }

    fn normalize_vnc_profile(profile: &mut vnc::VncProfile) -> Result<()> {
        profile.host = profile.host.trim().to_string();
        profile.name = profile.name.trim().to_string();
        profile.username = profile.username.trim().to_string();
        if profile.id.trim().is_empty() {
            profile.id = id();
        }
        Ok(profile.validate()?)
    }

    fn ensure_vnc_endpoint_kept(
        previous: Option<&vnc::VncProfile>,
        profile: &vnc::VncProfile,
        forget_password: bool,
    ) -> Result<()> {
        let Some(previous) = previous.filter(|p| p.credential_saved) else {
            return Ok(());
        };
        ensure!(
            forget_password
                || !profile.password.is_empty()
                || (previous.host == profile.host
                    && previous.port == profile.port
                    && previous.username == profile.username
                    && previous.options.transport == profile.options.transport
                    && previous.options.ssh == profile.options.ssh
                    && previous.options.tls_server_name == profile.options.tls_server_name
                    && previous.options.tls_ca_file == profile.options.tls_ca_file),
            "Enter a password or forget the saved credential before changing the authentication endpoint"
        );
        Ok(())
    }

    fn store_vnc_credential(
        &self,
        previous: Option<&vnc::VncProfile>,
        profile: &mut vnc::VncProfile,
        forget_password: bool,
    ) -> Result<()> {
        let paths = self.credential_paths();
        if forget_password {
            credentials::forget(paths, previous.unwrap_or(&*profile))?;
            zeroize::Zeroize::zeroize(&mut profile.password);
            profile.credential_saved = false;
        } else if !profile.password.is_empty() {
            credentials::save_replacing(credentials::SaveReplacingInput {
                paths,
                previous,
                profile,
                password: &profile.password,
            })?;
            zeroize::Zeroize::zeroize(&mut profile.password);
            profile.credential_saved = true;
        } else {
            profile.credential_saved = previous.is_some_and(|p| p.credential_saved);
        }
        Ok(())
    }

    fn upsert_vnc_profile(&self, mut profile: vnc::VncProfile) -> Result<()> {
        let mut state = relock(&self.state);
        if let Some(existing) = state.vnc_profiles.iter_mut().find(|p| p.id == profile.id) {
            if profile.last_connected == 0 {
                profile.last_connected = existing.last_connected;
            }
            *existing = profile;
        } else {
            ensure!(
                state.vnc_profiles.len() < 256,
                "Too many saved VNC profiles"
            );
            state.vnc_profiles.push(profile);
        }
        state.revision = state.revision.saturating_add(1);
        Ok(())
    }
}
