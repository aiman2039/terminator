//! `terminator-hook mosh SSH_ARGS... -- HOST REMOTE_COMMAND` runs in an SSH
//! project's PTY. OpenSSH (sharing the project's master, prompting here when
//! it must) runs the daemon-built `mosh-server` command on the host; the
//! local `mosh-client` then replaces this process and carries the session
//! over UDP.
use anyhow::{Context, Result, bail, ensure};
use std::{
    io::{BufRead, BufReader, Write},
    net::{IpAddr, ToSocketAddrs},
    process::{Command, Stdio},
};
use terminator_core::{find_executable, ssh};

pub(crate) fn run(args: &[String]) -> Result<()> {
    let split = args
        .iter()
        .position(|arg| arg == "--")
        .context("Expected SSH arguments, --, host and remote command")?;
    let (options, rest) = args.split_at(split);
    let [_, host, remote] = rest else {
        bail!("Expected SSH arguments, --, host and remote command");
    };
    let client = find_executable("mosh-client")
        .context("mosh-client is not installed. Install Mosh on this computer (for example `brew install mosh`).")?;
    let address = endpoint(options, host)?;
    let (port, key) = start_server(options, host, remote)?;
    connect(&client, address, port, &key)
}

/// The address OpenSSH would connect to after `~/.ssh/config`, resolved
/// here: Mosh talks UDP to it directly, not through a jump host.
fn endpoint(options: &[String], host: &str) -> Result<IpAddr> {
    let output = ssh::resolve_command(options, host)
        .output()
        .context("Run ssh -G")?;
    ensure!(
        output.status.success(),
        "Cannot read the SSH host configuration"
    );
    let resolved = ssh::parse_resolved(std::str::from_utf8(&output.stdout)?)?;
    let addresses = (resolved.hostname.as_str(), 0)
        .to_socket_addrs()
        .with_context(|| format!("Cannot resolve {}", resolved.hostname))?
        .map(|address| address.ip());
    pick_address(addresses, resolved.address_family.as_deref())
        .with_context(|| format!("{} has no usable address", resolved.hostname))
}

/// The first address of the family `AddressFamily` allows: with `inet`
/// SSH (and so `mosh-server -s`) uses IPv4 even when IPv6 resolves first.
fn pick_address(
    mut addresses: impl Iterator<Item = IpAddr>,
    family: Option<&str>,
) -> Option<IpAddr> {
    addresses.find(|address| match family {
        Some("inet") => address.is_ipv4(),
        Some("inet6") => address.is_ipv6(),
        _ => true,
    })
}

/// Runs `mosh-server` over SSH and returns its UDP port and session key.
/// Other output (errors, a missing `mosh-server`) is shown in the terminal.
fn start_server(options: &[String], host: &str, command: &str) -> Result<(u16, String)> {
    let mut child = ssh::scrubbed(Command::new(
        find_executable("ssh").context("OpenSSH client is not installed")?,
    ))
    .args(options)
    .args(["--", host, command])
    .stdin(Stdio::inherit())
    .stdout(Stdio::piped())
    .stderr(Stdio::inherit())
    .spawn()
    .context("Run ssh")?;
    let stdout = child.stdout.take().context("ssh output")?;
    let mut found = None;
    let mut stderr = std::io::stderr();
    for line in BufReader::new(stdout).lines() {
        let line = line?;
        match parse_connect(&line) {
            Some(connect) if found.is_none() => found = Some(connect),
            _ => {
                let _ = writeln!(stderr, "{}\r", line.trim_end_matches('\r'));
            }
        }
    }
    let status = child.wait()?;
    found.with_context(|| {
        format!(
            "mosh-server did not start ({status}). Install Mosh on the host and allow UDP ports 60000-61000."
        )
    })
}

/// `MOSH CONNECT <port> <key>` from `mosh-server new`.
fn parse_connect(line: &str) -> Option<(u16, String)> {
    let mut words = line
        .trim()
        .strip_prefix("MOSH CONNECT ")?
        .split_whitespace();
    let port = words.next()?.parse::<u16>().ok().filter(|p| *p > 0)?;
    let key = words.next()?;
    let valid = key.len() == 22
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "+/".contains(c));
    (valid && words.next().is_none()).then(|| (port, key.to_owned()))
}

#[cfg(unix)]
fn connect(client: &std::path::Path, address: IpAddr, port: u16, key: &str) -> Result<()> {
    use std::os::unix::process::CommandExt;
    let mut command = Command::new(client);
    command
        .arg(address.to_string())
        .arg(port.to_string())
        .env("MOSH_KEY", key);
    if terminator_core::remote::mosh::utf8_locale_vars().is_empty() {
        // mosh-client refuses a non-UTF-8 locale.
        command.env(
            "LC_CTYPE",
            if cfg!(target_os = "macos") {
                "en_US.UTF-8"
            } else {
                "C.UTF-8"
            },
        );
    }
    Err(command.exec()).context("Run mosh-client")
}

#[cfg(not(unix))]
fn connect(_: &std::path::Path, _: IpAddr, _: u16, _: &str) -> Result<()> {
    bail!("Mosh is not supported on this platform")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mosh_targets_the_address_family_ssh_config_selects() {
        let v6: IpAddr = "2001:db8::1".parse().unwrap();
        let v4: IpAddr = "192.0.2.1".parse().unwrap();
        let both = || [v6, v4].into_iter();
        assert_eq!(pick_address(both(), Some("inet")), Some(v4));
        assert_eq!(pick_address(both(), Some("inet6")), Some(v6));
        assert_eq!(pick_address(both(), None), Some(v6));
        assert_eq!(pick_address([v6].into_iter(), Some("inet")), None);
    }

    #[test]
    fn mosh_connect_line_yields_port_and_key() {
        assert_eq!(
            parse_connect("MOSH CONNECT 60001 4NeCCgvZFe2RnPgrcU1PQw\r"),
            Some((60001, "4NeCCgvZFe2RnPgrcU1PQw".to_owned()))
        );
        assert_eq!(parse_connect("MOSH CONNECT 0 4NeCCgvZFe2RnPgrcU1PQw"), None);
        assert_eq!(parse_connect("MOSH CONNECT 60001 short"), None);
        assert_eq!(parse_connect("mosh-server: command not found"), None);
    }
}
