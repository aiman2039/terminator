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
    Ok(format!(
        "{}:{}",
        profile.id,
        hex::encode(Sha256::digest(identity))
    ))
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

/// One background migration at a time; requests that see legacy
/// passwords call [`finish_activation`] again without piling up threads.
static MIGRATING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

struct MigratingGuard;
impl MigratingGuard {
    fn claim() -> Option<Self> {
        (!MIGRATING.swap(true, std::sync::atomic::Ordering::AcqRel)).then_some(Self)
    }
}
impl Drop for MigratingGuard {
    fn drop(&mut self) {
        MIGRATING.store(false, std::sync::atomic::Ordering::Release);
    }
}

/// A prepared generation cannot write the shared catalog. Finish verified
/// credential migration only after activation, under the workspace lock.
/// Runs on its own thread so keychain prompts never block short IPC requests.
pub(crate) fn finish_activation(weak: std::sync::Weak<crate::server::Shared>) {
    let Some(guard) = MigratingGuard::claim() else {
        return;
    };
    std::thread::spawn(move || {
        let _guard = guard;
        let started = std::time::Instant::now();
        while started.elapsed() < std::time::Duration::from_secs(30) {
            let Some(shared) = weak.upgrade() else {
                return;
            };
            let Some(paths) = shared.catalog_paths.as_ref() else {
                return;
            };
            let result = migrate_pass(&shared, paths);
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

/// One migration attempt. The keychain writes run first, without the
/// coordination lock: a keychain prompt may wait on the user, and every
/// coordinated request would fail with Busy meanwhile. The lock is taken
/// only to publish profiles whose stored identity and password did not
/// change since. True when nothing legacy remains.
fn migrate_pass(shared: &crate::server::Shared, paths: &Paths) -> Result<bool> {
    let generation = crate::server::relock(&shared.state).generation.clone();
    let legacy: Vec<VncProfile> = {
        let catalog = terminator_core::generations::Catalog::open(paths)?;
        if catalog.active()?.as_deref() != Some(&generation) {
            return Ok(false);
        }
        catalog
            .workspace()?
            .vnc_profiles
            .into_iter()
            .filter(|p| !p.password.is_empty())
            .collect()
    };
    if legacy.is_empty() {
        return Ok(true);
    }
    let mut stored = legacy.clone();
    ensure!(
        migrate(paths, &mut stored) == 0,
        "Unlock the OS credential store to finish legacy VNC migration"
    );
    let _lock = terminator_core::generations::coordinate_timeout(
        paths,
        std::time::Duration::from_millis(100),
    )?;
    let catalog = terminator_core::generations::Catalog::open(paths)?;
    if catalog.active()?.as_deref() != Some(&generation) {
        return Ok(false);
    }
    let mut updated = crate::server::relock(&shared.state).clone();
    catalog.workspace()?.apply(&mut updated);
    let remaining = mark_migrated(&mut updated.vnc_profiles, &legacy)?;
    catalog.save_workspace(&updated)?;
    {
        let mut state = crate::server::relock(&shared.state);
        state.vnc_profiles = updated.vnc_profiles;
        state.revision = state.revision.saturating_add(1);
    }
    shared.persist()?;
    Ok(remaining == 0)
}

/// Clears the plaintext password of each profile that still matches a
/// `migrated` one (same credential identity and password); returns how
/// many legacy passwords remain for the next pass.
fn mark_migrated(profiles: &mut [VncProfile], migrated: &[VncProfile]) -> Result<usize> {
    let mut remaining = 0usize;
    for p in profiles.iter_mut().filter(|p| !p.password.is_empty()) {
        let account = credential_account(p)?;
        let mut done = false;
        for m in migrated {
            if m.password == p.password && credential_account(m)? == account {
                done = true;
                break;
            }
        }
        if done {
            zeroize::Zeroize::zeroize(&mut p.password);
            p.credential_saved = true;
        } else {
            remaining = remaining.saturating_add(1);
        }
    }
    Ok(remaining)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_clears_only_passwords_that_were_stored_unchanged() {
        let mut stored = profile();
        stored.credential_saved = false;
        stored.password = "secret".into();
        let mut changed = stored.clone();
        changed.id = "other".into();
        changed.password = "typed later".into();
        let mut moved = stored.clone();
        moved.host = "two.example".into();
        let mut profiles = vec![stored.clone(), changed, moved];
        let remaining = mark_migrated(&mut profiles, std::slice::from_ref(&stored)).unwrap();
        assert_eq!(remaining, 2);
        assert!(profiles[0].password.is_empty() && profiles[0].credential_saved);
        assert_eq!(profiles[1].password, "typed later");
        assert_eq!(profiles[2].password, "secret");
    }
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
            labels: Vec::new(),
        }
    }

    #[test]
    fn background_migration_runs_once_at_a_time() {
        let first = MigratingGuard::claim();
        assert!(first.is_some());
        assert!(MigratingGuard::claim().is_none());
        drop(first);
        assert!(MigratingGuard::claim().is_some());
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
            labels: Vec::new(),
        };
        let original = credential_account(&p).unwrap();
        p.password = "new secret".into();
        p.options.quality = terminator_core::vnc::Quality::Sharp;
        assert_eq!(credential_account(&p).unwrap(), original);
        p.host = "other.example".into();
        assert_ne!(credential_account(&p).unwrap(), original);
    }
}
