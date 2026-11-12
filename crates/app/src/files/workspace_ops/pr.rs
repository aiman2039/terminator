//! Create PR: `gh pr create` or `glab mr create`, chosen by the `origin`
//! host. It runs only on an explicit click and never prompts.
use super::*;

/// Which CLI creates the review for a remote URL: GitLab hosts use `glab`,
/// every other host uses `gh`.
pub(crate) fn review_cli(remote: &str) -> &'static str {
    let host = crate::git_log::remote_host(remote).unwrap_or_default();
    if host.contains("gitlab") {
        "glab"
    } else {
        "gh"
    }
}

/// The CLI arguments that open a review from the pushed branch with a
/// title and body filled from its commits.
pub(super) fn review_args(cli: &str) -> &'static [&'static str] {
    if cli == "glab" {
        &["mr", "create", "--fill", "--yes"]
    } else {
        &["pr", "create", "--fill"]
    }
}

pub(super) fn create_pr(root: &Path) -> Result<Report> {
    let remote = git_text(root, &["remote", "get-url", "origin"])
        .context("Create PR needs an origin remote")?;
    let cli = review_cli(&remote);
    let mut command = std::process::Command::new(cli);
    command
        .current_dir(root)
        .args(review_args(cli))
        .env("GH_PROMPT_DISABLED", "1")
        .env("GLAB_NO_PROMPT", "1");
    let output = run_command(command, git_options())
        .with_context(|| format!("{cli} could not create the review"))?;
    let text = String::from_utf8_lossy(&output.stdout);
    let url = text.lines().rev().find(|line| line.starts_with("http"));
    Ok(Report::Message(match url {
        Some(url) => format!("Created {url}"),
        None => "Review created".to_owned(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gitlab_remotes_use_glab_and_others_use_gh() {
        assert_eq!(review_cli("git@gitlab.example.com:g/r.git"), "glab");
        assert_eq!(review_cli("https://gitlab.com/g/r"), "glab");
        assert_eq!(review_cli("git@github.com:o/r.git"), "gh");
        assert_eq!(review_cli("/srv/git/repo.git"), "gh");
    }

    #[test]
    fn review_creation_never_prompts_and_fills_from_commits() {
        assert_eq!(review_args("gh"), ["pr", "create", "--fill"]);
        assert!(review_args("glab").contains(&"--yes"));
    }
}
