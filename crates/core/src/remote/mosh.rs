//! Mosh terminals (`ssh-mosh-v1`): the daemon builds the remote
//! `mosh-server` command; `terminator-hook mosh` runs it over SSH and becomes
//! the local `mosh-client`.

const LOCALE_KEYS: [&str; 3] = ["LC_ALL", "LC_CTYPE", "LANG"];

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

/// Remote command that starts `mosh-server` running `script` under `sh`.
/// `mosh-server` is also looked up in common install directories a
/// non-login PATH misses; without a local UTF-8 locale it gets `C.UTF-8`.
#[must_use]
pub fn server_command(script: &str) -> String {
    let mut locale = utf8_locale_vars();
    if locale.is_empty() {
        locale.push("LANG=C.UTF-8".to_owned());
    }
    let mut server = String::from(
        "PATH=\"$PATH:/usr/local/bin:/opt/homebrew/bin:/opt/local/bin:/snap/bin\"; exec mosh-server new -s -c 256",
    );
    for var in &locale {
        server.push_str(" -l ");
        server.push_str(&crate::quote(var));
    }
    server.push_str(" -- sh -c ");
    server.push_str(&crate::quote(script));
    format!("exec sh -c {}", crate::quote(&server))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_command_quotes_script_and_sends_a_utf8_locale() {
        let command = server_command("cd -- '/a b'; exec \"$s\"");
        assert!(command.starts_with("exec sh -c '"), "{command}");
        assert!(command.contains("mosh-server new -s -c 256"), "{command}");
        assert!(command.contains(" -l "), "{command}");
        assert!(!command.contains('\n'), "{command}");
    }

    #[test]
    fn utf8_locales_are_recognized() {
        assert!(is_utf8_locale("en_US.UTF-8"));
        assert!(is_utf8_locale("C.utf8"));
        assert!(!is_utf8_locale("C"));
    }
}
