//! Authenticated GUI control, separate from the persistent daemon protocol.
//! Old daemons remain usable; an old GUI simply has no gui.sock endpoint.
use crate::{Paths, read_frame, transport, write_frame};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, time::Duration};
pub const CAPABILITY: &str = "gui-control-v1";

/// Why a GUI control request was rejected or failed.
#[derive(Debug, thiserror::Error)]
pub enum UiControlError {
    #[error("Invalid player fixture action")]
    InvalidFixtureAction { action: String },
    #[error("Player fixtures require loopback HTTP")]
    FixtureNotLoopback { url: String },
    #[error("Invalid session ID")]
    InvalidSession { session: String },
    #[error("Invalid split direction")]
    InvalidSplit { split: String },
    #[error("Splits require an originating session")]
    SplitWithoutAnchor { session: String },
    #[error("Expected project ID and absolute file path")]
    InvalidOpenFile { project: String, path: PathBuf },
    #[error("Unknown window command")]
    UnknownWindowCommand { action: String },
    #[error("GUI control unavailable; open the updated Terminator GUI")]
    Unavailable {
        #[source]
        source: transport::TransportError,
    },
    /// The GUI answered with an error; the text is the GUI's.
    #[error("{message}")]
    Gui { message: String },
    #[error("Missing GUI result")]
    MissingResult,
    #[error(transparent)]
    Url(#[from] url::ParseError),
    #[error(transparent)]
    Link(#[from] crate::metadata::MetadataError),
    #[error(transparent)]
    Transport(#[from] transport::TransportError),
    #[error(transparent)]
    Paths(#[from] crate::PathsError),
    #[error(transparent)]
    Frame(#[from] crate::FrameError),
}
type Result<T, E = UiControlError> = std::result::Result<T, E>;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Request {
    #[doc(hidden)]
    FixturePlayer {
        action: String,
        url: Option<String>,
    },
    Ping,
    Snapshot,
    ShowSession {
        session: String,
        anchor: Option<String>,
        split: Option<String>,
    },
    Focus {
        session: String,
    },
    OpenFile {
        project: String,
        path: PathBuf,
        as_text: bool,
    },
    OpenBrowser {
        url: String,
    },
    Window {
        action: String,
    },
}
#[derive(Serialize, Deserialize)]
pub struct Envelope {
    pub version: u32,
    pub auth: String,
    pub request: Request,
}
#[derive(Serialize, Deserialize)]
pub struct Response {
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
}
impl Request {
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::FixturePlayer { action, url } => validate_fixture(action, url.as_deref())?,
            Self::ShowSession {
                session,
                anchor,
                split,
            } => validate_show(session, anchor.as_deref(), split.as_deref())?,
            Self::Focus { session } if session.len() > 128 => {
                return Err(UiControlError::InvalidSession {
                    session: session.clone(),
                });
            }
            Self::OpenFile { project, path, .. }
                if !(project.len() <= 128
                    && path.is_absolute()
                    && path.as_os_str().len() <= 4096) =>
            {
                return Err(UiControlError::InvalidOpenFile {
                    project: project.clone(),
                    path: path.clone(),
                });
            }
            Self::OpenBrowser { url } => {
                crate::metadata::http_url(url)?;
            }
            Self::Window { action }
                if !matches!(
                    action.as_str(),
                    "minimize" | "maximize" | "restore" | "focus" | "close"
                ) =>
            {
                return Err(UiControlError::UnknownWindowCommand {
                    action: action.clone(),
                });
            }
            _ => {}
        }
        Ok(())
    }
}
fn validate_fixture(action: &str, url: Option<&str>) -> Result<()> {
    if !matches!(action, "play" | "stop" | "pause" | "resume") {
        return Err(UiControlError::InvalidFixtureAction {
            action: action.to_owned(),
        });
    }
    if let Some(url) = url {
        let parsed = url::Url::parse(url)?;
        if parsed.scheme() != "http" || parsed.host_str() != Some("127.0.0.1") {
            return Err(UiControlError::FixtureNotLoopback {
                url: url.to_owned(),
            });
        }
    }
    Ok(())
}
fn validate_show(session: &str, anchor: Option<&str>, split: Option<&str>) -> Result<()> {
    if session.len() > 128 || anchor.is_some_and(|s| s.len() > 128) {
        return Err(UiControlError::InvalidSession {
            session: session.to_owned(),
        });
    }
    if let Some(split) = split
        && !matches!(split, "left" | "right" | "up" | "down")
    {
        return Err(UiControlError::InvalidSplit {
            split: split.to_owned(),
        });
    }
    if split.is_some() && anchor.is_none() {
        return Err(UiControlError::SplitWithoutAnchor {
            session: session.to_owned(),
        });
    }
    Ok(())
}
pub fn rpc(paths: &Paths, request: Request) -> Result<serde_json::Value> {
    request.validate()?;
    let mut stream = transport::Stream::connect_ipc(&paths.runtime.join("gui.sock"))
        .map_err(|source| UiControlError::Unavailable { source })?;
    stream.set_timeouts(Duration::from_secs(8), Duration::from_secs(3))?;
    write_frame(
        &mut stream,
        &Envelope {
            version: 1,
            auth: paths.token()?,
            request,
        },
    )?;
    let response: Response = read_frame(&mut stream)?;
    if let Some(message) = response.error {
        return Err(UiControlError::Gui { message });
    }
    response.result.ok_or(UiControlError::MissingResult)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn split_requests_require_an_anchor_and_valid_direction() {
        assert!(
            Request::ShowSession {
                session: "s".into(),
                anchor: None,
                split: Some("right".into())
            }
            .validate()
            .is_err()
        );
        assert!(
            Request::ShowSession {
                session: "s".into(),
                anchor: Some("a".into()),
                split: Some("right".into())
            }
            .validate()
            .is_ok()
        );
        assert!(
            Request::OpenBrowser {
                url: "file:///etc/passwd".into()
            }
            .validate()
            .is_err()
        );
    }
}
