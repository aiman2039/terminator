#!/usr/bin/env python3
"""Validate a versioned source copy before changing the user's checkout."""

import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
import tomllib


def run(args, cwd, env=None):
    return subprocess.run(args, cwd=cwd, env=env, check=True)


def git(root, *args, env=None):
    return subprocess.check_output(
        ["git", *args], cwd=root, env=env, text=True
    ).strip()


def source_tree(root, directory):
    """Include staged, unstaged, deleted and untracked files; preserve the real index."""
    index = directory / "index"
    index.unlink(missing_ok=True)
    env = dict(os.environ, GIT_INDEX_FILE=str(index))
    git(root, "read-tree", "HEAD", env=env)
    git(root, "add", "--all", env=env)
    return git(root, "write-tree", env=env)


def require(program):
    if not shutil.which(program):
        raise RuntimeError(f"Missing required tool: {program}")


def validate(source, artifacts, args):
    results = {
        "host": {"status": "pending"},
        "linux": {"status": "skipped explicitly" if args.skip_linux else "pending"},
        "windows": {"status": "skipped explicitly; host cross-check only" if args.skip_windows else "pending"},
    }
    (artifacts / "results.json").write_text(json.dumps(results, indent=2) + "\n")
    env = dict(os.environ, CARGO_HUSKY_DONT_INSTALL_HOOKS="1")

    def stage(name, command, cwd=source, extra=None):
        destination = artifacts / name
        destination.mkdir()
        stage_env = dict(env, RUNNER_TEMP=str(destination), **(extra or {}))
        started = time.monotonic()
        print(f"==> {name}: {' '.join(map(str, command))}", flush=True)
        with (destination / "run.log").open("w") as log:
            process = subprocess.Popen(
                list(map(str, command)), cwd=cwd, env=stage_env,
                stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, errors="replace",
            )
            try:
                for line in process.stdout:
                    print(line, end="", flush=True)
                    log.write(line)
                code = process.wait()
            except BaseException:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
                raise
            finally:
                process.stdout.close()
        results[name] = {"status": "passed" if code == 0 else "failed",
                         "seconds": round(time.monotonic() - started, 1)}
        (artifacts / "results.json").write_text(json.dumps(results, indent=2) + "\n")
        if code:
            raise RuntimeError(f"{name} failed ({code}). See {destination / 'run.log'}")

    stage("release-flow-tests", ["python3", "-m", "unittest", "discover", "-s", "scripts", "-p", "test_release_flow.py"])
    stage("host", ["bash", "scripts/release-check.sh"], extra={
        "CARGO_TARGET_DIR": str(artifacts.parent.parent / "release-target"),
    })
    if not args.skip_linux:
        rust_version = tomllib.loads((source / "rust-toolchain.toml").read_text())["toolchain"]["channel"]
        image = f"terminator-release-check:rust-{rust_version}"
        stage("linux-image", [
            "docker", "build", "--platform", "linux/amd64", "-t", image,
            "--build-arg", f"RUST_VERSION={rust_version}",
            "-f", "scripts/release-linux.Dockerfile", "scripts",
        ])
        destination = artifacts / "linux-amd64"
        destination.mkdir()
        # Copy the snapshot into the container. Do not expose the source or .git
        # as writable host mounts, and do not share macOS Cargo build artifacts.
        stage("linux", [
            "docker", "run", "--rm", "--platform", "linux/amd64",
            "--mount", f"type=bind,src={source},dst=/snapshot,readonly",
            "--mount", f"type=bind,src={destination},dst=/artifacts",
            "--mount", "type=volume,src=terminator-release-linux-target,dst=/build",
            "--mount", "type=volume,src=terminator-release-linux-registry,dst=/root/.cargo/registry",
            "-e", "CARGO_TARGET_DIR=/build", "-e", "RUNNER_TEMP=/artifacts",
            image, "bash", "-euo", "pipefail", "-c",
            "cp -a --no-preserve=ownership /snapshot/. /src/; bash scripts/release-check.sh",
        ])
    if not args.skip_windows:
        (artifacts / "windows-runtime").mkdir()
        stage("windows", [args.windows_runner, source, artifacts / "windows-runtime"])
    (artifacts / "results.json").write_text(json.dumps(results, indent=2) + "\n")


