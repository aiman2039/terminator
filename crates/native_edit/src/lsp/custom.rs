//! User-defined language servers (Settings → Languages): file types, a
//! command and its arguments. They are stored in the same settings map
//! as the server path overrides, under keys that start with
//! [`CUSTOM_PREFIX`], so older daemons keep them unchanged. A user server
//! wins over a built-in one for the same language. Every user server
//! needs the project trust answer before it starts.
use super::servers::ServerFamily;
use crate::highlight::{Language, language_for_path};
use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError, RwLock};

/// Settings-map key prefix of a user-defined server.
pub const CUSTOM_PREFIX: &str = "custom:";

/// One user-defined server as Settings edits it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CustomServer {
    /// Unique name; the family id is `custom:<name>`.
    pub name: String,
    /// File extensions without the dot (`zig`, `nix`).
    pub extensions: Vec<String>,
    /// Executable: a bare name looked up on `PATH`, or a path.
    pub command: String,
    pub args: Vec<String>,
}

impl CustomServer {
    /// Settings-map key for this server.
    #[must_use]
    pub fn key(&self) -> String {
        format!("{CUSTOM_PREFIX}{}", self.name.trim())
    }

    /// Settings-map value: a small JSON object.
    #[must_use]
    pub fn encode(&self) -> String {
        serde_json::json!({
            "extensions": self.extensions,
            "command": self.command,
            "args": self.args,
        })
        .to_string()
    }

    /// Whether it can run: a name, a command, and one file type.
    #[must_use]
    pub fn usable(&self) -> bool {
        !self.name.trim().is_empty()
            && !self.command.trim().is_empty()
            && self.extensions.iter().any(|ext| !ext.trim().is_empty())
    }
}

/// User servers stored in a settings map, in key order. Malformed values
/// are skipped (the row reads back empty instead of failing the load).
#[must_use]
pub fn parse_custom_servers(map: &BTreeMap<String, String>) -> Vec<CustomServer> {
    map.iter()
        .filter_map(|(key, value)| {
            let name = key.strip_prefix(CUSTOM_PREFIX)?;
            let json: serde_json::Value = serde_json::from_str(value).ok()?;
            let strings = |field: &str| -> Vec<String> {
                json.get(field)
                    .and_then(serde_json::Value::as_array)
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|item| item.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default()
            };
            Some(CustomServer {
                name: name.to_owned(),
                extensions: strings("extensions"),
                command: json
                    .get("command")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                args: strings("args"),
            })
        })
        .collect()
}

/// Replace every user server in `map` with `servers`; path overrides
/// stay as they are.
pub fn store_custom_servers(map: &mut BTreeMap<String, String>, servers: &[CustomServer]) {
    map.retain(|key, _| !key.starts_with(CUSTOM_PREFIX));
    for server in servers
        .iter()
        .filter(|server| !server.name.trim().is_empty())
    {
        map.insert(server.key(), server.encode());
    }
}

struct Registered {
    family: &'static ServerFamily,
    languages: Vec<Language>,
}

static REGISTRY: RwLock<Vec<Registered>> = RwLock::new(Vec::new());
/// Families already leaked, so the same settings never leak twice.
static INTERNED: Mutex<Vec<&'static ServerFamily>> = Mutex::new(Vec::new());

fn leak_str(value: &str) -> &'static str {
    Box::leak(value.to_owned().into_boxed_str())
}

