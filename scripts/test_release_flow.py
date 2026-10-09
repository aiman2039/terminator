"""Offline release regressions. All Git writes use disposable repositories."""

import argparse
import base64
import importlib.util
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch

SCRIPTS = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("release_flow", SCRIPTS / "release_flow.py")
flow = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(flow)
WINDOWS_SPEC = importlib.util.spec_from_file_location("release_windows", SCRIPTS / "release_windows.py")
windows = importlib.util.module_from_spec(WINDOWS_SPEC)
WINDOWS_SPEC.loader.exec_module(windows)


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="release-tests-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name) / "repo"
        self.root.mkdir()
        flow.git(self.root, "init", "--quiet", "--initial-branch=master")
        flow.git(self.root, "config", "user.name", "Fixture")
        flow.git(self.root, "config", "user.email", "fixture@localhost")
        flow.git(self.root, "config", "commit.gpgsign", "false")
        flow.git(self.root, "config", "core.hooksPath", "/dev/null")
        (self.root / "scripts").mkdir()
        shutil.copyfile(SCRIPTS / "bump-version.sh", self.root / "scripts/bump-version.sh")
        (self.root / "Cargo.toml").write_text(
            '[workspace]\nmembers = ["crates/sample"]\n[workspace.package]\nversion = "0.1.0"\n'
        )
        member = self.root / "crates/sample"
        member.mkdir(parents=True)
        (member / "Cargo.toml").write_text('[package]\nname = "sample"\nversion.workspace = true\n')
        (self.root / "Cargo.lock").write_text('version = 4\n[[package]]\nname = "sample"\nversion = "0.1.0"\n')
        (self.root / ".gitignore").write_text(".artifacts/\nignored\n")
        (self.root / "tracked").write_text("before\n")
        flow.git(self.root, "add", "--all")
        flow.git(self.root, "commit", "--quiet", "-m", "Initial")
        self.remote = Path(self.temporary.name) / "remote.git"
        subprocess.run(["git", "init", "--bare", "--quiet", self.remote], check=True)
        flow.git(self.root, "remote", "add", "origin", str(self.remote))
        flow.git(self.root, "push", "--quiet", "origin", "master")
        self.args = argparse.Namespace(check_only=True, skip_linux=True, skip_windows=True, windows_runner=None)
        self.commands = []
        original_run = flow.run

        def run(command, cwd, env=None):
            self.commands.append(command)
            if command[0] in ("gh", "cargo"):
                return None
            return original_run(command, cwd, env)

        self.run_patch = patch.object(flow, "run", side_effect=run)
        self.run_patch.start()
        self.addCleanup(self.run_patch.stop)
        self.require_patch = patch.object(flow, "require")
        self.require_patch.start()
        self.addCleanup(self.require_patch.stop)

    def state(self):
        return (flow.git(self.root, "rev-parse", "HEAD"),
                flow.git(self.root, "write-tree"),
                (self.root / "Cargo.toml").read_bytes(),
                (self.root / "Cargo.lock").read_bytes())

    def test_check_only_tests_bumped_snapshot_and_preserves_staged_and_unstaged_work(self):
        (self.root / "tracked").write_text("staged\n")
        flow.git(self.root, "add", "tracked")
        (self.root / "tracked").write_text("unstaged\n")
        (self.root / "new").write_text("new\n")
        (self.root / "ignored").write_text("private\n")
        before = self.state()

        def check(source, artifacts, args):
            self.assertIn('version = "0.2.0"', (source / "Cargo.toml").read_text())
            self.assertEqual((source / "tracked").read_text(), "unstaged\n")
            self.assertTrue((source / "new").exists())
            self.assertFalse((source / "ignored").exists())
            self.assertEqual(self.state(), before)

        with patch.object(flow, "validate", side_effect=check):
            flow.release(self.root, self.args)
        self.assertEqual(self.state(), before)
        self.assertFalse(any(c[:3] == ["gh", "workflow", "run"] for c in self.commands))

    def test_failed_check_does_not_bump_stage_commit_or_push(self):
        before = self.state()
        with patch.object(flow, "validate", side_effect=RuntimeError("fixture test failure")):
            with self.assertRaisesRegex(RuntimeError, "fixture test failure"):
                flow.release(self.root, self.args)
        self.assertEqual(self.state(), before)

    def test_edit_during_checks_stops_release_and_preserves_edit(self):
        before = self.state()

        def edit(source, artifacts, args):
            (self.root / "tracked").write_text("concurrent edit\n")

        with patch.object(flow, "validate", side_effect=edit):
            with self.assertRaisesRegex(RuntimeError, "Source changed"):
                flow.release(self.root, self.args)
        self.assertEqual(self.state(), before)
        self.assertEqual((self.root / "tracked").read_text(), "concurrent edit\n")

    def test_check_that_changes_snapshot_stops_release(self):
        before = self.state()

        def edit(source, artifacts, args):
            (source / "tracked").write_text("modified by test\n")

        with patch.object(flow, "validate", side_effect=edit):
            with self.assertRaisesRegex(RuntimeError, "Checks changed source"):
                flow.release(self.root, self.args)
        self.assertEqual(self.state(), before)

    def test_success_pushes_tested_tree_and_dispatches_immutable_tag(self):
        self.args.check_only = False
        with patch.object(flow, "validate"):
            flow.release(self.root, self.args)
        revision = flow.git(self.root, "rev-parse", "HEAD")
        self.assertEqual(flow.git(self.remote, "rev-parse", "refs/heads/master"), revision)
        self.assertEqual(flow.git(self.remote, "rev-parse", "refs/tags/v0.2.0"), revision)
        self.assertEqual(self.commands[-1], ["gh", "workflow", "run", "release.yaml", "--ref", "v0.2.0"])

    def test_rejected_atomic_push_does_not_dispatch_or_update_remote(self):
        self.args.check_only = False
        before = flow.git(self.remote, "rev-parse", "refs/heads/master")
        hook = self.remote / "hooks/pre-receive"
        hook.write_text("#!/bin/sh\nexit 1\n")
        hook.chmod(0o755)
        with patch.object(flow, "validate"):
            with self.assertRaises(subprocess.CalledProcessError):
                flow.release(self.root, self.args)
        self.assertEqual(flow.git(self.remote, "rev-parse", "refs/heads/master"), before)
        self.assertEqual(flow.git(self.remote, "tag", "--list"), "")
        self.assertFalse(any(c[:3] == ["gh", "workflow", "run"] for c in self.commands))

    def test_missing_windows_runner_stops_before_any_source_change(self):
        self.args.skip_windows = False
        before = self.state()
        with self.assertRaisesRegex(RuntimeError, "TERMINATOR_WINDOWS_RUNNER"):
            flow.release(self.root, self.args)
        self.assertEqual(self.state(), before)

    def test_existing_tag_stops_before_version_change(self):
        self.args.check_only = False
        flow.git(self.root, "tag", "v0.2.0")
        before = self.state()
        with self.assertRaisesRegex(RuntimeError, "already exists"):
            flow.release(self.root, self.args)
        self.assertEqual(self.state(), before)

    def test_commit_hook_change_prevents_push(self):
        self.args.check_only = False
        hooks = self.root / ".git/hooks"
        hooks.mkdir(exist_ok=True)
        hook = hooks / "pre-commit"
        hook.write_text("#!/bin/sh\necho changed > tracked\ngit add tracked\n")
        hook.chmod(0o755)
        flow.git(self.root, "config", "core.hooksPath", str(hooks))
        before = flow.git(self.remote, "rev-parse", "refs/heads/master")
        with patch.object(flow, "validate"):
            with self.assertRaisesRegex(RuntimeError, "Commit hooks changed"):
                flow.release(self.root, self.args)
        self.assertEqual(flow.git(self.remote, "rev-parse", "refs/heads/master"), before)
        self.assertEqual(flow.git(self.remote, "tag", "--list"), "")

    def test_other_branch_cannot_publish(self):
        self.args.check_only = False
        flow.git(self.root, "checkout", "--quiet", "-b", "feature")
        before = self.state()
        with self.assertRaisesRegex(RuntimeError, "must run from master"):
            flow.release(self.root, self.args)
        self.assertEqual(self.state(), before)

    def test_repository_hook_skips_only_the_validated_staged_tree(self):
        hook = SCRIPTS.parent / ".cargo-husky/hooks/pre-commit"
        marker = self.root / "check-ran"
        (self.root / "scripts/check.sh").write_text("#!/bin/sh\ntouch check-ran\nexit 3\n")
        tree = flow.git(self.root, "write-tree")
        env = dict(os.environ, TERMINATOR_RELEASE_VALIDATED_TREE=tree)
        result = subprocess.run(["sh", hook], cwd=self.root, env=env)
        self.assertEqual(result.returncode, 0)
        self.assertFalse(marker.exists())
        env["TERMINATOR_RELEASE_VALIDATED_TREE"] = "different-tree"
        result = subprocess.run(["sh", hook], cwd=self.root, env=env)
        self.assertEqual(result.returncode, 3)
        self.assertTrue(marker.exists())

    def test_dispatch_failure_reports_retry_without_new_version(self):
        self.args.check_only = False
        original_run = flow.run

        def fail_dispatch(command, cwd, env=None):
            if command[:3] == ["gh", "workflow", "run"]:
                raise subprocess.CalledProcessError(1, command)
            return original_run(command, cwd, env)

        with patch.object(flow, "validate"), patch.object(flow, "run", side_effect=fail_dispatch):
            with self.assertRaisesRegex(RuntimeError, "Retry only: gh workflow run release.yaml --ref v0.2.0"):
                flow.release(self.root, self.args)
        self.assertEqual(flow.git(self.remote, "rev-parse", "refs/tags/v0.2.0"),
                         flow.git(self.root, "rev-parse", "HEAD"))


