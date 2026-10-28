use super::{
    BIG_BYTES, Fixture, MAIN, PW, ROOT, add, forwards, free_port, http_get, operation, probe,
    remote_project, rpc, sha256_file, ssh_failure, trust, wait_transfer, write_big,
};
use anyhow::{Context, Result, bail, ensure};
use std::{
    fs,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};
use terminator_core::{
    Request, Response,
    remote::{Forward, Operation, RemoteProject, Reply},
    ssh::{SshFailure, SshOptions},
};

/// The main fixture service's SSH options and its remote project.
struct Main<'a> {
    ssh: &'a SshOptions,
    project: &'a RemoteProject,
}

pub(super) fn steps(fixture: &mut Fixture) -> Result<serde_json::Value> {
    let fixture: &Fixture = fixture;
    let mut report = serde_json::Map::new();
    let h = fixture.h();
    let known_hosts = fixture.temp.path().join("known_hosts");
    fs::write(&known_hosts, "")?;
    let ssh = fixture.ssh(fixture.port, &known_hosts);
    let project = remote_project("ssh-fixture", ssh.clone());
    let main = Main {
        ssh: &ssh,
        project: &project,
    };

    // (a) Empty known_hosts → UnknownHostKey.
    let failure = ssh_failure(probe(h, &project))?;
    ensure!(
        failure == SshFailure::UnknownHostKey,
        "Expected UnknownHostKey, got {failure:?}"
    );
    println!("a) probe with empty known_hosts: {failure:?}");
    report.insert("a_unknown_host_key".into(), true.into());
    let fingerprint = trusted_probe(fixture, &main)?;
    report.insert("b_trusted_fingerprint".into(), fingerprint.into());
    let (id, control, opened) = reused_master(fixture, &main)?;
    report.insert("c_new_connections_for_20_calls".into(), opened.into());
    let pw_id = forward(fixture)?;
    report.insert("d_forward_fetch".into(), true.into());
    let local_hash = transfers(fixture, &id)?;
    report.insert("e_sha256".into(), local_hash.into());
    changed_host_key(fixture, &main, (&id, &control))?;
    report.insert("f_changed_host_key".into(), true.into());
    rpc(h, Request::RemoveRemoteProject { project: pw_id })?;
    Ok(report.into())
}

// (b) Scan + trust → probe succeeds. Returns the trusted fingerprint.
fn trusted_probe(fixture: &Fixture, main: &Main<'_>) -> Result<String> {
    let h = fixture.h();
    let Main { ssh, project } = *main;
    let fingerprint = trust(fixture, MAIN, ssh)?;
    let Response::RemoteProject(Reply::Probe { root }) = probe(h, project)? else {
        bail!("Probe did not reply Probe");
    };
    ensure!(root == ROOT, "Probe resolved {root}");
    println!("b) trusted {fingerprint}; probe resolved {root}");
    Ok(fingerprint)
}

// (c) 20 list/read/git calls ride on one master. Returns the project ID,
// its control socket, and the number of new SSH connections.
fn reused_master(fixture: &Fixture, main: &Main<'_>) -> Result<(String, PathBuf, usize)> {
    let h = fixture.h();
    let Main { ssh, project } = *main;
    let id = add(h, project)?;
    let control = terminator_core::Paths::at(h.root.clone())
        .ssh_control(ssh)
        .context("control path")?;
    let before = fixture.connections(MAIN)?;
    for index in 0..20u32 {
        let op = match index % 4 {
            0 => Operation::List {
                path: String::new(),
            },
            1 => Operation::Read {
                path: "README.md".into(),
            },
            2 => Operation::GitStatus,
            _ => Operation::GitDiff {
                path: "README.md".into(),
                staged: false,
            },
        };
        let reply = operation(h, &id, op)?;
        if let Reply::Git { changes, .. } = &reply {
            ensure!(
                changes.iter().any(|c| c.path == "README.md"),
                "git status lost the fixture change: {changes:?}"
            );
        }
    }
    let after = fixture.connections(MAIN)?;
    let opened = after.saturating_sub(before);
    ensure!(
        control.exists(),
        "No control socket at {}",
        control.display()
    );
    ensure!(
        opened <= 1,
        "20 calls opened {opened} SSH connections; expected the master to be reused"
    );
    println!("c) 20 list/read/git calls opened {opened} new sshd connection(s) ({after} total)");
    Ok((id, control, opened))
}

// (d) Forward through the `AllowTcpForwarding local` variant. Returns the
// forwarding project's ID.
fn forward(fixture: &Fixture) -> Result<String> {
    let h = fixture.h();
    let pw_known = fixture.temp.path().join("known_hosts_pw");
    fs::write(&pw_known, "")?;
    let pw_ssh = fixture.ssh(fixture.pw_port, &pw_known);
    trust(fixture, PW, &pw_ssh)?;
    let pw_id = add(h, &remote_project("ssh-fixture-pw", pw_ssh))?;
    let local_port = free_port()?;
    rpc(
        h,
        Request::SetRemoteProjectForwards {
            project: pw_id.clone(),
            forwards: vec![Forward {
                local_port,
                remote_host: "127.0.0.1".into(),
                remote_port: 8000,
                auto_start: false,
            }],
        },
    )?;
    let status = forwards(rpc(
        h,
        Request::StartForward {
            project: pw_id.clone(),
            local_port,
        },
    )?)?;
    ensure!(
        status.is_active(&pw_id, local_port),
        "Forward not active: {status:?}"
    );
    let body = http_get(local_port, "/marker.txt")?;
    ensure!(
        body.starts_with("HTTP/1.0 200") && body.contains("terminator-forward-ok"),
        "Forwarded fetch failed: {body}"
    );
    stop_forward(fixture, &pw_id, local_port)?;
    println!("d) forward 127.0.0.1:{local_port} → 127.0.0.1:8000 served marker; stopped");
    Ok(pw_id)
}

