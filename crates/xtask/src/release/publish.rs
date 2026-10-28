use super::command::{Runner, cmd, git};
use super::{
    ExportSourceInput, Options, export_paths, export_source, reusable_export, source_tree,
};
use anyhow::{Context, Result, ensure};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
};

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
    let (source, marker) = export_paths(root);
    let (tag, validated) = select_source(root, temporary.path(), (&source, &marker, &original))?;
    if !options.check_only {
        refuse_existing_tag(root, &tag)?;
    }
    fs::write(
        artifacts.join("source.json"),
        serde_json::to_vec_pretty(&json!({
            "base_commit":head, "original_tree":original, "validated_tree":validated, "tag":tag,
        }))?,
    )?;
    validate(&source, &artifacts)?;
    ensure!(
        git(&source, &["status", "--porcelain"])?.is_empty()
            && git(&source, &["rev-parse", "HEAD^{tree}"])? == validated,
        "Checks changed source files. Review the logs; no release was made."
    );
    ensure!(
        git(root, &["rev-parse", "HEAD"])? == head
            && git(root, &["symbolic-ref", "--short", "HEAD"])? == branch
            && source_tree(root, temporary.path())? == original,
        "Source changed during validation. Run the checks again."
    );
    if options.check_only {
        println!(
            "Selected checks passed for {tag}. Version, index, commits and remote are unchanged."
        );
        return Ok(());
    }
    publish(root, runner, (&source, &tag, &validated, &artifacts))
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

/// Reuse a validated export of this tree or export a fresh one; returns the tag
/// and validated tree.
fn select_source(
    root: &Path,
    temporary: &Path,
    (source, marker, original): (&PathBuf, &PathBuf, &str),
) -> Result<(String, String)> {
    if let Some((tag, validated)) = reusable_export(source, marker, original) {
        println!(
            "Reusing the validated source for {tag}: {}",
            source.display()
        );
        return Ok((tag, validated));
    }
    export_source(ExportSourceInput {
        root,
        temporary,
        source,
        marker,
        original,
    })
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
    (source, tag, validated, artifacts): (&Path, &str, &str, &Path),
) -> Result<()> {
    for name in ["Cargo.toml", "Cargo.lock"] {
        fs::copy(source.join(name), root.join(name))?;
    }
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
