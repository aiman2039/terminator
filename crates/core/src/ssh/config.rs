//! Host aliases from the user's OpenSSH configuration. Parsing only lists
//! concrete `Host` names; `ssh -G` then reports what OpenSSH would use.
use super::{private_environment_keys, validate_host};
use std::{
    collections::HashSet,
    io::Read,
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
    time::Duration,
};

/// OpenSSH refuses deeper `Include` nesting too.
const INCLUDE_DEPTH: usize = 16;
const MAX_HOSTS: usize = 256;
const RESOLVE_CONCURRENCY: usize = 8;
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(3);
const RESOLVE_OUTPUT_LIMIT: u64 = 256 * 1024;

/// A concrete `Host` alias and the endpoint `ssh -G` resolved for it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConfigHost {
    pub alias: String,
    pub user: String,
    pub hostname: String,
    pub port: Option<u16>,
    pub proxyjump: Option<String>,
}

/// Files the parser may read. Tests substitute an in-memory fixture.
pub trait ConfigSource {
    fn read(&self, path: &Path) -> Option<String>;
    /// Entry names in `dir`, in any order.
    fn list(&self, dir: &Path) -> Vec<String>;
}

struct FileSystem;
impl ConfigSource for FileSystem {
    fn read(&self, path: &Path) -> Option<String> {
        if !path.is_file() {
            return None;
        }
        std::fs::read_to_string(path).ok()
    }
    fn list(&self, dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .map(|entries| {
                entries
                    .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Concrete aliases from `~/.ssh/config` and its includes, resolved with
/// `ssh -G`. Blocking: call it off the GUI thread. Empty when there is no
/// configuration.
#[must_use]
pub fn config_hosts() -> Vec<ConfigHost> {
    let Some(base) = directories::BaseDirs::new() else {
        return Vec::new();
    };
    let home = base.home_dir();
    let ssh_dir = home.join(".ssh");
    let aliases = parse_config_aliases(&FileSystem, home, &ssh_dir.join("config"));
    let mut hosts = Vec::with_capacity(aliases.len());
    for chunk in aliases.chunks(RESOLVE_CONCURRENCY) {
        std::thread::scope(|scope| {
            let handles: Vec<_> = chunk
                .iter()
                .map(|alias| scope.spawn(move || resolve(alias)))
                .collect();
            for (alias, handle) in chunk.iter().zip(handles) {
                hosts.push(handle.join().unwrap_or_else(|_| unresolved(alias)));
            }
        });
    }
    hosts
}

fn unresolved(alias: &str) -> ConfigHost {
    ConfigHost {
        alias: alias.to_owned(),
        hostname: alias.to_owned(),
        ..ConfigHost::default()
    }
}

/// Runs `ssh -G <alias>`; an alias it cannot resolve in time is still listed.
fn resolve(alias: &str) -> ConfigHost {
    let mut command = Command::new("ssh");
    command
        .args(["-G", "--", alias])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    for key in private_environment_keys() {
        command.env_remove(key);
    }
    let Ok(mut child) = command.spawn() else {
        return unresolved(alias);
    };
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return unresolved(alias);
    };
    let (sender, receiver) = mpsc::channel();
    // A `Match exec` grandchild may keep stdout open; never join this reader.
    std::thread::spawn(move || {
        let mut text = String::new();
        let ok = stdout
            .take(RESOLVE_OUTPUT_LIMIT)
            .read_to_string(&mut text)
            .is_ok();
        let _ = sender.send(ok.then_some(text));
    });
    match receiver.recv_timeout(RESOLVE_TIMEOUT) {
        Ok(Some(output)) if child.wait().is_ok_and(|s| s.success()) => {
            parse_config_host(alias, &output)
        }
        _ => {
            let _ = child.kill();
            let _ = child.wait();
            unresolved(alias)
        }
    }
}

/// Parses `ssh -G <alias>` output.
#[must_use]
pub fn parse_config_host(alias: &str, output: &str) -> ConfigHost {
    let mut host = unresolved(alias);
    for line in output.lines() {
        let Some((key, value)) = line.split_once(' ') else {
            continue;
        };
        let value = value.trim();
        match key {
            "user" => value.clone_into(&mut host.user),
            "hostname" if !value.is_empty() => value.clone_into(&mut host.hostname),
            "port" => host.port = value.parse::<u16>().ok().filter(|p| *p > 0),
            "proxyjump" if !value.is_empty() && value != "none" => {
                host.proxyjump = Some(value.to_owned());
            }
            _ => {}
        }
    }
    host
}

/// Concrete `Host` aliases in `config` and the files it includes, in order
/// and without duplicates. Wildcard and negated patterns are skipped, as are
/// names `validate_host` rejects.
#[must_use]
pub fn parse_config_aliases(source: &dyn ConfigSource, home: &Path, config: &Path) -> Vec<String> {
    let ssh_dir = config
        .parent()
        .map_or_else(|| home.join(".ssh"), Path::to_path_buf);
    let mut parser = Parser {
        source,
        home,
        ssh_dir: &ssh_dir,
        aliases: Vec::new(),
        seen: HashSet::new(),
        visited: HashSet::new(),
    };
    parser.file(config, 0);
    parser.aliases
}

struct Parser<'a> {
    source: &'a dyn ConfigSource,
    home: &'a Path,
    ssh_dir: &'a Path,
    aliases: Vec<String>,
    seen: HashSet<String>,
    visited: HashSet<PathBuf>,
}
impl Parser<'_> {
    fn file(&mut self, path: &Path, depth: usize) {
        if depth > INCLUDE_DEPTH || !self.visited.insert(path.to_path_buf()) {
            return;
        }
        let Some(text) = self.source.read(path) else {
            return;
        };
        for line in text.lines() {
            let mut words = tokens(line).into_iter();
            let Some(keyword) = words.next() else {
                continue;
            };
            if keyword.eq_ignore_ascii_case("host") {
                for pattern in words {
                    self.alias(pattern);
                }
            } else if keyword.eq_ignore_ascii_case("include") {
                for pattern in words {
                    for included in self.expand(&pattern) {
                        self.file(&included, depth.saturating_add(1));
                    }
                }
            }
        }
    }
    fn alias(&mut self, pattern: String) {
        if self.aliases.len() >= MAX_HOSTS
            || pattern.starts_with('!')
            || pattern.contains(['*', '?'])
            || validate_host(&pattern).is_err()
            || !self.seen.insert(pattern.clone())
        {
            return;
        }
        self.aliases.push(pattern);
    }
    /// `~` is the home directory and relative paths are inside `~/.ssh`, as
    /// for the user configuration. `*` and `?` match within one component.
    fn expand(&self, pattern: &str) -> Vec<PathBuf> {
        let path = if let Some(rest) = pattern.strip_prefix("~/") {
            self.home.join(rest)
        } else if Path::new(pattern).is_absolute() {
            PathBuf::from(pattern)
        } else {
            self.ssh_dir.join(pattern)
        };
        let mut matches = vec![PathBuf::new()];
        for component in path.components() {
            let name = match component {
                Component::Normal(name) => name.to_string_lossy(),
                other => {
                    for candidate in &mut matches {
                        candidate.push(other.as_os_str());
                    }
                    continue;
                }
            };
            if !name.contains(['*', '?']) {
                for candidate in &mut matches {
                    candidate.push(name.as_ref());
                }
                continue;
            }
            let mut next = Vec::new();
            for candidate in &matches {
                let mut names: Vec<String> = self
                    .source
                    .list(candidate)
                    .into_iter()
                    .filter(|entry| {
                        (name.starts_with('.') || !entry.starts_with('.'))
                            && wildcard(name.as_bytes(), entry.as_bytes())
                    })
                    .collect();
                names.sort();
                next.extend(names.into_iter().map(|entry| candidate.join(entry)));
            }
            matches = next;
        }
        matches
    }
}

/// `*` matches any run of bytes and `?` any one byte.
fn wildcard(pattern: &[u8], text: &[u8]) -> bool {
    match pattern.split_first() {
        None => text.is_empty(),
        Some((b'*', rest)) => {
            (0..=text.len()).any(|skip| text.get(skip..).is_some_and(|tail| wildcard(rest, tail)))
        }
        Some((first, rest)) => text
            .split_first()
            .is_some_and(|(byte, tail)| (*first == b'?' || first == byte) && wildcard(rest, tail)),
    }
}

/// Splits one configuration line like OpenSSH: the keyword ends at
/// whitespace or `=`, arguments are whitespace-separated and may be quoted.
fn tokens(line: &str) -> Vec<String> {
    let line = line.trim_start();
    if line.starts_with('#') {
        return Vec::new();
    }
    let keyword_end = line
        .find(|c: char| c.is_whitespace() || c == '=')
        .unwrap_or(line.len());
    let (keyword, rest) = line.split_at(keyword_end);
    if keyword.is_empty() {
        return Vec::new();
    }
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('=').unwrap_or(rest);
    let mut words = vec![keyword.to_owned()];
    let mut current = String::new();
    let mut quoted = false;
    let mut started = false;
    for c in rest.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            '#' if !quoted && !started => break,
            c if c.is_whitespace() && !quoted => {
                if started {
                    words.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            c => {
                current.push(c);
                started = true;
            }
        }
    }
    if started {
        words.push(current);
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct Fixture(HashMap<PathBuf, String>);
    impl ConfigSource for Fixture {
        fn read(&self, path: &Path) -> Option<String> {
            self.0.get(path).cloned()
        }
        fn list(&self, dir: &Path) -> Vec<String> {
            self.0
                .keys()
                .filter(|path| path.parent() == Some(dir))
                .filter_map(|path| Some(path.file_name()?.to_str()?.to_owned()))
                .collect()
        }
    }

    #[test]
    fn config_aliases_follow_includes_and_skip_patterns() {
        let fixture = Fixture(
            [
                (
                    "/home/u/.ssh/config",
                    "# comment\n\
                     Include config.d/*.conf\n\
                     Host build bastion\n\
                     \x20 HostName build.example.com\n\
                     \x20 ProxyJump bastion\n\
                     Host *.internal web-? !blocked staging\n\
                     Host=\"quoted\" build\n\
                     Match host foo\n\
                     Include ~/extra/hosts\n\
                     Include config\n\
                     host -oProxyCommand=x\n",
                ),
                ("/home/u/.ssh/config.d/b.conf", "Host second\n  Port 2222\n"),
                ("/home/u/.ssh/config.d/a.conf", "HOST first # trailing\n"),
                ("/home/u/.ssh/config.d/.hidden.conf", "Host hidden\n"),
                ("/home/u/.ssh/config.d/skip.txt", "Host skipped\n"),
                ("/home/u/extra/hosts", "Host extra build\n"),
            ]
            .into_iter()
            .map(|(path, text)| (PathBuf::from(path), text.to_owned()))
            .collect(),
        );
        assert_eq!(
            parse_config_aliases(
                &fixture,
                Path::new("/home/u"),
                Path::new("/home/u/.ssh/config")
            ),
            [
                "first", "second", "build", "bastion", "staging", "quoted", "extra"
            ]
        );
        assert!(
            parse_config_aliases(&fixture, Path::new("/none"), Path::new("/none/.ssh/config"))
                .is_empty()
        );
    }

    #[test]
    fn resolved_config_host_reports_user_endpoint_and_jump() {
        let host = parse_config_host(
            "build",
            "host build\nuser developer\nhostname build.example.com\nport 2222\nproxyjump bastion\n",
        );
        assert_eq!(
            host,
            ConfigHost {
                alias: "build".into(),
                user: "developer".into(),
                hostname: "build.example.com".into(),
                port: Some(2222),
                proxyjump: Some("bastion".into()),
            }
        );
        let plain = parse_config_host("plain", "user me\nport 22\n");
        assert_eq!(plain.hostname, "plain");
        assert_eq!(plain.proxyjump, None);
    }

    #[test]
    fn include_wildcards_match_one_component() {
        assert!(wildcard(b"*.conf", b"a.conf"));
        assert!(wildcard(b"web-?", b"web-1"));
        assert!(!wildcard(b"web-?", b"web-12"));
        assert!(!wildcard(b"*.conf", b"a.txt"));
        assert_eq!(
            tokens("  IdentityFile = \"/a b/c\" # x"),
            ["IdentityFile", "/a b/c"]
        );
    }
}