class StageTests(unittest.TestCase):
    def test_failed_host_keeps_logs_and_does_not_start_linux_or_windows(self):
        original_popen = subprocess.Popen
        commands = []

        def execute(command, **kwargs):
            commands.append(command)
            code = 8 if command[0] == "bash" else 0
            return original_popen([os.sys.executable, "-c", f"print('fixture output'); exit({code})"], **kwargs)

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            artifacts = root / "artifacts"
            artifacts.mkdir()
            args = argparse.Namespace(skip_linux=False, skip_windows=False, windows_runner="fixture-runner")
            with patch.object(flow.subprocess, "Popen", side_effect=execute):
                with self.assertRaisesRegex(RuntimeError, "host failed"):
                    flow.validate(root, artifacts, args)
            self.assertEqual(len(commands), 2)
            self.assertIn("fixture output", (artifacts / "host/run.log").read_text())
            self.assertIn('"status": "failed"', (artifacts / "results.json").read_text())


class WindowsRunnerTests(unittest.TestCase):
    def test_ssh_passes_encoded_powershell_without_a_local_shell(self):
        script = "Write-Output 'spaces and $variables'"
        with patch.object(windows.subprocess, "run") as execute:
            windows.powershell("windows-fixture", script)
        command = execute.call_args.args[0]
        self.assertEqual(command[0], "ssh")
        self.assertEqual(base64.b64decode(command[-1]).decode("utf-16-le"), script)
        self.assertNotIn("shell", execute.call_args.kwargs)

    def test_failed_windows_checks_still_retrieve_artifacts_and_propagate_failure(self):
        calls = []

        def remote(host, script, capture=False):
            calls.append(script)
            if len(calls) == 2:
                raise subprocess.CalledProcessError(9, ["ssh"])
            return subprocess.CompletedProcess([], 0, stdout="C:/Users/Fixture/run\n")

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "source"
            source.mkdir()
            (source / "fixture").write_text("source snapshot")
            with patch.dict(os.environ, TERMINATOR_WINDOWS_HOST="windows-fixture"), \
                    patch.object(windows, "powershell", side_effect=remote), \
                    patch.object(windows.subprocess, "run") as copy:
                with self.assertRaises(subprocess.CalledProcessError):
                    windows.check(source, root / "artifacts")
            self.assertEqual(copy.call_count, 2)
            self.assertIn("artifacts.tar.gz", copy.call_args.args[0][2])
            self.assertEqual(len(calls), 3)
            self.assertIn("scripts/release-check.sh", calls[1])


