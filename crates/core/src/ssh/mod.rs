//! Shared OpenSSH configuration. Keys and agents remain owned by OpenSSH.
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

mod config;
mod failure;
mod tuning;
pub use config::{ConfigHost, ConfigSource, config_hosts, parse_config_aliases, parse_config_host};
pub use failure::{SshError, SshFailure, classify};
pub use tuning::{TUNING_CAPABILITY, Tuning, set_tuning};

/// `ScanHostKey`, `TrustHostKey` and `Response::SshError`.
pub const HOST_KEYS_CAPABILITY: &str = "ssh-hostkeys-v1";

/// Why SSH options, `ssh -G` output, or a forward request were rejected
/// before OpenSSH ran.
#[derive(Debug, thiserror::Error)]
pub enum SshOptionsError {
    #[error("Invalid host name or IP address")]
    InvalidHost { host: String },
    #[error("SSH port must be between 1 and 65535")]
    InvalidPort,
    #[error("Invalid SSH username")]
    InvalidUsername { username: String },
    #[error("SSH file paths must be absolute")]
    RelativePath { path: PathBuf },
    #[error("SSH paths must be valid text without control characters")]
    InvalidPathText { path: PathBuf },
    #[error("ssh -G did not report a hostname")]
    MissingHostname,
    #[error("ssh -G did not report a port")]
    MissingPort,
    #[error(transparent)]
    Forward(#[from] crate::remote::ForwardError),
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SshOptions {
    pub host: String,
    /// `None` passes no `-p`, so `~/.ssh/config` (or 22) decides. Omitted
    /// when unset so older daemons fall back to their own default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    pub username: String,
    pub identity_file: Option<PathBuf>,
    pub known_hosts_file: Option<PathBuf>,
}
impl SshOptions {
    pub fn validate(&self) -> Result<(), SshOptionsError> {
        validate_host(&self.host)?;
        if self.port == Some(0) {
            return Err(SshOptionsError::InvalidPort);
        }
        let valid_username = self.username.len() <= 128
            && !self.username.starts_with('-')
            && self
                .username
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "_-.".contains(c));
        if !valid_username {
            return Err(SshOptionsError::InvalidUsername {
                username: self.username.clone(),
            });
        }
        for path in [&self.identity_file, &self.known_hosts_file]
            .into_iter()
            .flatten()
        {
            if !path.is_absolute() {
                return Err(SshOptionsError::RelativePath { path: path.clone() });
            }
            let valid_text = path
                .to_str()
                .is_some_and(|s| s.len() <= 4096 && !s.chars().any(char::is_control));
            if !valid_text {
                return Err(SshOptionsError::InvalidPathText { path: path.clone() });
            }
        }
        Ok(())
    }
    /// `user@host:port` for display, omitting whatever OpenSSH configuration
    /// decides.
    #[must_use]
    pub fn authority(&self) -> String {
        let mut text = String::new();
        if !self.username.is_empty() {
            text.push_str(&self.username);
            text.push('@');
        }
        text.push_str(&self.host);
        if let Some(port) = self.port {
            text.push_str(&format!(":{port}"));
        }
        text
    }
}

/// `control` enables OpenSSH multiplexing through that socket; see
/// `Paths::ssh_control`. Without it every call opens its own connection.
pub fn arguments(
    options: &SshOptions,
    terminal: bool,
    control: Option<&Path>,
) -> Result<Vec<OsString>, SshOptionsError> {
    build(options, terminal, control.map(|path| (path, false)))
}

/// Batch arguments for a dedicated multiplexing master that only carries
/// port forwards: `-f -N` backgrounds it after authentication, and it
/// persists until `ssh -O exit` rather than for an idle timeout, because
/// idle forward listeners do not count as activity for `ControlPersist`.
///
/// `ClearAllForwardings=yes` stays: OpenSSH applies it once, while
/// finalizing the master's own options, to forwards from configuration and
/// its command line. Forwards added later through `ssh -O forward` are mux
/// requests the master processes on its own and are unaffected (verified
/// against OpenSSH 10.3). It is the *requesting* client that must not carry
/// it, or that client clears its own `-L` before sending; see
/// [`forward_control`].
pub fn forward_master_arguments(
    options: &SshOptions,
    control: &Path,
) -> Result<Vec<OsString>, SshOptionsError> {
    let mut args = build(options, false, Some((control, true)))?;
    args.extend(["-f".into(), "-N".into()]);
    Ok(args)
}

fn build(
    options: &SshOptions,
    terminal: bool,
    control: Option<(&Path, bool)>,
) -> Result<Vec<OsString>, SshOptionsError> {
    options.validate()?;
    // A terminal has a PTY, so OpenSSH may prompt there for passwords,
    // passphrases, 2FA and first-connect host keys. Batch calls never prompt;
    // with `control` they reuse the master a terminal authenticated.
    let (tty, strict) = if terminal {
        ("-tt", "-oStrictHostKeyChecking=ask")
    } else {
        ("-T", "-oStrictHostKeyChecking=yes")
    };
    let mut args: Vec<OsString> = [
        tty,
        "-oForwardAgent=no",
        "-oForwardX11=no",
        "-oPermitLocalCommand=no",
        strict,
        "-oClearAllForwardings=yes",
        "-oLogLevel=ERROR",
    ]
    .into_iter()
    .map(OsString::from)
    .collect();
    let tuning = tuning::current();
    args.extend(tuning.connect_args().map(OsString::from));
    if !terminal {
        args.push("-oBatchMode=yes".into());
    }
    if let Some(port) = options.port {
        args.extend([OsString::from("-p"), port.to_string().into()]);
    }
    if !options.username.is_empty() {
        args.extend([OsString::from("-l"), options.username.clone().into()]);
    }
    if let Some(path) = &options.identity_file {
        args.extend([OsString::from("-i"), path.as_os_str().into()]);
    }
    if let Some(path) = &options.known_hosts_file {
        args.extend([
            OsString::from("-o"),
            format!("UserKnownHostsFile={}", quoted(path)).into(),
        ]);
    }
    if let Some((path, dedicated)) = control {
        let (master, persist) = if dedicated {
            ("-oControlMaster=yes", "-oControlPersist=yes".to_owned())
        } else {
            ("-oControlMaster=auto", tuning.persist_arg())
        };
        args.extend([
            master.into(),
            // ControlPath expands `%` tokens; a literal runtime path must not.
            format!("-oControlPath={}", quoted(path).replace('%', "%%")).into(),
            persist.into(),
        ]);
    } else {
        args.push("-oControlMaster=no".into());
    }
    Ok(args)
}

/// Asks the multiplexing master behind `control` to exit. Best-effort: the
/// master may already be gone. `-F none` keeps user configuration out of it.
pub fn exit_control(control: &Path) -> std::process::Command {
    control_command("exit", control, None)
}

/// Exits 0 only while a multiplexing master answers on `control`.
pub fn check_control(control: &Path) -> std::process::Command {
    control_command("check", control, None)
}

/// `ssh -O forward` (`start`) or `ssh -O cancel` for one local forward on
/// the master behind `control`. `-F none` keeps `LocalForward` entries from
/// `~/.ssh/config` out of the request, and no `ClearAllForwardings` is
/// passed: on this client it would drop the `-L` before it is sent.
pub fn forward_control(
    start: bool,
    control: &Path,
    forward: &crate::remote::Forward,
) -> Result<std::process::Command, SshOptionsError> {
    forward.validate()?;
    Ok(control_command(
        if start { "forward" } else { "cancel" },
        control,
        Some(&forward.spec()),
    ))
}

fn control_command(operation: &str, control: &Path, local: Option<&str>) -> std::process::Command {
    let mut command = std::process::Command::new("ssh");
    command.args(["-F", "none", "-O", operation]).arg(format!(
        "-oControlPath={}",
        quoted(control).replace('%', "%%")
    ));
    if let Some(spec) = local {
        command.arg("-L").arg(spec);
    }
    command.args(["--", "terminator-control"]);
    for key in private_environment_keys() {
        command.env_remove(key);
    }
    command
}

/// One public host key as `ssh-keyscan` reports it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostKey {
    pub key_type: String,
    /// Base64 public key blob.
    pub key: String,
    /// `SHA256:…` from `ssh-keygen -l`. Informational; never trusted.
    #[serde(default)]
    pub fingerprint: String,
}

