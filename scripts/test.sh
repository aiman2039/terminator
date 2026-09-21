#!/bin/sh
# Workspace unit tests, real-PTY integration, idle-close, and native GUI fixtures.
# Requires a desktop for GUI cases. Does not run #[ignore] live/network tests,
# load soaks, or account-backed fixtures (codex-live).
set -eu
cd "$(dirname "$0")/.."
export CARGO_HUSKY_DONT_INSTALL_HOOKS=1
export TERMINATOR_DATA_DIR="${TMPDIR:-/tmp}/terminator-test-state"
mkdir -p "$TERMINATOR_DATA_DIR"

run() {
    echo "==> $*"
    "$@"
}

run cargo test --workspace --all-features --locked
# cargo test does not refresh target/debug binaries. xtask launches those.
run cargo build --locked --bin terminator-daemon --bin terminator-hook
run cargo xtask integration
run cargo xtask idle-close

if [ "$(uname -s)" != Darwin ] && [ -z "${DISPLAY:-}${WAYLAND_DISPLAY:-}" ]; then
    echo "No display; native GUI fixtures need a desktop (or Xvfb/Weston)." >&2
    exit 1
fi

run cargo build --workspace --bins --examples --features terminator/test-support --locked
run cargo xtask gui all

echo "All tests passed."
