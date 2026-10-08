#[cfg(test)]
use super::cache::PresentationCache;
use super::cache::{SessionView, ago};
use super::counts::{AttentionCounts, attention_status_icon};
use super::owner::OwnerSupport;
use super::presentation::{AgentPresentation, Diagnostics};
#[cfg(test)]
use terminator_core::State;
use terminator_core::{
    AgentState, Session,
    agents::{self, PresenceOutcome},
};
/// Present one terminal session. Callers must pass the current time; the
/// model never reads the clock itself so tests stay deterministic.
#[cfg(test)]
#[must_use]
pub(crate) fn present_session(state: &State, session_id: &str, now: u64) -> AgentPresentation {
    let mut cache = PresentationCache::default();
    (*cache.get(state, session_id, now, None)).clone()
}

pub(super) fn unknown_presentation() -> AgentPresentation {
    AgentPresentation {
        brand_icon: None,
        brand_label: None,
        detected_kinds: Vec::new(),
        verified: false,
        live: false,
        lifecycle: None,
        status_label: "Status unavailable".into(),
        status_icon: attention_status_icon(AgentState::Unknown),
        spin: false,
        unread: 0,
        attention: AttentionCounts::default(),
        notice_preview: None,
        diagnostics: None,
    }
}

pub(crate) fn build_presentation(view: &SessionView<'_>, now: u64) -> AgentPresentation {
    let SessionView {
        session,
        support,
        presence,
        hook,
        attention,
        unread,
        fresh,
        notice_preview,
        ..
    } = *view;
    let verified = session.lifecycle.live()
        && support.supported
        && support.available
        && presence.is_some_and(|item| item.outcome == PresenceOutcome::Verified)
        && fresh.unwrap_or_else(|| agents::presence_verified(presence, now));
    let detected: Vec<_> = if verified {
        presence.map(|item| item.agents.clone()).unwrap_or_default()
    } else {
        Vec::new()
    };
    let detected_kinds: Vec<String> = detected.iter().map(|agent| agent.kind.clone()).collect();
    // A fresh verified-empty observation means the agent exited and the
    // terminal is a plain shell again. Hook history stays for the inbox,
    // but terminal chrome must not keep the brand or lifecycle status.
    let plain_shell = verified && detected.is_empty();
    let (brand_icon, brand_label) = if !detected.is_empty() {
        match agents::preferred_agent(&detected) {
            Some(agent) => (
                Some(agents::icon_key(&agent.kind)),
                Some(agents::display_name(&agent.kind).to_string()),
            ),
            None => (
                Some(agents::MULTIPLE_ICON),
                Some(format!("{} agents", detected.len())),
            ),
        }
    } else if let Some(hook) = hook
        && !plain_shell
    {
        (
            Some(agents::icon_key(&hook.kind)),
            Some(agents::display_name(&hook.kind).to_string()),
        )
    } else {
        (None, None)
    };
    let lifecycle = if plain_shell {
        None
    } else {
        hook.map(|agent| agent.state)
    };
    let status_label = match lifecycle {
        Some(status) if status != AgentState::Unknown => status.label().to_string(),
        _ => "Status unavailable".to_string(),
    };
    let status_icon = attention_status_icon(lifecycle.unwrap_or(AgentState::Unknown));
    let hook_linked = match (hook, hook.and_then(|agent| agent.process.as_ref())) {
        (Some(current), Some(identity)) => detected
            .iter()
            .any(|agent| agent.kind == current.kind && agent.process == *identity),
        _ => false,
    };
    AgentPresentation {
        brand_icon,
        brand_label,
        detected_kinds,
        verified,
        live: !detected.is_empty(),
        lifecycle,
        status_label,
        status_icon,
        spin: lifecycle == Some(AgentState::Running),
        unread,
        attention,
        notice_preview: notice_preview.clone(),
        diagnostics: Some(Diagnostics {
            session: session.clone(),
            presence: presence.cloned(),
            detected,
            hook: hook.cloned(),
            support,
            hook_linked,
        }),
    }
}

