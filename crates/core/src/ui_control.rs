//! Authenticated GUI control, separate from the persistent daemon protocol.
//! Old daemons remain usable; an old GUI simply has no gui.sock endpoint.
use crate::{Paths, read_frame, write_frame};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{os::unix::net::UnixStream, path::PathBuf, time::Duration};
pub const CAPABILITY: &str = "gui-control-v1";
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
            Self::FixturePlayer { action, url } => {
                ensure!(
                    matches!(action.as_str(), "play" | "stop" | "pause" | "resume"),
                    "Invalid player fixture action"
                );
                if let Some(url) = url {
                    let url = url::Url::parse(url)?;
                    ensure!(
                        url.scheme() == "http" && url.host_str() == Some("127.0.0.1"),
                        "Player fixtures require loopback HTTP"
                    );
                }
            }

            Self::ShowSession {
                session,
                anchor,
                split,
            } => {
                ensure!(
                    session.len() <= 128 && anchor.as_ref().is_none_or(|s| s.len() <= 128),
                    "Invalid session ID"
                );
                ensure!(
                    split
                        .as_deref()
                        .is_none_or(|s| matches!(s, "left" | "right" | "up" | "down")),
                    "Invalid split direction"
                );
                ensure!(
                    split.is_none() || anchor.is_some(),
                    "Splits require an originating session"
                );
            }
            Self::Focus { session } => ensure!(session.len() <= 128, "Invalid session ID"),
            Self::OpenFile { project, path, .. } => ensure!(
                project.len() <= 128 && path.is_absolute() && path.as_os_str().len() <= 4096,
                "Expected project ID and absolute file path"
            ),
            Self::OpenBrowser { url } => {
                crate::metadata::http_url(url)?;
            }
            Self::Window { action } => ensure!(
                matches!(
                    action.as_str(),
                    "minimize" | "maximize" | "restore" | "focus" | "close"
                ),
                "Unknown window command"
            ),
            _ => {}
        }
        Ok(())
    }
}
pub fn rpc(paths: &Paths, request: Request) -> Result<serde_json::Value> {
    request.validate()?;
    let mut stream = UnixStream::connect(paths.runtime.join("gui.sock"))
        .context("GUI control unavailable; open the updated Terminator GUI")?;
    stream.set_read_timeout(Some(Duration::from_secs(8)))?;
    stream.set_write_timeout(Some(Duration::from_secs(3)))?;
    write_frame(
        &mut stream,
        &Envelope {
            version: 1,
            auth: paths.token()?,
            request,
        },
    )?;
    let response: Response = read_frame(&mut stream)?;
    if let Some(error) = response.error {
        anyhow::bail!("{error}");
    }
    response.result.context("Missing GUI result")
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