class CheckScriptTests(unittest.TestCase):
    def check(self, fail_test=False):
        with tempfile.TemporaryDirectory(prefix="release-check-tests-") as temporary:
            root = Path(temporary)
            binary = root / "bin"
            binary.mkdir()
            log = root / "commands"
            (binary / "cargo").write_text(
                '#!/bin/sh\nprintf "%s\\n" "$*" >> "$COMMAND_LOG"\n'
                'test "$TERMINATOR_DATA_DIR" = "$RUNNER_TEMP/state" || exit 9\n'
                'test -z "${TERMINATOR_TEST_BIN_DIR-}${TERMINATOR_SESSION_ID-}" || exit 9\n'
                'if [ "${FAIL_TEST:-0}" = 1 ] && [ "$1" = test ]; then exit 7; fi\n'
            )
            (binary / "uname").write_text("#!/bin/sh\necho Darwin\n")
            (binary / "git").write_text("#!/bin/sh\nexit 0\n")
            for program in ("rustc", "nvim"):
                (binary / program).write_text("#!/bin/sh\nexit 0\n")
            for path in binary.iterdir():
                path.chmod(0o755)
            env = dict(os.environ, PATH=f"{binary}:{os.environ['PATH']}", COMMAND_LOG=str(log),
                       RUNNER_TEMP=str(root / "state"), FAIL_TEST=str(int(fail_test)),
                       TERMINATOR_TEST_BIN_DIR="must-clear", TERMINATOR_SESSION_ID="must-clear")
            result = subprocess.run(["bash", SCRIPTS / "release-check.sh"], env=env, capture_output=True, text=True)
            return result, log.read_text().splitlines()

    def test_unit_failure_stops_before_runtime_and_packaging(self):
        result, commands = self.check(fail_test=True)
        self.assertEqual(result.returncode, 7, result.stderr)
        self.assertEqual(commands[-1], "test --workspace --all-features --locked")
        self.assertFalse(any("xtask package" in c or "xtask integration" in c for c in commands))

    def test_success_runs_tests_cross_checks_runtime_gui_and_release_package(self):
        result, commands = self.check()
        self.assertEqual(result.returncode, 0, result.stderr)
        for expected in ("test --workspace --all-features --locked", "xtask integration", "xtask idle-close"):
            self.assertIn(expected, commands)
        self.assertTrue(any("x86_64-pc-windows-msvc" in c for c in commands))
        self.assertTrue(any(c.startswith("xtask gui all") for c in commands))
        self.assertTrue(commands[-1].startswith("xtask package --output"))


if __name__ == "__main__":
    unittest.main()