/// Plain-text sentence from a hook summary. Markdown markers are dropped.
pub(crate) fn notice_preview(markdown: &str) -> String {
    use pulldown_cmark::{Event, Parser, TagEnd};
    let mut text = String::new();
    for event in Parser::new(markdown) {
        match event {
            Event::Text(value) | Event::Code(value) => text.push_str(&value),
            Event::SoftBreak
            | Event::HardBreak
            | Event::End(
                TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::Item | TagEnd::CodeBlock,
            ) => text.push(' '),
            _ => {}
        }
    }
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[allow(clippy::too_many_arguments)]
pub(super) fn diagnostics_text(
    session: &Session,
    presence: Option<&terminator_core::agents::TerminalPresence>,
    detected: &[terminator_core::agents::DetectedAgent],
    hook: Option<&terminator_core::Agent>,
    support: OwnerSupport,
    verified: bool,
    hook_linked: bool,
    status_label: &str,
    unread: usize,
    now: u64,
) -> String {
    let mut lines = Vec::new();
    if !session.lifecycle.live() {
        lines.push("Session ended".into());
    } else if detected.is_empty() {
        if verified {
            lines.push(format!(
                "No live agent detected (verified {})",
                ago(presence.map(|p| p.observed_at).unwrap_or(now), now)
            ));
        } else if !support.available {
            lines.push("Presence unverified: session owner unavailable".into());
        } else if !support.supported {
            lines.push("Presence unverified: unsupported daemon".into());
        } else if presence.is_some_and(|p| p.outcome == PresenceOutcome::Unavailable) {
            lines.push("Presence unverified: detection unavailable".into());
        } else if let Some(observed) = presence.map(|p| p.observed_at) {
            lines.push(format!(
                "Presence unverified: stale observation ({})",
                ago(observed, now)
            ));
        } else {
            lines.push("Presence unverified: no observation yet".into());
        }
    } else {
        let names: Vec<_> = detected
            .iter()
            .map(|a| agents::display_name(&a.kind))
            .collect();
        let age = ago(presence.map(|p| p.observed_at).unwrap_or(now), now);
        if detected.len() == 1
            && agents::preferred_agent(detected).is_some()
            && let (Some(agent), Some(name)) = (detected.first(), names.first())
        {
            let how = if agent.foreground {
                "foreground"
            } else {
                "only agent"
            };
            lines.push(format!("{name} live (verified {age}; {how})"));
        } else {
            lines.push(format!("{} live (verified {age})", names.join(", ")));
        }
    }
    lines.push(if detected.is_empty() {
        match hook {
            Some(_) if !verified => "Identity source: hook event".into(),
            _ => "Identity source: none".into(),
        }
    } else {
        "Identity source: process inspection".into()
    });
    match hook {
        Some(h) => lines.push(format!(
            "Hook ({}): {} · {} · {}",
            agents::display_name(&h.kind),
            h.state.label(),
            if hook_linked {
                "linked to live process"
            } else {
                "last reported"
            },
            ago(h.updated, now),
        )),
        None => lines.push("Hook: none reported".into()),
    }
    lines.push(format!("Status: {status_label}"));
    if unread > 0 {
        lines.push(format!("Unread: {unread}"));
    }
    lines.push(session.cwd.display().to_string());
    lines.join("\n")
}

/// Sessions with verified live agents, for the All-live view.
#[cfg(test)]
#[must_use]
pub(crate) fn live_sessions(state: &State, now: u64) -> Vec<(&Session, Vec<String>)> {
    state
        .sessions
        .iter()
        .filter(|s| s.lifecycle.live())
        .filter_map(|s| {
            let presentation = present_session(state, &s.id, now);
            if presentation.live {
                Some((s, presentation.detected_kinds))
            } else {
                None
            }
        })
        .collect()
}

/// Live sessions with hook records but no verified live agent. Covers older
/// daemons, unavailable owners, stale observations, and detection failures.
/// A verified-empty observation means the agent exited: the terminal is a
/// plain shell and is excluded.
#[cfg(test)]
#[must_use]
pub(crate) fn unverified_sessions(state: &State, now: u64) -> Vec<&Session> {
    state
        .sessions
        .iter()
        .filter(|s| {
            if !s.lifecycle.live() {
                return false;
            }
            let presentation = present_session(state, &s.id, now);
            !presentation.live && !presentation.verified && presentation.lifecycle.is_some()
        })
        .collect()
}
