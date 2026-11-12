//! First-connect host-key trust. Keys are only ever appended after the user
//! saw their fingerprints; a changed key is never replaced here.
use super::*;
use std::io::Write;
use terminator_core::ssh::{self, HostKey, HostKeyScan, SshOptions};

/// `ssh -G` applies `~/.ssh/config`, so aliases resolve to the real endpoint.
fn resolve(options: &SshOptions) -> Result<ssh::Resolved> {
    let output = run_command(
        ssh::resolve_command(ssh::arguments(options, false, None)?, &options.host),
        CommandOptions {
            timeout: Duration::from_secs(5),
            stdout_limit: 256 * 1024,
            stderr_limit: STDERR_LIMIT,
            ..CommandOptions::default()
        },
    )
    .context("Cannot resolve the SSH host configuration")?;
    Ok(ssh::parse_resolved(std::str::from_utf8(&output.stdout)?)?)
}
pub(crate) fn scan(options: &SshOptions) -> Result<HostKeyScan> {
    scan_resolved(options).map(|(_, scan)| scan)
}
fn scan_resolved(options: &SshOptions) -> Result<(ssh::Resolved, HostKeyScan)> {
    let resolved = resolve(options)?;
    let mut command = ssh::scrubbed(Command::new("ssh-keyscan"));
    command
        .args(["-T", "5", "-p"])
        .arg(resolved.port.to_string())
        .arg("--")
        .arg(&resolved.hostname);
    let output = run_command(
        command,
        CommandOptions {
            timeout: Duration::from_secs(15),
            stdout_limit: 256 * 1024,
            stderr_limit: STDERR_LIMIT,
            accepted_exit_codes: None,
            ..CommandOptions::default()
        },
    )
    .context("Cannot run ssh-keyscan")?;
    let mut keys = ssh::parse_keyscan(&String::from_utf8_lossy(&output.stdout));
    ensure!(
        !keys.is_empty(),
        "{}:{} presented no host keys",
        resolved.hostname,
        resolved.port
    );
    let host = resolved.known_hosts_host();
    let input: String = keys
        .iter()
        .map(|key| ssh::known_hosts_line(&host, key))
        .collect();
    let mut command = ssh::scrubbed(Command::new("ssh-keygen"));
    command.args(["-l", "-E", "sha256", "-f", "-"]);
    let output = run_command(
        command,
        CommandOptions {
            timeout: Duration::from_secs(5),
            input: Some(input.into_bytes()),
            stdout_limit: 64 * 1024,
            stderr_limit: STDERR_LIMIT,
            ..CommandOptions::default()
        },
    )
    .context("Cannot fingerprint host keys")?;
    let fingerprints = ssh::parse_fingerprints(&String::from_utf8_lossy(&output.stdout));
    ensure!(
        fingerprints.len() == keys.len(),
        "ssh-keygen fingerprinted {} of {} host keys",
        fingerprints.len(),
        keys.len()
    );
    for (key, fingerprint) in keys.iter_mut().zip(fingerprints) {
        key.fingerprint = fingerprint;
    }
    Ok((resolved, HostKeyScan { host, keys }))
}
/// The file OpenSSH reads first for a host without a project file: the
/// first `UserKnownHostsFile` from `ssh_config`, else `~/.ssh/known_hosts`.
fn known_hosts_path(configured: &[String], home: Option<&Path>) -> Result<PathBuf> {
    let Some(first) = configured.first() else {
        return Ok(home
            .context("Home directory is unavailable")?
            .join(".ssh")
            .join("known_hosts"));
    };
    ensure!(
        first != "/dev/null" && !first.eq_ignore_ascii_case("none"),
        "Your SSH configuration sets UserKnownHostsFile {first} for this host, so a trusted key would not be kept. Sign in through an SSH terminal instead."
    );
    ensure!(
        !first.contains('%') && !first.contains("${"),
        "Your SSH configuration sets UserKnownHostsFile {first} for this host, which Terminator cannot expand. Sign in through an SSH terminal instead."
    );
    let path = match first.strip_prefix("~/") {
        Some(rest) => home.context("Home directory is unavailable")?.join(rest),
        None => PathBuf::from(first),
    };
    ensure!(
        path.is_absolute(),
        "Your SSH configuration sets a relative UserKnownHostsFile {first}. Sign in through an SSH terminal instead."
    );
    Ok(path)
}
fn known_hosts(options: &SshOptions, resolved: &ssh::Resolved) -> Result<PathBuf> {
    if let Some(path) = &options.known_hosts_file {
        return Ok(path.clone());
    }
    let home = ssh::default_known_hosts()
        .and_then(|path| path.parent().and_then(Path::parent).map(Path::to_path_buf));
    let path = known_hosts_path(&resolved.user_known_hosts, home.as_deref())?;
    let directory = path.parent().context("known_hosts has no parent")?;
    if !directory.exists() {
        #[cfg_attr(
            not(unix),
            expect(unused_mut, reason = "only Unix sets the directory mode")
        )]
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        builder.create(directory)?;
    }
    Ok(path)
}
pub(crate) fn trust(options: &SshOptions, accepted: &[HostKey]) -> Result<()> {
    let (resolved, current) = scan_resolved(options)?;
    ensure!(
        ssh::same_keys(&current.keys, accepted),
        "The host presented different keys than the ones shown; nothing was trusted. Check the host and try again."
    );
    let path = known_hosts(options, &resolved)?;
    let known = known_keys(&path, &current.host)?;
    // A changed key is never replaced here, and appending beside it would
    // make OpenSSH accept the new key: it trusts any matching line.
    if let Some(stale) = current.keys.iter().find(|key| {
        known
            .iter()
            .any(|(kind, blob)| *kind == key.key_type && *blob != key.key)
    }) {
        anyhow::bail!(
            "known_hosts already has a different {} key for {}; nothing was trusted. If the change is expected, run: ssh-keygen -R '{}' -f '{}'",
            stale.key_type,
            current.host,
            current.host,
            path.display()
        );
    }
    let missing: Vec<_> = current
        .keys
        .iter()
        .filter(|key| {
            !known
                .iter()
                .any(|(kind, blob)| *kind == key.key_type && *blob == key.key)
        })
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    let mut open = std::fs::OpenOptions::new();
    open.create(true).append(true).read(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut open, 0o600);
    let mut file = open
        .open(&path)
        .with_context(|| format!("Cannot open {}", path.display()))?;
    let mut lines = String::new();
    // A file without a trailing newline would merge our first line into its last.
    if file.metadata()?.len() > 0 {
        use std::io::{Read, Seek, SeekFrom};
        let mut last = [0u8; 1];
        file.seek(SeekFrom::End(-1))?;
        file.read_exact(&mut last)?;
        if last != *b"\n" {
            lines.push('\n');
        }
    }
    for key in missing {
        lines.push_str(&ssh::known_hosts_line(&current.host, key));
    }
    file.write_all(lines.as_bytes())?;
    file.sync_all()?;
    Ok(())
}
/// `(type, key)` pairs `path` already holds for `host`, hashed entries
/// included (`ssh-keygen -F`). A missing file holds none.
fn known_keys(path: &Path, host: &str) -> Result<Vec<(String, String)>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let mut command = ssh::scrubbed(Command::new("ssh-keygen"));
    command.arg("-F").arg(host).arg("-f").arg(path);
    let output = run_command(
        command,
        CommandOptions {
            timeout: Duration::from_secs(5),
            stdout_limit: 1024 * 1024,
            stderr_limit: STDERR_LIMIT,
            // 1: no entry for the host.
            accepted_exit_codes: Some(vec![0, 1]),
            ..CommandOptions::default()
        },
    )
    .context("Cannot read known_hosts")?;
    Ok(parse_known(&String::from_utf8_lossy(&output.stdout)))
}
/// `ssh-keygen -F` lines: `host type key`, optionally after a `@marker`.
fn parse_known(output: &str) -> Vec<(String, String)> {
    output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            if line.starts_with('@') {
                parts.next();
            }
            let (_host, kind, key) = (parts.next()?, parts.next()?, parts.next()?);
            Some((kind.to_owned(), key.to_owned()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{known_hosts_path, parse_known};
    use std::path::{Path, PathBuf};
    #[test]
    fn trusted_keys_go_to_the_known_hosts_file_ssh_config_names() {
        let home = Some(Path::new("/home/dev"));
        assert_eq!(
            known_hosts_path(&[], home).unwrap(),
            PathBuf::from("/home/dev/.ssh/known_hosts")
        );
        let configured = ["~/.ssh/vm_hosts".to_owned(), "/etc/other".to_owned()];
        assert_eq!(
            known_hosts_path(&configured, home).unwrap(),
            PathBuf::from("/home/dev/.ssh/vm_hosts")
        );
        assert_eq!(
            known_hosts_path(&["/srv/hosts".to_owned()], home).unwrap(),
            PathBuf::from("/srv/hosts")
        );
        for refused in ["/dev/null", "none", "~/.ssh/%h_hosts", "relative"] {
            assert!(
                known_hosts_path(&[refused.to_owned()], home).is_err(),
                "{refused}"
            );
        }
    }
    #[test]
    fn known_host_lines_parse_with_markers_and_comments() {
        let output = "# Host [h]:2222 found: line 3\n[h]:2222 ssh-ed25519 AAAA1\n@revoked [h]:2222 ssh-rsa AAAA2\n|1|salt|hash ecdsa-sha2-nistp256 AAAA3 comment\n";
        assert_eq!(
            parse_known(output),
            vec![
                ("ssh-ed25519".into(), "AAAA1".into()),
                ("ssh-rsa".into(), "AAAA2".into()),
                ("ecdsa-sha2-nistp256".into(), "AAAA3".into()),
            ]
        );
    }
}
