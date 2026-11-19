//! SSH sign-in with prompts in the GUI (`ssh-askpass-v1`).
//!
//! The daemon runs one OpenSSH client that becomes the host's multiplexing
//! master. `SSH_ASKPASS` points at `terminator-hook`, and
//! [`ASKPASS_SOCKET_ENV`] names a private socket that only this sign-in
//! listens on. Each password, passphrase or one-time-code prompt goes
//! through that socket to the daemon, which shows it to the GUI and waits
//! for the answer typed there. Answers stay in memory: they are never
//! saved, logged or put on a command line.
use serde::{Deserialize, Serialize};
use std::{ffi::OsString, path::Path};

use super::{SshOptions, SshOptionsError};

/// `SshSignIn`, `SshSignInStatus` and `AnswerSshPrompt`.
pub const SIGN_IN_CAPABILITY: &str = "ssh-askpass-v1";

/// Names the sign-in's private socket in the `ssh` child's environment;
/// `terminator-hook` in askpass mode connects to it.
pub const ASKPASS_SOCKET_ENV: &str = "TERMINATOR_ASKPASS_SOCKET";

/// Longest prompt or answer the socket carries.
pub const MAX_ASKPASS_TEXT: usize = 4096;

/// A prompt OpenSSH is waiting on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignInPrompt {
    pub id: u64,
    /// OpenSSH's own prompt text, for example `(me@host) Password:`.
    pub text: String,
}

/// Text that must not show up in logs or debug output.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Secret(String);

impl Secret {
    #[must_use]
    pub fn new(text: String) -> Self {
        Self(text)
    }
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

/// One line from `terminator-hook` to the daemon.
#[derive(Debug, Serialize, Deserialize)]
pub struct AskpassQuestion {
    pub prompt: String,
}

/// The daemon's reply line. `None` cancels: the hook exits non-zero and
/// OpenSSH stops asking.
#[derive(Debug, Serialize, Deserialize)]
pub struct AskpassAnswer {
    pub answer: Option<Secret>,
}

/// Arguments for the sign-in client: the batch arguments with the shared
/// `control` master, but without `BatchMode`, so OpenSSH may ask through
/// `SSH_ASKPASS`. Host keys stay strict: unknown keys go through the trust
/// prompt first. The caller adds `--`, the host and a command.
pub fn arguments(options: &SshOptions, control: &Path) -> Result<Vec<OsString>, SshOptionsError> {
    let mut args = super::arguments(options, false, Some(control))?;
    args.retain(|arg| arg != "-oBatchMode=yes");
    Ok(args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_never_show_in_debug_output() {
        let answer = AskpassAnswer {
            answer: Some(Secret::new("hunter2".into())),
        };
        assert!(!format!("{answer:?}").contains("hunter2"));
        let json = serde_json::to_string(&answer).unwrap();
        assert_eq!(json, r#"{"answer":"hunter2"}"#);
    }

    #[test]
    fn sign_in_may_prompt_but_keeps_strict_host_keys_and_the_master() {
        let options = SshOptions {
            host: "box".into(),
            ..SshOptions::default()
        };
        let args = arguments(&options, Path::new("/run/ssh-0123abcd")).unwrap();
        let args: Vec<String> = args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(!args.iter().any(|a| a == "-oBatchMode=yes"), "{args:?}");
        assert!(args.iter().any(|a| a == "-oStrictHostKeyChecking=yes"));
        assert!(args.iter().any(|a| a == "-oControlMaster=auto"));
    }
}