/// A `'static` family for `server`, reusing an equal one made before.
fn intern(server: &CustomServer) -> &'static ServerFamily {
    let mut interned = INTERNED.lock().unwrap_or_else(PoisonError::into_inner);
    let key = server.key();
    let command = server.command.trim();
    if let Some(found) = interned.iter().find(|family| {
        family.id == key
            && family.candidates == [command]
            && family.args.len() == server.args.len()
            && family.args.iter().zip(&server.args).all(|(a, b)| a == b)
    }) {
        return found;
    }
    let args: Vec<&'static str> = server.args.iter().map(|arg| leak_str(arg)).collect();
    let family: &'static ServerFamily = Box::leak(Box::new(ServerFamily {
        id: leak_str(&key),
        label: leak_str(server.name.trim()),
        candidates: Box::leak(vec![leak_str(command)].into_boxed_slice()),
        args: Box::leak(args.into_boxed_slice()),
        needs_trust: true,
        install: "",
    }));
    interned.push(family);
    family
}

/// Highlight languages a server's file types open as. Extensions with no
/// bundled grammar read as plain text and are left out: a server for
/// "plain text" would take every unknown file.
#[must_use]
pub fn custom_languages(server: &CustomServer) -> Vec<Language> {
    let mut languages = Vec::new();
    for ext in &server.extensions {
        let ext = ext.trim().trim_start_matches('.');
        if ext.is_empty() {
            continue;
        }
        let language = language_for_path(&format!("file.{ext}"));
        if language != Language::Plain && !languages.contains(&language) {
            languages.push(language);
        }
    }
    languages
}

/// Make `servers` the active user servers. Later entries lose to earlier
/// ones for the same language.
pub fn set_custom_servers(servers: &[CustomServer]) {
    let registered: Vec<Registered> = servers
        .iter()
        .filter(|server| server.usable())
        .map(|server| Registered {
            family: intern(server),
            languages: custom_languages(server),
        })
        .collect();
    *REGISTRY.write().unwrap_or_else(PoisonError::into_inner) = registered;
}

/// Active user server by family id.
#[must_use]
pub fn custom_family(id: &str) -> Option<&'static ServerFamily> {
    if !id.starts_with(CUSTOM_PREFIX) {
        return None;
    }
    REGISTRY
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .find(|entry| entry.family.id == id)
        .map(|entry| entry.family)
}

/// Active user server for a language.
#[must_use]
pub fn custom_family_for(language: Language) -> Option<&'static ServerFamily> {
    REGISTRY
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .find(|entry| entry.languages.contains(&language))
        .map(|entry| entry.family)
}

/// Every active user server, in Settings order.
#[must_use]
pub fn custom_families() -> Vec<&'static ServerFamily> {
    REGISTRY
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .map(|entry| entry.family)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zig() -> CustomServer {
        CustomServer {
            name: "zls".into(),
            extensions: vec!["zig".into(), String::new()],
            command: "zls".into(),
            args: vec!["--stdio".into()],
        }
    }

    #[test]
    fn user_servers_round_trip_beside_path_overrides() {
        let mut map = BTreeMap::new();
        map.insert("rust".to_owned(), "/opt/ra".to_owned());
        map.insert("custom:old".to_owned(), "{}".to_owned());
        store_custom_servers(&mut map, &[zig()]);
        assert_eq!(map.get("rust").map(String::as_str), Some("/opt/ra"));
        assert!(!map.contains_key("custom:old"));
        assert_eq!(parse_custom_servers(&map), vec![zig()]);
    }

    #[test]
    fn malformed_user_server_value_is_skipped() {
        let mut map = BTreeMap::new();
        map.insert("custom:bad".to_owned(), "not json".to_owned());
        assert!(parse_custom_servers(&map).is_empty());
    }

    #[test]
    fn plain_text_file_types_never_take_a_server() {
        let server = CustomServer {
            extensions: vec!["no-such-ext-xyz".into()],
            ..zig()
        };
        assert!(custom_languages(&server).is_empty());
    }

    #[test]
    fn same_settings_reuse_one_family() {
        assert!(std::ptr::eq(intern(&zig()), intern(&zig())));
        let other = CustomServer {
            args: vec![],
            ..zig()
        };
        assert!(!std::ptr::eq(intern(&zig()), intern(&other)));
        assert!(intern(&zig()).needs_trust);
    }
}
