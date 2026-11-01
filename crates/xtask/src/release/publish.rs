use super::command::{Runner, cmd, git};
use super::{Options, source_tree, version};
use anyhow::{Context, Result, ensure};
use serde_json::json;
use std::{fs, path::Path};

pub(super) struct ExecuteInput<'a, R, V>
where
    R: Runner,
    V: FnOnce(&Path, &Path) -> Result<()>,
{
    pub(super) root: &'a Path,
    pub(super) options: &'a Options,
    pub(super) runner: &'a R,
    pub(super) validate: V,
}

pub(super) fn execute<R, V>(input: ExecuteInput<'_, R, V>) -> Result<()>
where
    R: Runner,
    V: FnOnce(&Path, &Path) -> Result<()>,
{
    let ExecuteInput {
        root,
        options,
        runner,
        validate,
    } = input;
    let (head, branch) = check_branch(root, options, runner)?;
    let parent = root.join(".artifacts/releases");
    fs::create_dir_all(&parent)?;
    let artifacts = tempfile::Builder::new()
        .prefix("run-")
        .tempdir_in(parent)?
        .keep();
    println!("Release logs: {}", artifacts.display());
    let temporary = tempfile::Builder::new()
        .prefix("terminator-release-")
        .tempdir()?;
    let original = source_tree(root, temporary.path())?;
    let (next, manifest, lock) = version::prepare(root)?;
    let tag = format!("v{next}");
    if !options.check_only {
        refuse_existing_tag(root, &tag)?;
    }
    // Check-only uses the current version without touching source files.
    // Restore our version edit on failure, unless another writer changed it.
    let mut version_edit = VersionEdit::new(root, manifest, lock)?;
    if !options.check_only {
        version_edit.apply()?;
    }
    let validated = source_tree(root, temporary.path())?;
    fs::write(
        artifacts.join("source.json"),
        serde_json::to_vec_pretty(&json!({
            "base_commit":head, "original_tree":original, "validated_tree":validated, "tag":tag,
        }))?,
    )?;
    println!(
        "Checking the current checkout: {}. Keep source files unchanged until checks finish.",
        root.display()
    );
    validate(root, &artifacts)?;
    ensure!(
        git(root, &["rev-parse", "HEAD"])? == head
            && git(root, &["symbolic-ref", "--short", "HEAD"])? == branch
            && source_tree(root, temporary.path())? == validated,
        "Source changed during validation. Run the checks again."
    );
    if options.check_only {
        println!(
            "Selected checks passed for the current version. Version, index, commits and remote are unchanged."
        );
        return Ok(());
    }
    // Publication owns the version edit from here, even if push/dispatch fails.
    version_edit.applied = false;
    publish(root, runner, (&tag, &validated, &artifacts))
}

struct VersionEdit<'a> {
    root: &'a Path,
    files: [(&'static str, Vec<u8>, String); 2],
    applied: bool,
}

impl<'a> VersionEdit<'a> {
    fn new(root: &'a Path, manifest: String, lock: String) -> Result<Self> {
        Ok(Self {
            root,
            files: [
                ("Cargo.toml", fs::read(root.join("Cargo.toml"))?, manifest),
                ("Cargo.lock", fs::read(root.join("Cargo.lock"))?, lock),
            ],
            applied: false,
        })
    }

    fn apply(&mut self) -> Result<()> {
        self.applied = true;
        for (name, _, changed) in &self.files {
            fs::write(self.root.join(name), changed)?;
        }
        Ok(())
    }
}

impl Drop for VersionEdit<'_> {
    fn drop(&mut self) {
        if self.applied {
            for (name, original, changed) in &self.files {
                let path = self.root.join(name);
                if fs::read(&path).is_ok_and(|bytes| bytes == changed.as_bytes())
                    && let Err(error) = fs::write(&path, original)
                {
                    eprintln!("Could not restore {}: {error}", path.display());
                }
            }
        }
    }
}

/// Releases run from an up-to-date master; returns HEAD and the branch name.
fn check_branch(root: &Path, options: &Options, runner: &impl Runner) -> Result<(String, String)> {
    let head = git(root, &["rev-parse", "HEAD"])?;
    let branch = git(root, &["symbolic-ref", "--short", "HEAD"])?;
    if !options.check_only {
        ensure!(
            branch == "master",
            "Release must run from master. Use --check-only on other branches."
        );
        runner.run(&mut cmd(root, "gh", &["auth", "status"]))?;
        runner.run(&mut cmd(
            root,
            "git",
            &["fetch", "--no-tags", "origin", "master"],
        ))?;
        let remote = git(root, &["rev-parse", "refs/remotes/origin/master"])?;
        runner.run(&mut cmd(
            root,
            "git",
            &["merge-base", "--is-ancestor", &remote, &head],
        ))?;
    }
    Ok((head, branch))
}

fn refuse_existing_tag(root: &Path, tag: &str) -> Result<()> {
    ensure!(
        git(root, &["tag", "--list", tag])?.is_empty()
            && git(
                root,
                &["ls-remote", "--tags", "origin", &format!("refs/tags/{tag}")]
            )?
            .is_empty(),
        "Tag {tag} already exists. No files changed."
    );
    Ok(())
}

/// Commit the validated tree, tag and push it, then dispatch the release workflow.
fn publish(
    root: &Path,
    runner: &impl Runner,
    (tag, validated, artifacts): (&str, &str, &Path),
) -> Result<()> {
    git(root, &["add", "--all"])?;
    ensure!(
        git(root, &["write-tree"])? == validated,
        "Staged source differs from the validated source. Nothing pushed."
    );
    runner.run(
        cmd(root, "git", &["commit", "-m", &format!("Release {tag}")])
            .env("TERMINATOR_RELEASE_VALIDATED_TREE", validated),
    )?;
    ensure!(
        git(root, &["rev-parse", "HEAD^{tree}"])? == validated,
        "Commit hooks changed the validated source. Nothing pushed."
    );
    let revision = git(root, &["rev-parse", "HEAD"])?;
    git(root, &["tag", tag, &revision])?;
    runner.run(&mut cmd(
        root,
        "git",
        &[
            "push",
            "--atomic",
            "origin",
            &format!("{revision}:refs/heads/master"),
            &format!("refs/tags/{tag}"),
        ],
    ))?;
    runner.run(&mut cmd(
        root,
        "gh",
        &[
            "workflow",
            "run",
            "release.yaml",
            "--ref",
            "master",
            "-f",
            &format!("tag={tag}"),
        ],
    ))
    .with_context(|| format!("{tag} was pushed, but workflow dispatch failed. Retry only: gh workflow run release.yaml --ref master -f tag={tag} (if master has moved, use --ref {tag} without the tag input)"))?;
    println!(
        "Started Release for {tag} ({revision}). Logs: {}",
        artifacts.display()
    );
    Ok(())
}
