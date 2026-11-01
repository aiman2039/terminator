//! Web links for commits: map the `origin` remote URL (ssh, scp-like or
//! https) to the commit page on its host.
use super::CommandOptions;
use std::path::Path;

/// The commit page prefix for the repository at `root`: append a hash to
/// get the page. `None` without an `origin` remote or on an unknown host.
pub fn commit_web_base(root: &Path) -> Option<String> {
    let options = CommandOptions {
        timeout: std::time::Duration::from_secs(5),
        ..Default::default()
    };
    let bytes =
        terminator_core::git::run_blocking(root, &["remote", "get-url", "origin"], options).ok()?;
    commit_url_base(String::from_utf8_lossy(&bytes).trim())
}

/// The commit page prefix for a remote URL on a known host.
pub fn commit_url_base(remote: &str) -> Option<String> {
    let url = git_url_parse::GitUrl::parse(remote).ok()?;
    let host = url.host()?.to_ascii_lowercase();
    let path = url.path().trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    if path.is_empty() {
        return None;
    }
    let commit = if host.contains("gitlab") {
        "-/commit"
    } else if host == "bitbucket.org" {
        "commits"
    } else if host.contains("github")
        || host.contains("gitea")
        || host.contains("forgejo")
        || host == "codeberg.org"
    {
        "commit"
    } else {
        return None;
    };
    Some(format!("https://{host}/{path}/{commit}/"))
}

/// The host name shown in "Open commit on …".
pub fn commit_host(base: &str) -> &str {
    base.strip_prefix("https://")
        .and_then(|rest| rest.split('/').next())
        .unwrap_or(base)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_urls_map_to_commit_pages() {
        let cases = [
            (
                "git@github.com:owner/repo.git",
                Some("https://github.com/owner/repo/commit/"),
            ),
            (
                "https://github.com/owner/repo",
                Some("https://github.com/owner/repo/commit/"),
            ),
            (
                "ssh://git@gitlab.example.com:2222/group/sub/repo.git",
                Some("https://gitlab.example.com/group/sub/repo/-/commit/"),
            ),
            (
                "https://user@bitbucket.org/team/repo.git",
                Some("https://bitbucket.org/team/repo/commits/"),
            ),
            (
                "git@codeberg.org:me/tool.git",
                Some("https://codeberg.org/me/tool/commit/"),
            ),
            ("git@example.com:me/tool.git", None),
            ("/srv/git/repo.git", None),
        ];
        for (remote, want) in cases {
            assert_eq!(commit_url_base(remote).as_deref(), want, "{remote}");
        }
        assert_eq!(
            commit_host("https://github.com/owner/repo/commit/"),
            "github.com"
        );
    }
}
