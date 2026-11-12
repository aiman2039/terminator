//! The ntfy access token in the OS credential store. Only the ntfy worker
//! thread reads or writes it, never an IPC request with a short deadline.
use anyhow::{Context, Result, ensure};
use std::path::Path;
use zeroize::Zeroizing;

/// Where the token lives. Tests use an in-memory store; they never touch
/// the real keychain.
pub(crate) trait TokenStore: Send {
    fn load(&mut self) -> Result<Option<Zeroizing<String>>>;
    /// `None` removes the token.
    fn save(&mut self, token: Option<&str>) -> Result<()>;
}

/// An ntfy access token: 1–256 printable ASCII characters without spaces
/// or quotes, so it is safe in a curl config line.
pub(crate) fn valid(token: &str) -> bool {
    (1..=256).contains(&token.len())
        && token
            .bytes()
            .all(|b| b.is_ascii_graphic() && b != b'"' && b != b'\\')
}

#[cfg(test)]
#[test]
fn a_token_that_could_break_the_curl_config_is_refused() {
    assert!(valid("tk_AgQdq7mVBoFD37zQVN29RhuMzNIz2"));
    for bad in ["", "has space", "quote\"d", "back\\slash", "line\nbreak"] {
        assert!(!valid(bad), "{bad:?}");
    }
    assert!(!valid(&"a".repeat(257)));
}

/// The OS credential store entry, one per data directory.
pub(crate) struct Keychain {
    service: String,
}

impl Keychain {
    pub(crate) fn new(data: &Path) -> Result<Self> {
        let root = data
            .canonicalize()
            .context("Credential namespace is unavailable")?;
        let root = root
            .to_str()
            .context("Credential namespace must be valid UTF-8")?;
        Ok(Self {
            service: format!("dev.terminator.ntfy:{root}"),
        })
    }

    fn entry(&self) -> Result<keyring::Entry> {
        keyring::Entry::new(&self.service, "token").context("OS credential store is unavailable")
    }
}

impl TokenStore for Keychain {
    fn load(&mut self) -> Result<Option<Zeroizing<String>>> {
        match self.entry()?.get_password() {
            Ok(token) => Ok(Some(Zeroizing::new(token))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(error).context("Saved ntfy token is unavailable"),
        }
    }

    fn save(&mut self, token: Option<&str>) -> Result<()> {
        let entry = self.entry()?;
        match token {
            Some(token) => {
                ensure!(valid(token), "Invalid ntfy token");
                entry
                    .set_password(token)
                    .context("Cannot save the ntfy token in the OS credential store")
            }
            None => match entry.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(error) => Err(error).context("Cannot remove the saved ntfy token"),
            },
        }
    }
}
