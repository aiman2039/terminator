//! First-connect host-key trust against the Docker SSH fixture. Skips like
//! `cargo xtask ssh` when Docker or the fixture is unavailable.
use super::CaptureInput;
use super::{Options, Result, capture, ensure, fs, json};
use crate::ssh_fixture::{Fixture, add, remote_project, unavailable};

pub fn run(o: &Options) -> Result<()> {
    if let Some(reason) = unavailable() {
        println!("Skipping ssh-hostkey: {reason}");
        return Ok(());
    }
    let fixture = Fixture::start()?;
    let h = fixture.h();
    h.setup()?;
    let known_hosts = fixture.temp.path().join("known_hosts");
    fs::write(&known_hosts, "")?;
    let ssh = fixture.ssh(fixture.port, &known_hosts);
    // Selected on add, so the GUI opens it, lists the root, gets
    // UnknownHostKey and scans the key into the trust dialog.
    add(h, &remote_project("ssh-hostkey", ssh))?;
    let logs = capture(
        CaptureInput {
            h,
            opts: o,
            name: "ssh-hostkey",
            actions: json!([{"at_ms":1500,"target":"ssh-hostkey-trust"}]),
            after: 7000,
        },
        |_| Ok(()),
    )?;
    let trusted = fs::read_to_string(&known_hosts)?;
    ensure!(
        trusted.starts_with(&format!("[127.0.0.1]:{} ssh-ed25519 ", fixture.port)),
        "Trust and connect did not write known_hosts: {trusted:?}\n{logs}"
    );
    Ok(())
}
