//! One fix button for a failed VNC session, chosen from the daemon's error
//! kind (`vnc-errors-v1`). Older daemons send no kind: only the message
//! shows, as before.
use super::*;
use terminator_core::vnc::VncErrorKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ErrorFix {
    /// Ask for the VNC password in this pane.
    EnterPassword,
    /// Open the host's Properties.
    Properties,
    /// Open the Hosts sidebar to sign in over SSH or trust the host key.
    Hosts,
    /// Connect again with the same settings.
    Retry,
}

impl ErrorFix {
    pub(super) fn for_kind(kind: VncErrorKind) -> Self {
        match kind {
            VncErrorKind::PasswordRequired | VncErrorKind::AuthFailed => Self::EnterPassword,
            VncErrorKind::SshAuth | VncErrorKind::SshHostKey => Self::Hosts,
            VncErrorKind::StartCommand | VncErrorKind::Tls => Self::Properties,
            VncErrorKind::Unreachable | VncErrorKind::Protocol | VncErrorKind::Other => Self::Retry,
        }
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::EnterPassword => "Enter password",
            Self::Properties => "Properties",
            Self::Hosts => "Open Hosts",
            Self::Retry => "Retry",
        }
    }
}

/// The failed session a fix applies to.
pub(super) struct ApplyFixInput<'a> {
    pub(super) fix: ErrorFix,
    pub(super) state_key: &'a str,
    pub(super) session: &'a VncSession,
}

impl App {
    /// Apply a fix. Retry returns the connection to start.
    pub(super) fn apply_vnc_error_fix(
        &mut self,
        input: ApplyFixInput<'_>,
    ) -> Option<PendingVncConnect> {
        let ApplyFixInput {
            fix,
            state_key,
            session,
        } = input;
        let profile = self.vnc_session_profile(session);
        match fix {
            ErrorFix::EnterPassword => {
                self.vnc_prompt_password(state_key, session, profile.as_ref());
                None
            }
            ErrorFix::Properties => {
                if let Some(profile) = profile {
                    self.vnc_sidebar.begin_edit(&profile);
                    self.show_vnc_page();
                }
                None
            }
            ErrorFix::Hosts => {
                self.preferences.tool = crate::SidebarTool::Hosts;
                self.preferences.visible = true;
                None
            }
            ErrorFix::Retry => Some(self.vnc_retry_request(state_key, session)),
        }
    }

    /// Show the password prompt in this pane once for a session whose
    /// server asked for a password. "All hosts" in the prompt is not undone.
    pub(crate) fn vnc_auto_password_prompt(&mut self, state_key: &str, session: &VncSession) {
        let wants = session.status == VncSessionStatus::Failed
            && session.error_kind == Some(VncErrorKind::PasswordRequired);
        let shown = self
            .vnc_views
            .get(state_key)
            .is_some_and(|view| view.prompted_for.as_deref() == Some(session.id.as_str()));
        if !wants || shown {
            return;
        }
        let profile = self.vnc_session_profile(session);
        self.vnc_prompt_password(state_key, session, profile.as_ref());
        self.vnc_views
            .entry(state_key.to_owned())
            .or_default()
            .prompted_for = Some(session.id.clone());
    }

    fn vnc_session_profile(&self, session: &VncSession) -> Option<VncProfile> {
        let id = session.profile_id.as_deref()?;
        self.state.vnc_profiles.iter().find(|p| p.id == id).cloned()
    }

    fn vnc_prompt_password(
        &mut self,
        state_key: &str,
        session: &VncSession,
        profile: Option<&VncProfile>,
    ) {
        let view = self.vnc_views.entry(state_key.to_owned()).or_default();
        view.save_profile = false;
        view.draft_password.clear();
        view.draft_host.clone_from(&session.host);
        view.draft_port = session.port.to_string();
        view.draft_display = display_for_port(&view.draft_port);
        view.draft_options = session.options.clone();
        view.draft_view_only = session.view_only;
        view.draft_name.clone_from(&session.label);
        if let Some(profile) = profile {
            view.draft_username.clone_from(&profile.username);
            view.draft_name = profile.display_name();
            view.draft_shared = profile.shared;
        }
    }

    fn vnc_retry_request(&self, state_key: &str, session: &VncSession) -> PendingVncConnect {
        let view = self.vnc_views.get(state_key);
        PendingVncConnect {
            profile_id: session.profile_id.clone(),
            host: session.host.clone(),
            port: session.port,
            username: view.map(|v| v.draft_username.clone()).unwrap_or_default(),
            password: view.map(|v| v.draft_password.clone()).unwrap_or_default(),
            shared: view.is_none_or(|v| v.draft_shared),
            view_only: session.view_only,
            label: Some(session.label.clone()),
            save_profile: false,
            options: session.options.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_error_kind_offers_a_matching_fix() {
        assert_eq!(
            ErrorFix::for_kind(VncErrorKind::PasswordRequired),
            ErrorFix::EnterPassword
        );
        assert_eq!(
            ErrorFix::for_kind(VncErrorKind::SshHostKey),
            ErrorFix::Hosts
        );
        assert_eq!(
            ErrorFix::for_kind(VncErrorKind::StartCommand),
            ErrorFix::Properties
        );
        assert_eq!(
            ErrorFix::for_kind(VncErrorKind::Unreachable),
            ErrorFix::Retry
        );
    }
}
