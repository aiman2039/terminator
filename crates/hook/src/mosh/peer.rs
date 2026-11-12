//! The address the shared SSH master is connected to. `mosh-server -s`
//! binds the server address of that SSH connection, so the client must
//! use the same host: a second DNS lookup can pick another round-robin
//! node or address family. Read locally from the master's TCP socket
//! (`ssh -O check` for its pid, `lsof` for the peer); nothing runs on the
//! host.
use std::{
    net::IpAddr,
    path::PathBuf,
    process::{Command, Stdio},
};
use terminator_core::{find_executable, ssh};

/// The peer address of the master behind `options`, when it connects to
/// the host on `port` itself (no ProxyJump or ProxyCommand).
pub(super) fn master_peer(options: &[String], host: &str, port: u16) -> Option<IpAddr> {
    let check = ssh::scrubbed(Command::new(find_executable("ssh")?))
        .args(options)
        .args(["-O", "check", "--", host])
        .stdin(Stdio::null())
        .output()
        .ok()?;
    if !check.status.success() {
        return None;
    }
    let pid = master_pid(&String::from_utf8_lossy(&check.stderr))
        .or_else(|| master_pid(&String::from_utf8_lossy(&check.stdout)))?;
    tcp_peer(pid, port)
}

/// The first established TCP peer of process `pid` on `port`. The SSH
/// connection is the master's first socket; forwarded connections come later.
pub(super) fn tcp_peer(pid: u32, port: u16) -> Option<IpAddr> {
    let output = Command::new(lsof()?)
        .args(["-a", "-n", "-P", "-iTCP", "-sTCP:ESTABLISHED", "-Fn", "-p"])
        .arg(pid.to_string())
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    connection_peer(&String::from_utf8_lossy(&output.stdout), port)
}

/// `lsof` on PATH or in a system folder a non-login PATH may miss.
fn lsof() -> Option<PathBuf> {
    find_executable("lsof").or_else(|| {
        ["/usr/sbin/lsof", "/usr/bin/lsof", "/sbin/lsof"]
            .into_iter()
            .map(PathBuf::from)
            .find(|path| path.is_file())
    })
}

/// `Master running (pid=1234)` from `ssh -O check`.
fn master_pid(text: &str) -> Option<u32> {
    let rest = text.split("(pid=").nth(1)?;
    rest.split(')').next()?.trim().parse().ok()
}

/// The first `n<local>-><peer>` line of `lsof -Fn` whose peer is on `port`.
fn connection_peer(lsof: &str, port: u16) -> Option<IpAddr> {
    lsof.lines().find_map(|line| {
        let (_, peer) = line.strip_prefix('n')?.split_once("->")?;
        let (address, peer_port) = peer.rsplit_once(':')?;
        if peer_port.parse::<u16>().ok()? != port {
            return None;
        }
        address
            .trim_start_matches('[')
            .trim_end_matches(']')
            .parse()
            .ok()
    })
}

/// Whether `ssh -G` output sends the connection through a proxy: then the
/// master's socket does not reach the host.
pub(super) fn proxied(resolved: &str) -> bool {
    resolved.lines().any(|line| {
        line.split_once(' ').is_some_and(|(key, value)| {
            matches!(key, "proxyjump" | "proxycommand")
                && !value.trim().eq_ignore_ascii_case("none")
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_master_peer_is_the_connection_on_the_ssh_port() {
        let lsof =
            "p42\nf3\nn192.0.2.10:50111->198.51.100.7:22\nf7\nn127.0.0.1:8080->127.0.0.1:50999\n";
        assert_eq!(connection_peer(lsof, 22), "198.51.100.7".parse().ok());
        let v6 = "p42\nf3\nn[2001:db8::2]:50111->[2001:db8::9]:2222\n";
        assert_eq!(connection_peer(v6, 2222), "2001:db8::9".parse().ok());
        assert_eq!(connection_peer(v6, 22), None);
        assert_eq!(master_pid("Master running (pid=4242)\r\n"), Some(4242));
        assert_eq!(master_pid("Control socket connect: No such file"), None);
    }

    #[test]
    fn a_proxied_host_does_not_use_the_master_socket() {
        assert!(!proxied("hostname example.org\nport 22\n"));
        assert!(!proxied("proxycommand none\nproxyjump none\n"));
        assert!(proxied("proxyjump bastion\n"));
        assert!(proxied("proxycommand nc %h %p\n"));
    }

    #[test]
    fn the_peer_of_a_live_connection_is_read_from_lsof() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let _client = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        let _server = listener.accept().unwrap();
        if lsof().is_none() {
            return; // No lsof on this machine.
        }
        assert_eq!(
            tcp_peer(std::process::id(), port),
            Some(IpAddr::from([127, 0, 0, 1]))
        );
    }
}
