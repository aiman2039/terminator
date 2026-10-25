use super::{
    auth::{AuthHelper, AuthResult, SecurityType},
    secure_transport::{Secured, SecurityPolicy},
};
use crate::{VncError, VncVersion};
use std::future::Future;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub(super) async fn authenticate<S, F>(
    stream: &mut Secured<S>,
    version: VncVersion,
    policy: &SecurityPolicy,
    password: &mut Option<F>,
    username: &str,
) -> Result<(), VncError>
where
    S: AsyncRead + AsyncWrite + Unpin,
    F: Future<Output = Result<String, VncError>>,
{
    let types = SecurityType::read(stream, &version).await?;
    match policy {
        SecurityPolicy::Tls {
            server_name,
            config,
        } => {
            if version != VncVersion::RFB38 || !types.contains(&SecurityType::VeNCrypt) {
                return Err(VncError::General("Server does not offer certificate-validated VeNCrypt; plaintext fallback is disabled".into()));
            }
            stream.write_u8(19).await?;
            let mut v = [0; 2];
            stream.read_exact(&mut v).await?;
            if v != [0, 2] {
                return Err(VncError::General("Only VeNCrypt 0.2 is supported".into()));
            }
            stream.write_all(&[0, 2]).await?;
            if stream.read_u8().await? != 0 {
                return Err(VncError::General("VeNCrypt version rejected".into()));
            }
            let count = stream.read_u8().await?;
            let mut offered = Vec::with_capacity(usize::from(count));
            for _ in 0..count {
                offered.push(stream.read_u32().await?);
            }
            let selected = if !username.is_empty() && offered.contains(&262) {
                262
            } else if offered.contains(&261) {
                261
            } else {
                return Err(VncError::General(
                    "Server does not offer X509Vnc or X509Plain; anonymous TLS is not accepted"
                        .into(),
                ));
            };
            stream.write_u32(selected).await?;
            if stream.read_u8().await? != 1 {
                return Err(VncError::General("VeNCrypt subtype rejected".into()));
            }
            stream.upgrade(server_name, Arc::clone(config)).await?;
            let secret =
                zeroize::Zeroizing::new(password.take().ok_or(VncError::NoPassword)?.await?);
            if selected == 261 {
                let auth = AuthHelper::read(stream, &secret).await?;
                auth.write(stream).await?;
                if matches!(auth.finish(stream).await?, AuthResult::Failed) {
                    return Err(auth_error(stream, version).await?);
                }
            } else {
                if username.len() > 4096 || secret.len() > 4096 {
                    return Err(VncError::General("Credential exceeds 4096 bytes".into()));
                }
                stream.write_u32(username.len() as u32).await?;
                stream.write_u32(secret.len() as u32).await?;
                stream.write_all(username.as_bytes()).await?;
                stream.write_all(secret.as_bytes()).await?;
                if stream.read_u32().await? != 0 {
                    return Err(auth_error(stream, version).await?);
                }
            }
        }
        SecurityPolicy::Plain { allow_no_auth } => {
            // Prefer authentication even if a server also offers None.
            if types.contains(&SecurityType::VncAuth) {
                if version != VncVersion::RFB33 {
                    stream.write_u8(2).await?;
                }
                let secret =
                    zeroize::Zeroizing::new(password.take().ok_or(VncError::NoPassword)?.await?);
                let auth = AuthHelper::read(stream, &secret).await?;
                auth.write(stream).await?;
                if matches!(auth.finish(stream).await?, AuthResult::Failed) {
                    return Err(auth_error(stream, version).await?);
                }
            } else if *allow_no_auth && types.contains(&SecurityType::None) {
                if version != VncVersion::RFB33 {
                    stream.write_u8(1).await?;
                }
                if version == VncVersion::RFB38 && stream.read_u32().await? != 0 {
                    return Err(auth_error(stream, version).await?);
                }
            } else {
                return Err(VncError::General(
                    "Server offers no permitted authentication mechanism".into(),
                ));
            }
        }
    }
    Ok(())
}
use std::sync::Arc;
async fn auth_error<S: AsyncRead + Unpin>(
    stream: &mut S,
    version: VncVersion,
) -> Result<VncError, VncError> {
    if version == VncVersion::RFB38 {
        let reason = crate::limits::string(stream, crate::limits::MAX_NAME).await?;
        Ok(VncError::General(format!(
            "Authentication failed: {reason}"
        )))
    } else {
        Ok(VncError::WrongPassword)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    fn tls_policy() -> SecurityPolicy {
        let config = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(rustls::RootCertStore::empty())
        .with_no_client_auth();
        SecurityPolicy::Tls {
            server_name: "localhost".into(),
            config: Arc::new(config),
        }
    }
    #[tokio::test]
    async fn tls_never_selects_a_plaintext_offer() {
        let (client, mut server) = tokio::io::duplex(64);
        server.write_all(&[2, 1, 2]).await.unwrap();
        let mut secured = Secured::Plain(Some(client));
        let mut password = Some(async { Ok("fixture".to_owned()) });
        assert!(authenticate(
            &mut secured,
            VncVersion::RFB38,
            &tls_policy(),
            &mut password,
            ""
        )
        .await
        .is_err());
        drop(secured);
        let mut received = Vec::new();
        server.read_to_end(&mut received).await.unwrap();
        assert!(received.is_empty());
        assert!(
            password.is_some(),
            "credentials were not requested before TLS was selected"
        );
    }
    #[tokio::test]
    async fn authentication_is_preferred_when_none_is_also_offered() {
        let (client, mut server) = tokio::io::duplex(128);
        let peer = tokio::spawn(async move {
            server.write_all(&[2, 1, 2]).await.unwrap();
            assert_eq!(server.read_u8().await.unwrap(), 2);
            server.write_all(&[0u8; 16]).await.unwrap();
            let mut response = [0u8; 16];
            server.read_exact(&mut response).await.unwrap();
            server.write_u32(0).await.unwrap();
        });
        let mut secured = Secured::Plain(Some(client));
        let mut password = Some(async { Ok("fixture".to_owned()) });
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            authenticate(
                &mut secured,
                VncVersion::RFB38,
                &SecurityPolicy::Plain {
                    allow_no_auth: false,
                },
                &mut password,
                "",
            ),
        )
        .await
        .unwrap()
        .unwrap();
        peer.await.unwrap();
    }
    #[tokio::test]
    async fn unauthenticated_connections_require_explicit_permission() {
        let (client, mut server) = tokio::io::duplex(64);
        server.write_all(&[1, 1]).await.unwrap();
        let mut secured = Secured::Plain(Some(client));
        let mut password = Some(async { Ok("fixture".into()) });
        assert!(authenticate(
            &mut secured,
            VncVersion::RFB38,
            &SecurityPolicy::Plain {
                allow_no_auth: false
            },
            &mut password,
            ""
        )
        .await
        .is_err());
    }
}
