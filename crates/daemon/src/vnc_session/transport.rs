//! OpenSSH owns authentication, host-key verification, agent use and SSH config.
use anyhow::{Context, Result};
use rustls::pki_types::pem::PemObject;
use rustls_platform_verifier::BuilderVerifierExt;
use std::{
    io,
    pin::Pin,
    process::Stdio,
    sync::Arc,
    task::{Context as TaskContext, Poll},
    time::Duration,
};
use terminator_core::vnc::{SshOptions, Transport, VncOptions, validate_host};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::TcpStream,
    process::{Child, ChildStdin, ChildStdout, Command},
};

pub(super) struct SshStream {
    child: Child,
    stdin: ChildStdin,
    stdout: ChildStdout,
}
impl Drop for SshStream {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}
impl AsyncRead for SshStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().stdout).poll_read(cx, buf)
    }
}
impl AsyncWrite for SshStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().stdin).poll_write(cx, buf)
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().stdin).poll_flush(cx)
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().stdin).poll_shutdown(cx)
    }
}
pub(super) trait Io: AsyncRead + AsyncWrite + Unpin + Send + Sync {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send + Sync> Io for T {}
pub(super) type Stream = Box<dyn Io>;

pub(super) fn ssh_command(ssh: &SshOptions) -> Result<Command> {
    let mut command = Command::new("ssh");
    command
        .args(terminator_core::ssh::arguments(ssh, false)?)
        .kill_on_drop(true);
    for key in terminator_core::ssh::private_environment_keys() {
        command.env_remove(key);
    }
    Ok(command)
}
pub(super) async fn connect(host: &str, port: u16, options: &VncOptions) -> Result<Stream> {
    validate_host(host)?;
    options.validate()?;
    if options.transport == Transport::Ssh {
        let target = if host.contains(':') {
            format!("[{host}]:{port}")
        } else {
            format!("{host}:{port}")
        };
        let mut command = ssh_command(&options.ssh)?;
        command
            .arg("-W")
            .arg(target)
            .arg("--")
            .arg(&options.ssh.host)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = command
            .spawn()
            .context("Cannot start OpenSSH; install ssh and configure agent/key authentication")?;
        let stdin = child.stdin.take().context("SSH stdin unavailable")?;
        let stdout = child.stdout.take().context("SSH stdout unavailable")?;
        Ok(Box::new(SshStream {
            child,
            stdin,
            stdout,
        }))
    } else {
        let tcp = tokio::time::timeout(Duration::from_secs(10), TcpStream::connect((host, port)))
            .await
            .context("VNC connection timed out")??;
        tcp.set_nodelay(true)?;
        socket2::SockRef::from(&tcp).set_tcp_keepalive(
            &socket2::TcpKeepalive::new()
                .with_time(Duration::from_secs(30))
                .with_interval(Duration::from_secs(10)),
        )?;
        Ok(Box::new(tcp))
    }
}
pub(super) fn security(host: &str, options: &VncOptions) -> Result<vnc::SecurityPolicy> {
    if options.transport != Transport::Tls {
        return Ok(vnc::SecurityPolicy::Plain {
            allow_no_auth: options.allow_no_auth,
        });
    }
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?;
    let config = if let Some(path) = &options.tls_ca_file {
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(
            &mut std::io::Read::take(
                std::fs::File::open(path).context("Cannot read TLS CA file")?,
                1024 * 1024 + 1,
            ),
            &mut bytes,
        )?;
        anyhow::ensure!(bytes.len() <= 1024 * 1024, "TLS CA file exceeds 1 MiB");
        let mut roots = rustls::RootCertStore::empty();
        for certificate in rustls::pki_types::CertificateDer::pem_slice_iter(&bytes) {
            roots.add(certificate?)?;
        }
        anyhow::ensure!(!roots.is_empty(), "TLS CA file contains no certificates");
        builder.with_root_certificates(roots).with_no_client_auth()
    } else {
        builder.with_platform_verifier()?.with_no_client_auth()
    };
    Ok(vnc::SecurityPolicy::Tls {
        server_name: if options.tls_server_name.is_empty() {
            host.into()
        } else {
            options.tls_server_name.clone()
        },
        config: Arc::new(config),
    })
}
