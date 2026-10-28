//! OS credential storage. Never create a plaintext fallback for new secrets.
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::sync::{Mutex, OnceLock};
use terminator_core::{Paths, vnc::VncProfile};

fn serialized() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}
pub(super) fn credential_account(profile: &VncProfile) -> Result<String> {
    let identity = serde_json::to_vec(&(
        &profile.host,
        profile.port,
        &profile.username,
        profile.options.transport,
        &profile.options.tls_server_name,
        &profile.options.tls_ca_file,
        &profile.options.ssh,
    ))?;
    Ok(format!("{}:{:x}", profile.id, Sha256::digest(identity)))
}
fn entry(paths: &Paths, profile: &VncProfile) -> Result<keyring::Entry> {
    let id = &profile.id;
    ensure!(
        !id.is_empty()
            && id.len() <= 128
            && id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)),
        "Invalid credential identity"
    );
    let root = paths
        .data
        .canonicalize()
        .context("Credential namespace is unavailable")?;
    keyring::Entry::new(
        &format!(
            "dev.terminator.vnc:{}",
            root.to_str()
                .context("Credential namespace must be valid UTF-8")?
        ),
        &credential_account(profile)?,
    )
    .context("OS credential store is unavailable")
}
pub(crate) fn save(paths: &Paths, profile: &VncProfile, password: &str) -> Result<()> {
    ensure!(password.len() <= 4096, "VNC password exceeds 4096 bytes");
    let _lock = serialized()
        .lock()
        .map_err(|_| anyhow::anyhow!("Credential operation failed"))?;
    let e = entry(paths, profile)?;
    e.set_password(password)
        .context("Cannot save VNC password in the OS credential store")?;
    let check = zeroize::Zeroizing::new(
        e.get_password()
            .context("Cannot verify stored VNC password")?,
    );
    ensure!(
        check.as_str() == password,
        "OS credential verification failed"
    );
    Ok(())
}
pub(crate) struct SaveReplacingInput<'a> {
    pub(crate) paths: &'a Paths,
    pub(crate) previous: Option<&'a VncProfile>,
    pub(crate) profile: &'a VncProfile,
    pub(crate) password: &'a str,
}

/// Publish a verified credential under the new identity, then remove the old
/// identity. If removal fails, retain the old profile and undo the new entry.
pub(crate) fn save_replacing(input: SaveReplacingInput<'_>) -> Result<()> {
    let SaveReplacingInput {
        paths,
        previous,
        profile,
        password,
    } = input;
    replace_with(ReplaceWithInput {
        previous,
        profile,
        password,
        save: |p, secret| save(paths, p, secret),
        forget: |p| forget(paths, p),
    })
}

struct ReplaceWithInput<'a, S, F>
where
    S: FnMut(&VncProfile, &str) -> Result<()>,
    F: FnMut(&VncProfile) -> Result<()>,
{
    previous: Option<&'a VncProfile>,
    profile: &'a VncProfile,
    password: &'a str,
    save: S,
    forget: F,
}
fn replace_with<S, F>(input: ReplaceWithInput<'_, S, F>) -> Result<()>
where
    S: FnMut(&VncProfile, &str) -> Result<()>,
    F: FnMut(&VncProfile) -> Result<()>,
{
    let ReplaceWithInput {
        previous,
        profile,
        password,
        mut save,
        mut forget,
    } = input;
    let previous = previous.filter(|p| p.credential_saved);
    let changed = previous
        .map(|p| Ok::<_, anyhow::Error>(credential_account(p)? != credential_account(profile)?))
        .transpose()?
        .unwrap_or(true);
    if let Err(error) = save(profile, password) {
        // A store can accept the write and then fail its verification read.
        if changed {
            forget(profile).context("Cannot remove the new credential after its save failed")?;
        }
        return Err(error);
    }
    if changed
        && let Some(previous) = previous
        && let Err(error) = forget(previous)
    {
        forget(profile).context(
            "Cannot remove the new credential after the previous credential could not be removed",
        )?;
        return Err(error).context("Cannot replace the previous saved VNC credential");
    }
    Ok(())
}

pub(super) fn load(paths: &Paths, profile: &VncProfile) -> Result<String> {
    let _lock = serialized()
        .lock()
        .map_err(|_| anyhow::anyhow!("Credential operation failed"))?;
    entry(paths,profile)?.get_password().context("Saved VNC password is unavailable; unlock the OS credential store or enter the password again")
}
pub(crate) fn forget(paths: &Paths, profile: &VncProfile) -> Result<()> {
    let _lock = serialized()
        .lock()
        .map_err(|_| anyhow::anyhow!("Credential operation failed"))?;
    match entry(paths, profile)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e).context("Cannot remove saved VNC password"),
    }
}
pub(crate) fn migrate(paths: &Paths, profiles: &mut [VncProfile]) -> usize {
    let mut pending = 0usize;
    for p in profiles {
        if !p.password.is_empty() {
            if save(paths, p, &p.password).is_ok() {
                zeroize::Zeroize::zeroize(&mut p.password);
                p.credential_saved = true;
            } else {
                pending = pending.saturating_add(1);
            }
        }
    }
    pending
}