/// Keys a host presented, plus the `known_hosts` host field they would be
/// stored under.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostKeyScan {
    pub host: String,
    pub keys: Vec<HostKey>,
}

/// Endpoint after `ssh -G` applied the user's configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    pub hostname: String,
    pub port: u16,
    pub host_key_alias: Option<String>,
}
impl Resolved {
    /// The host field OpenSSH looks up: a `HostKeyAlias` verbatim, otherwise
    /// the hostname, bracketed with the port when it is not 22.
    #[must_use]
    pub fn known_hosts_host(&self) -> String {
        match &self.host_key_alias {
            Some(alias) => alias.clone(),
            None if self.port == 22 => self.hostname.clone(),
            None => format!("[{}]:{}", self.hostname, self.port),
        }
    }
}

/// Parses `ssh -G` output.
pub fn parse_resolved(output: &str) -> Result<Resolved, SshOptionsError> {
    let mut hostname = None;
    let mut port = None;
    let mut host_key_alias = None;
    for line in output.lines() {
        let Some((key, value)) = line.split_once(' ') else {
            continue;
        };
        let value = value.trim();
        match key {
            "hostname" => hostname = Some(value.to_owned()),
            "port" => port = value.parse::<u16>().ok(),
            "hostkeyalias" if !value.is_empty() && value != "none" => {
                host_key_alias = Some(value.to_owned());
            }
            _ => {}
        }
    }
    let hostname = hostname.ok_or(SshOptionsError::MissingHostname)?;
    validate_host(&hostname)?;
    if let Some(alias) = &host_key_alias {
        validate_host(alias)?;
    }
    Ok(Resolved {
        hostname,
        port: port
            .filter(|p| *p > 0)
            .ok_or(SshOptionsError::MissingPort)?,
        host_key_alias,
    })
}

