//! Explicit security policy. TLS failures never fall back to plaintext.
use crate::VncError;
use std::{
    io,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

#[derive(Clone)]
pub enum SecurityPolicy {
    Plain {
        allow_no_auth: bool,
    },
    Tls {
        server_name: String,
        config: Arc<rustls::ClientConfig>,
    },
}
impl Default for SecurityPolicy {
    fn default() -> Self {
        Self::Plain {
            allow_no_auth: true,
        }
    }
}

pub(crate) enum Secured<S> {
    Plain(Option<S>),
    Tls(Box<tokio_rustls::client::TlsStream<S>>),
}
impl<S: AsyncRead + AsyncWrite + Unpin> Secured<S> {
    pub(crate) async fn upgrade(
        &mut self,
        name: &str,
        config: Arc<rustls::ClientConfig>,
    ) -> Result<(), VncError> {
        let name = rustls::pki_types::ServerName::try_from(name.to_owned())
            .map_err(|_| VncError::General("Invalid TLS server name".into()))?;
        let stream = match self {
            Self::Plain(s) => s.take(),
            Self::Tls(_) => None,
        }
        .ok_or_else(|| VncError::General("Invalid TLS transport state".into()))?;
        let tls = tokio_rustls::TlsConnector::from(config)
            .connect(name, stream)
            .await?;
        *self = Self::Tls(Box::new(tls));
        Ok(())
    }
}
fn closed() -> io::Error {
    io::Error::new(
        io::ErrorKind::NotConnected,
        "TLS negotiation did not finish",
    )
}
impl<S: AsyncRead + AsyncWrite + Unpin> AsyncRead for Secured<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Plain(Some(s)) => Pin::new(s).poll_read(cx, buf),
            Self::Tls(s) => Pin::new(s.as_mut()).poll_read(cx, buf),
            _ => Poll::Ready(Err(closed())),
        }
    }
}
impl<S: AsyncRead + AsyncWrite + Unpin> AsyncWrite for Secured<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            Self::Plain(Some(s)) => Pin::new(s).poll_write(cx, buf),
            Self::Tls(s) => Pin::new(s.as_mut()).poll_write(cx, buf),
            _ => Poll::Ready(Err(closed())),
        }
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Plain(Some(s)) => Pin::new(s).poll_flush(cx),
            Self::Tls(s) => Pin::new(s.as_mut()).poll_flush(cx),
            _ => Poll::Ready(Err(closed())),
        }
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Plain(Some(s)) => Pin::new(s).poll_shutdown(cx),
            Self::Tls(s) => Pin::new(s.as_mut()).poll_shutdown(cx),
            _ => Poll::Ready(Err(closed())),
        }
    }
}
