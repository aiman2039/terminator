//! Mosh terminals (`ssh-mosh-v1`): the daemon builds the remote
//! `mosh-server` command; `terminator-hook mosh` runs it over SSH and becomes
//! the local `mosh-client`.

const LOCALE_KEYS: [&str; 3] = ["LC_ALL", "LC_CTYPE", "LANG"];
/// Common install directories a non-login PATH misses.
const PATH_PREFIX: &str =
    "PATH=\"$PATH:/usr/local/bin:/opt/homebrew/bin:/opt/local/bin:/snap/bin\"; ";

/// This process's UTF-8 locale settings (`KEY=value`), which `mosh-server
/// -l` applies on the host, as the `mosh` script sends them.
#[must_use]
pub fn utf8_locale_vars() -> Vec<String> {
    LOCALE_KEYS
        .into_iter()
        .filter_map(|key| {
            let value = std::env::var(key).ok()?;
            is_utf8_locale(&value).then(|| format!("{key}={value}"))
        })
        .collect()
}

fn is_utf8_locale(value: &str) -> bool {
    let value = value.to_ascii_lowercase();
    value.contains("utf-8") || value.contains("utf8")
}

/// The `mosh-server` command line a host uses unless it saves its own:
/// this computer's UTF-8 locale, else `C.UTF-8`. Read once: the locale does
/// not change while the process runs.
#[must_use]
pub fn default_server_command() -> &'static str {
    static DEFAULT: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
        let mut locale = utf8_locale_vars();
        if locale.is_empty() {
            locale.push("LANG=C.UTF-8".to_owned());
        }
        let mut server = String::from("mosh-server new -s -c 256");
        for var in &locale {
            server.push_str(" -l ");
            server.push_str(&crate::quote(var));
        }
        server
    });
    &DEFAULT
}

/// Remote command that runs `server` (else [`default_server_command`]) with
/// `-- sh -c script` added, so the session starts in its folder.
#[must_use]
pub fn server_command(server: Option<&str>, script: &str) -> String {
    let server = server.unwrap_or(default_server_command());
    let line = format!("{PATH_PREFIX}{server} -- sh -c {}", crate::quote(script));
    format!("exec sh -c {}", crate::quote(&line))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_command_quotes_script_and_sends_a_utf8_locale() {
        let command = server_command(None, "cd -- '/a b'; exec \"$s\"");
        assert!(command.starts_with("exec sh -c '"), "{command}");
        assert!(command.contains("mosh-server new -s -c 256"), "{command}");
        assert!(command.contains(" -l "), "{command}");
        assert!(!command.contains('\n'), "{command}");
    }

    #[test]
    fn saved_server_command_replaces_the_default_and_keeps_the_script() {
        let command = server_command(Some("env A=1 ~/bin/mosh-server new -p 60001"), "cd -- /w");
        assert!(
            command.contains("env A=1 ~/bin/mosh-server new -p 60001 -- sh -c"),
            "{command}"
        );
        assert!(!command.contains("-c 256"), "{command}");
        assert!(command.contains("/w"), "{command}");
    }

    #[test]
    fn utf8_locales_are_recognized() {
        assert!(is_utf8_locale("en_US.UTF-8"));
        assert!(is_utf8_locale("C.utf8"));
        assert!(!is_utf8_locale("C"));
    }
}
