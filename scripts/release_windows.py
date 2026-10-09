#!/usr/bin/env python3
"""Run release checks on a Windows OpenSSH host with Git Bash and Rust installed."""

import base64
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import uuid


def powershell(host, script, capture=False):
    encoded = base64.b64encode(script.encode("utf-16-le")).decode("ascii")
    return subprocess.run(
        ["ssh", "-o", "BatchMode=yes", "--", host, "powershell.exe",
         "-NoProfile", "-NonInteractive", "-EncodedCommand", encoded],
        check=True, text=True, stdout=subprocess.PIPE if capture else None,
    )


def check(source, artifacts):
    host = os.environ.get("TERMINATOR_WINDOWS_HOST")
    if not host or host.startswith("-"):
        raise RuntimeError("Set TERMINATOR_WINDOWS_HOST to a Windows OpenSSH host (user@host).")
    bash = os.environ.get("TERMINATOR_WINDOWS_BASH", "C:/Program Files/Git/bin/bash.exe")
    bash_literal = bash.replace("'", "''")
    identifier = uuid.uuid4().hex
    relative = f".terminator-release/{identifier}"
    setup = f"""
$ErrorActionPreference = 'Stop'
$base = Join-Path $env:USERPROFILE '{relative}'
New-Item -ItemType Directory -Force -Path (Join-Path $base 'artifacts') | Out-Null
Write-Output ($base.Replace('\\', '/'))
"""
    remote = powershell(host, setup, capture=True).stdout.strip().splitlines()[-1]
    remote_literal = remote.replace("'", "''")
    artifacts.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="windows-release-") as temporary:
        archive = Path(temporary) / "source.tar.gz"
        with tarfile.open(archive, "w:gz") as bundle:
            bundle.add(source, arcname="source")
        subprocess.run(["scp", "-B", str(archive), f"{host}:{relative}/source.tar.gz"], check=True)
        script = f"""
$ErrorActionPreference = 'Stop'
$base = '{remote_literal}'
tar -xzf (Join-Path $base 'source.tar.gz') -C $base
if ($LASTEXITCODE -ne 0) {{ exit $LASTEXITCODE }}
$env:RUNNER_TEMP = (Join-Path $base 'artifacts').Replace('\\', '/')
$env:CARGO_TARGET_DIR = (Join-Path $env:USERPROFILE '.terminator-release/target').Replace('\\', '/')
$env:CARGO_HUSKY_DONT_INSTALL_HOOKS = '1'
Set-Location (Join-Path $base 'source')
& '{bash_literal}' 'scripts/release-check.sh'
exit $LASTEXITCODE
"""
        failure = None
        try:
            powershell(host, script)
        except subprocess.CalledProcessError as error:
            failure = error
        # Retrieve artifacts after both successful and failed test runs. Keep
        # the remote source and logs for inspection; never delete remote state.
        try:
            powershell(host, f"""
$ErrorActionPreference = 'Stop'
tar -czf '{remote_literal}/artifacts.tar.gz' -C '{remote_literal}/artifacts' .
exit $LASTEXITCODE
""")
            subprocess.run(["scp", "-B", f"{host}:{relative}/artifacts.tar.gz",
                            str(artifacts / "artifacts.tar.gz")], check=True)
        except subprocess.CalledProcessError:
            if failure is None:
                raise
            print(f"Could not download artifacts. Remote logs: {remote}/artifacts", file=sys.stderr)
        if failure is not None:
            raise failure


if __name__ == "__main__":
    try:
        if len(sys.argv) != 3:
            sys.exit("Usage: release_windows.py SOURCE ARTIFACTS")
        check(Path(sys.argv[1]).resolve(), Path(sys.argv[2]).resolve())
    except (RuntimeError, OSError, subprocess.CalledProcessError) as error:
        sys.exit(f"Windows checks stopped: {error}")