/// Parses `ssh-keyscan` stdout. Comments and malformed lines are skipped;
/// duplicate keys are reported once.
#[must_use]
pub fn parse_keyscan(output: &str) -> Vec<HostKey> {
    let mut keys: Vec<HostKey> = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let (Some(_host), Some(key_type), Some(key)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let valid_type = key_type.len() <= 64
            && key_type
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-@.".contains(c));
        let valid_key = !key.is_empty()
            && key.len() <= 16 * 1024
            && key
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "+/=".contains(c));
        if valid_type && valid_key && !keys.iter().any(|k| k.key_type == key_type && k.key == key) {
            keys.push(HostKey {
                key_type: key_type.to_owned(),
                key: key.to_owned(),
                fingerprint: String::new(),
            });
        }
    }
    keys
}

/// Parses `ssh-keygen -l` lines (`256 SHA256:… comment (ED25519)`), in order.
#[must_use]
pub fn parse_fingerprints(output: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|line| line.split_whitespace().nth(1))
        .filter(|fingerprint| fingerprint.starts_with("SHA256:"))
        .map(str::to_owned)
        .collect()
}

/// `~/.ssh/known_hosts`, where OpenSSH looks first without configuration.
#[must_use]
pub fn default_known_hosts() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|base| base.home_dir().join(".ssh").join("known_hosts"))
}

/// One `known_hosts` line, newline-terminated.
#[must_use]
pub fn known_hosts_line(host: &str, key: &HostKey) -> String {
    format!("{host} {} {}\n", key.key_type, key.key)
}

/// Whether two scans present the same keys, ignoring order and fingerprints.
#[must_use]
pub fn same_keys(a: &[HostKey], b: &[HostKey]) -> bool {
    let set = |keys: &[HostKey]| {
        let mut set: Vec<(String, String)> = keys
            .iter()
            .map(|k| (k.key_type.clone(), k.key.clone()))
            .collect();
        set.sort();
        set.dedup();
        set
    };
    !a.is_empty() && set(a) == set(b)
}

fn quoted(path: &Path) -> String {
    format!(
        "\"{}\"",
        path.to_string_lossy()
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
    )
}

/// Local daemon capabilities must never reach a remote host through `SendEnv`.
pub fn private_environment_keys() -> Vec<OsString> {
    std::env::vars_os()
        .filter_map(|(key, _)| {
            key.to_string_lossy()
                .to_ascii_uppercase()
                .starts_with("TERMINATOR_")
                .then_some(key)
        })
        .collect()
}

pub fn validate_host(host: &str) -> Result<(), SshOptionsError> {
    let valid = !host.is_empty()
        && host.len() <= 255
        && !host.starts_with('-')
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || ".-_:".contains(c));
    if !valid {
        return Err(SshOptionsError::InvalidHost {
            host: host.to_owned(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests;
