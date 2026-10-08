#[cfg(test)]
use terminator_core::{AGENT_PRESENCE_CAPABILITY, Session, State};
/// Per-owner presence support for a session. Single-daemon states carry
/// capabilities on the snapshot; multi-owner states carry them per owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OwnerSupport {
    pub supported: bool,
    pub available: bool,
}

#[cfg(test)]
#[must_use]
pub(crate) fn owner_support(state: &State, session: &Session) -> OwnerSupport {
    match state
        .generations
        .iter()
        .find(|health| health.owner.id == session.generation)
    {
        Some(health) => OwnerSupport {
            supported: health
                .capabilities
                .iter()
                .any(|c| c == AGENT_PRESENCE_CAPABILITY),
            available: health.error.is_none(),
        },
        None if state.generations.is_empty() => OwnerSupport {
            supported: state
                .capabilities
                .iter()
                .any(|c| c == AGENT_PRESENCE_CAPABILITY),
            available: true,
        },
        None => OwnerSupport {
            supported: false,
            available: false,
        },
    }
}