/// A prepared generation cannot write the shared catalog. Finish verified
/// credential migration only after activation, under the workspace lock.
pub(crate) fn finish_activation(weak: std::sync::Weak<crate::server::Shared>) {
    std::thread::spawn(move || {
        let started = std::time::Instant::now();
        while started.elapsed() < std::time::Duration::from_secs(30) {
            let Some(shared) = weak.upgrade() else {
                return;
            };
            let Some(paths) = shared.catalog_paths.as_ref() else {
                return;
            };
            let result = (|| -> Result<bool> {
                let _lock = terminator_core::generations::coordinate_timeout(
                    paths,
                    std::time::Duration::from_millis(100),
                )?;
                let catalog = terminator_core::generations::Catalog::open(paths)?;
                let mut updated = crate::server::relock(&shared.state).clone();
                if catalog.active()?.as_deref() != Some(&updated.generation) {
                    return Ok(false);
                }
                catalog.refresh(&mut updated)?;
                if !updated.vnc_profiles.iter().any(|p| !p.password.is_empty()) {
                    return Ok(true);
                }
                ensure!(
                    migrate(paths, &mut updated.vnc_profiles) == 0,
                    "Unlock the OS credential store to finish legacy VNC migration"
                );
                catalog.save_workspace(&updated)?;
                {
                    let mut state = crate::server::relock(&shared.state);
                    state.vnc_profiles = updated.vnc_profiles;
                    state.revision = state.revision.saturating_add(1);
                }
                shared.persist()?;
                Ok(true)
            })();
            if matches!(result, Ok(true)) {
                return;
            }
            drop(shared);
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        if let Some(shared) = weak.upgrade() {
            shared.degraded("VNC credential migration is waiting for the active session service and an unlocked OS credential store");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    fn profile() -> VncProfile {
        VncProfile {
            id: "fixture".into(),
            name: "Fixture".into(),
            host: "one.example".into(),
            port: 5900,
            username: String::new(),
            password: String::new(),
            shared: true,
            view_only: false,
            last_connected: 0,
            options: terminator_core::vnc::VncOptions::default(),
            credential_saved: true,
        }
    }

    #[test]
    fn replacing_endpoint_removes_only_the_old_credential_after_verified_save() {
        use std::cell::RefCell;
        use std::collections::HashMap;
        for fail_save in [false, true] {
            for fail_remove in [false, true] {
                let previous = profile();
                let mut next = previous.clone();
                next.host = "two.example".into();
                let old = credential_account(&previous).unwrap();
                let new = credential_account(&next).unwrap();
                let store = RefCell::new(HashMap::from([(old.clone(), "old".to_owned())]));
                let result = replace_with(ReplaceWithInput {
                    previous: Some(&previous),
                    profile: &next,
                    password: "new",
                    save: |p, secret| {
                        store
                            .borrow_mut()
                            .insert(credential_account(p)?, secret.into());
                        ensure!(!fail_save, "Verification failed after write");
                        Ok(())
                    },
                    forget: |p| {
                        let account = credential_account(p)?;
                        ensure!(!(fail_remove && account == old), "Delete failed");
                        store.borrow_mut().remove(&account);
                        Ok(())
                    },
                });
                let store = store.into_inner();
                if fail_save || fail_remove {
                    assert!(result.is_err());
                    assert_eq!(store.get(&old).map(String::as_str), Some("old"));
                    assert!(!store.contains_key(&new));
                } else {
                    result.unwrap();
                    assert!(!store.contains_key(&old));
                    assert_eq!(store.get(&new).map(String::as_str), Some("new"));
                }
            }
        }
    }

    #[test]
    fn updating_password_at_same_endpoint_does_not_delete_the_credential() {
        let p = profile();
        let mut saves = 0;
        replace_with(ReplaceWithInput {
            previous: Some(&p),
            profile: &p,
            password: "new",
            save: |_, _| {
                saves += 1;
                Ok(())
            },
            forget: |_| panic!("The current credential must not be deleted"),
        })
        .unwrap();
        assert_eq!(saves, 1);
    }

    #[test]
    fn credentials_are_bound_to_authentication_identity_not_display_options() {
        let mut p = VncProfile {
            id: "fixture".into(),
            name: "name".into(),
            host: "one.example".into(),
            port: 5900,
            username: String::new(),
            password: String::new(),
            shared: true,
            view_only: false,
            last_connected: 0,
            options: terminator_core::vnc::VncOptions::default(),
            credential_saved: false,
        };
        let original = credential_account(&p).unwrap();
        p.password = "new secret".into();
        p.options.quality = terminator_core::vnc::Quality::Sharp;
        assert_eq!(credential_account(&p).unwrap(), original);
        p.host = "other.example".into();
        assert_ne!(credential_account(&p).unwrap(), original);
    }
}