fn stop_forward(fixture: &Fixture, pw_id: &str, local_port: u16) -> Result<()> {
    let h = fixture.h();
    let status = forwards(rpc(h, Request::ForwardStatus)?)?;
    ensure!(
        status.is_active(pw_id, local_port),
        "ForwardStatus lost the forward: {status:?}"
    );
    let status = forwards(rpc(
        h,
        Request::StopForward {
            project: pw_id.to_owned(),
            local_port,
        },
    )?)?;
    ensure!(
        !status.is_active(pw_id, local_port),
        "Forward still active: {status:?}"
    );
    let end = Instant::now()
        .checked_add(Duration::from_secs(5))
        .context("deadline overflow")?;
    while http_get(local_port, "/marker.txt").is_ok_and(|b| b.contains("terminator-forward-ok")) {
        ensure!(Instant::now() < end, "Stopped forward still serves");
        thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}

// (e) 50 MB upload + download with matching SHA-256. Returns the hash.
fn transfers(fixture: &Fixture, id: &str) -> Result<String> {
    let h = fixture.h();
    let upload_dir = fixture.temp.path().join("upload");
    fs::create_dir_all(&upload_dir)?;
    let big = upload_dir.join("big.bin");
    write_big(&big)?;
    let local_hash = sha256_file(&big)?;
    let started = Instant::now();
    let Reply::Transfer(status) = operation(
        h,
        id,
        Operation::Upload {
            local: big.clone(),
            remote_dir: String::new(),
        },
    )?
    else {
        bail!("Upload did not start");
    };
    let status = wait_transfer(h, id, &status.id)?;
    let up_secs = started.elapsed().as_secs_f64();
    ensure!(
        status.bytes == BIG_BYTES as u64,
        "Upload reported {} bytes",
        status.bytes
    );
    let remote_hash = fixture.exec(MAIN, &format!("sha256sum {ROOT}/big.bin"))?;
    ensure!(
        remote_hash.split_whitespace().next() == Some(local_hash.as_str()),
        "Remote hash {remote_hash} differs from {local_hash}"
    );
    let down_secs = download_twice(fixture, id, &local_hash)?;
    println!("e) 50 MiB upload {up_secs:.1}s, download {down_secs:.1}s, sha256 {local_hash}");
    Ok(local_hash)
}

/// Download and verify the uploaded file, then check a second download refuses
/// to overwrite it. Returns the download time in seconds.
fn download_twice(fixture: &Fixture, id: &str, local_hash: &str) -> Result<f64> {
    let h = fixture.h();
    let download = fixture.temp.path().join("download");
    fs::create_dir_all(&download)?;
    let downloaded = download.join("big.bin");
    let started = Instant::now();
    let Reply::Transfer(status) = operation(
        h,
        id,
        Operation::Download {
            path: "big.bin".into(),
            local: downloaded.clone(),
        },
    )?
    else {
        bail!("Download did not start");
    };
    wait_transfer(h, id, &status.id)?;
    let down_secs = started.elapsed().as_secs_f64();
    let downloaded_hash = sha256_file(&downloaded)?;
    ensure!(
        downloaded_hash == local_hash,
        "Downloaded hash {downloaded_hash} differs from {local_hash}"
    );
    // Never overwrite: the same download again must fail.
    ensure!(
        operation(
            h,
            id,
            Operation::Download {
                path: "big.bin".into(),
                local: downloaded,
            },
        )
        .is_err(),
        "Download replaced an existing local file"
    );
    Ok(down_secs)
}

// (f) Rekey → ChangedHostKey; trusting the new key is refused.
fn changed_host_key(
    fixture: &Fixture,
    main: &Main<'_>,
    (id, control): (&str, &Path),
) -> Result<()> {
    let h = fixture.h();
    let Main { ssh, project } = *main;
    rpc(
        h,
        Request::RemoveRemoteProject {
            project: id.to_owned(),
        },
    )?;
    let end = Instant::now()
        .checked_add(Duration::from_secs(5))
        .context("deadline overflow")?;
    while control.exists() {
        ensure!(
            Instant::now() < end,
            "Removing the project left its master running"
        );
        thread::sleep(Duration::from_millis(50));
    }
    fixture.script(&["rekey", MAIN])?;
    fixture.wait_listening(fixture.port)?;
    let failure = ssh_failure(probe(h, project))?;
    ensure!(
        failure == SshFailure::ChangedHostKey,
        "Expected ChangedHostKey, got {failure:?}"
    );
    let Response::HostKeys(scan) = rpc(h, Request::ScanHostKey { ssh: ssh.clone() })? else {
        bail!("ScanHostKey did not return keys");
    };
    ensure!(
        rpc(
            h,
            Request::TrustHostKey {
                ssh: ssh.clone(),
                keys: scan.keys
            }
        )
        .is_err(),
        "TrustHostKey accepted a changed host key"
    );
    ensure!(
        ssh_failure(probe(h, project))? == SshFailure::ChangedHostKey,
        "Changed key became trusted"
    );
    println!("f) after rekey: {failure:?}; TrustHostKey refused");
    Ok(())
}
