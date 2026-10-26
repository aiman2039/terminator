//! Shared OpenSSH configuration. Keys and agents remain owned by OpenSSH.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{ffi::OsString, path::PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SshOptions {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub identity_file: Option<PathBuf>,
    pub known_hosts_file: Option<PathBuf>,
}
impl Default for SshOptions {
    fn default() -> Self {
        Self {
            host: String::new(),
            port: 22,
            username: String::new(),
            identity_file: None,
            known_hosts_file: None,
        }
    }
}
impl SshOptions {
    pub fn validate(&self) -> Result<()> {
        validate_host(&self.host)?;
        ensure!(self.port > 0, "SSH port must be between 1 and 65535");
        ensure!(
            self.username.len() <= 128
                && !self.username.starts_with('-')
                && self
                    .username
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "_-.".contains(c)),
            "Invalid SSH username"
        );
        for path in [&self.identity_file, &self.known_hosts_file]
            .into_iter()
            .flatten()
        {
            ensure!(path.is_absolute(), "SSH file paths must be absolute");
            ensure!(
                path.to_str()
                    .is_some_and(|s| s.len() <= 4096 && !s.chars().any(char::is_control)),
                "SSH paths must be valid text without control characters"
            );
        }
        Ok(())
    }
}

pub fn arguments(options: &SshOptions, terminal: bool) -> Result<Vec<OsString>> {
    options.validate()?;
    let mut args: Vec<OsString> = [
        if terminal { "-tt" } else { "-T" },
        "-oBatchMode=yes",
        "-oForwardAgent=no",
        "-oForwardX11=no",
        "-oPermitLocalCommand=no",
        "-oStrictHostKeyChecking=yes",
        "-oControlMaster=no",
        "-oClearAllForwardings=yes",
        "-oConnectTimeout=10",
        "-oServerAliveInterval=15",
        "-oServerAliveCountMax=3",
        "-oLogLevel=ERROR",
    ]
    .into_iter()
    .map(OsString::from)
    .collect();
    args.extend([OsString::from("-p"), options.port.to_string().into()]);
    if !options.username.is_empty() {
        args.extend([OsString::from("-l"), options.username.clone().into()]);
    }
    if let Some(path) = &options.identity_file {
        args.extend([OsString::from("-i"), path.as_os_str().into()]);
    }
    if let Some(path) = &options.known_hosts_file {
        args.extend([
            OsString::from("-o"),
            format!(
                "UserKnownHostsFile=\"{}\"",
                path.to_string_lossy()
                    .replace('\\', "\\\\")
                    .replace('"', "\\\"")
            )
            .into(),
        ]);
    }
    Ok(args)
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

pub fn validate_host(host: &str) -> Result<()> {
    ensure!(
        !host.is_empty()
            && host.len() <= 255
            && !host.starts_with('-')
            && host
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || ".-_:".contains(c)),
        "Invalid host name or IP address"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ssh_arguments_require_trust_and_disable_implicit_forwarding() {
        let config = SshOptions {
            host: "fixture".into(),
            username: "developer".into(),
            ..SshOptions::default()
        };
        let args = arguments(&config, true).unwrap();
        for argument in [
            "-tt",
            "-oBatchMode=yes",
            "-oStrictHostKeyChecking=yes",
            "-oForwardAgent=no",
            "-oPermitLocalCommand=no",
            "-oClearAllForwardings=yes",
        ] {
            assert!(args.contains(&OsString::from(argument)));
        }
    }
}