def release(root, args):
    require("cargo")
    require("nvim")
    if sys.platform == "darwin":
        require(os.environ.get("TERMINATOR_ZIG", "zig"))
    if not args.skip_linux:
        require("docker")
        run(["docker", "info", "--format", "{{.Architecture}}"], root)
    if not args.skip_windows:
        if not args.windows_runner:
            raise RuntimeError(
                "Set TERMINATOR_WINDOWS_RUNNER to a Windows runner executable, "
                "or use --skip-windows. Cross-checks do not run Windows tests."
            )
        require(args.windows_runner)
        args.windows_runner = str(Path(shutil.which(args.windows_runner)).resolve())
    for plugin in ("audit", "deny"):
        run(["cargo", plugin, "--version"], root)

    head = git(root, "rev-parse", "HEAD")
    branch = git(root, "symbolic-ref", "--short", "HEAD")
    if not args.check_only:
        if branch != "master":
            raise RuntimeError("Release must run from master. Use --check-only on other branches.")
        require("gh")
        run(["gh", "auth", "status"], root)
        run(["git", "fetch", "--no-tags", "origin", "master"], root)
        remote = git(root, "rev-parse", "refs/remotes/origin/master")
        run(["git", "merge-base", "--is-ancestor", remote, head], root)

    artifacts_parent = root / ".artifacts" / "releases"
    artifacts_parent.mkdir(parents=True, exist_ok=True)
    artifacts = Path(tempfile.mkdtemp(prefix="run-", dir=artifacts_parent))
    print(f"Release logs: {artifacts}", flush=True)
    with tempfile.TemporaryDirectory(prefix="terminator-release-") as temporary:
        temporary = Path(temporary)
        original = source_tree(root, temporary)
        source = temporary / "source"
        source.mkdir()
        # checkout-index exports the exact index, including executable bits and
        # files marked export-ignore (unlike git archive).
        env = dict(os.environ, GIT_INDEX_FILE=str(temporary / "index"))
        git(root, "checkout-index", "--all", f"--prefix={source}/", env=env)
        run(["bash", "scripts/bump-version.sh"], source)
        version = tomllib.loads((source / "Cargo.toml").read_text())["workspace"]["package"]["version"]
        tag = f"v{version}"
        if not args.check_only:
            if git(root, "tag", "--list", tag) or git(root, "ls-remote", "--tags", "origin", f"refs/tags/{tag}"):
                raise RuntimeError(f"Tag {tag} already exists. No files changed.")
        # Give source-dependent tooling a real, isolated Git checkout.
        git(source, "init", "--quiet")
        git(source, "add", "--force", "--all")
        git(source, "-c", "user.name=Release validation", "-c", "user.email=release@localhost",
            "-c", "core.hooksPath=/dev/null", "-c", "commit.gpgsign=false",
            "commit", "--quiet", "-m", f"Validate {tag}")
        validated = git(source, "rev-parse", "HEAD^{tree}")
        (artifacts / "source.json").write_text(json.dumps({
            "base_commit": head, "original_tree": original,
            "validated_tree": validated, "tag": tag,
        }, indent=2) + "\n")
        validate(source, artifacts, args)
        if (git(source, "status", "--porcelain")
                or git(source, "rev-parse", "HEAD^{tree}") != validated):
            raise RuntimeError("Checks changed source files. Review the logs; no release was made.")
        if (git(root, "rev-parse", "HEAD") != head
                or git(root, "symbolic-ref", "--short", "HEAD") != branch
                or source_tree(root, temporary) != original):
            raise RuntimeError("Source changed during validation. Run the checks again.")
        if args.check_only:
            print(f"Selected checks passed for {tag}. Version, index, commits and remote are unchanged.")
            return
        # Only now change the real checkout. Leave a failed commit/push in place
        # for inspection; never reset, clean, force-push or delete a release tag.
        for name in ("Cargo.toml", "Cargo.lock"):
            shutil.copyfile(source / name, root / name)
        git(root, "add", "--all")
        if git(root, "write-tree") != validated:
            raise RuntimeError("Staged source differs from the validated source. Nothing pushed.")
        run(["git", "commit", "-m", f"Release {tag}"], root,
            env=dict(os.environ, TERMINATOR_RELEASE_VALIDATED_TREE=validated))
        if git(root, "rev-parse", "HEAD^{tree}") != validated:
            raise RuntimeError("Commit hooks changed the validated source. Nothing pushed.")
        revision = git(root, "rev-parse", "HEAD")
        git(root, "tag", tag, revision)
        run(["git", "push", "--atomic", "origin", f"{revision}:refs/heads/master", f"refs/tags/{tag}"], root)
        try:
            run(["gh", "workflow", "run", "release.yaml", "--ref", tag], root)
        except subprocess.CalledProcessError as error:
            raise RuntimeError(
                f"{tag} was pushed, but workflow dispatch failed. Retry only: "
                f"gh workflow run release.yaml --ref {tag}"
            ) from error
        print(f"Started Release for {tag} ({revision}). Logs: {artifacts}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check-only", action="store_true", help="Validate without changing versions, commits or remotes")
    parser.add_argument("--skip-linux", action="store_true", help="Explicitly omit Linux execution")
    parser.add_argument("--skip-windows", action="store_true", help="Explicitly omit Windows execution; keep host cross-checks")
    default_runner = os.environ.get("TERMINATOR_WINDOWS_RUNNER")
    if not default_runner and os.environ.get("TERMINATOR_WINDOWS_HOST"):
        default_runner = str(Path(__file__).with_name("release-windows.sh"))
    parser.add_argument("--windows-runner", default=default_runner,
                        help="Executable receiving source and artifact directories; must run release-check.sh on Windows")
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    lock = root / ".artifacts" / "release.lock"
    lock.parent.mkdir(exist_ok=True)
    try:
        lock.mkdir()
    except FileExistsError:
        parser.exit(1, f"Release lock exists: {lock}. Remove it only when no release is running.\n")
    try:
        release(root, args)
    finally:
        lock.rmdir()


if __name__ == "__main__":
    try:
        main()
    except (RuntimeError, subprocess.CalledProcessError, OSError) as error:
        sys.exit(f"Release stopped: {error}")
    except KeyboardInterrupt:
        sys.exit("Release interrupted. No automatic retry was made.")
