#!/usr/bin/env bash
# The same quality commands used by CI, plus local runtime and package checks.
set -euo pipefail
cd "$(dirname -- "${BASH_SOURCE[0]}")/.."
: "${RUNNER_TEMP:?Set RUNNER_TEMP to an isolated validation directory}"
export CARGO_HUSKY_DONT_INSTALL_HOOKS=1
export TERMINATOR_DATA_DIR="$RUNNER_TEMP/state"
export TERMINATOR_CONFIG_DIR="$RUNNER_TEMP/config"
export TERMINATOR_RUNTIME_DIR="$RUNNER_TEMP/runtime"
unset TERMINATOR_SESSION_ID TERMINATOR_TEST_BIN_DIR
export TERMINATOR_FIXTURE_RENDERER=glow
export LIBGL_ALWAYS_SOFTWARE=1
mkdir -p "$TERMINATOR_DATA_DIR" "$TERMINATOR_CONFIG_DIR"

run() {
    echo "==> $*"
    "$@"
}

run rustc --version
run cargo --version
run nvim --version
if [[ "$(uname -s)" == Darwin ]]; then
    # Fail early if the Windows cross-compiler setup is missing or incompatible.
    run sh scripts/check.sh windows-clippy
fi
for check in async-boundary fmt lint build clippy test audit deny; do
    run sh scripts/check.sh "$check"
done

case "$(uname -s)" in
    Darwin|Linux) ;;
    MINGW*|MSYS*|CYGWIN*)
        echo "Windows PTY and native GUI fixtures are not yet supported."
        run cargo xtask package --output "$RUNNER_TEMP/package"
        if [[ -n "$(git status --porcelain)" ]]; then
            echo "Validation changed source files." >&2
            exit 1
        fi
        exit 0
        ;;
    *) echo "Unsupported operating system" >&2; exit 2 ;;
esac

run cargo build --locked --bin terminator-daemon --bin terminator-hook
run cargo xtask integration
run cargo xtask idle-close
run cargo build --workspace --bins --examples --features terminator/test-support --locked

if [[ "$(uname -s)" == Linux ]]; then
    # The inner shell expands its own variables and EXIT trap.
    # shellcheck disable=SC2016
    run xvfb-run --auto-servernum --server-args="-screen 0 3200x2000x24" \
        dbus-run-session -- bash -euo pipefail -c '
            openbox > "$RUNNER_TEMP/window-manager.log" 2>&1 &
            manager_pid=$!
            trap '\''kill "$manager_pid" 2>/dev/null || true'\'' EXIT
            cargo xtask gui all --output "$RUNNER_TEMP/native"
        '
else
    run cargo xtask gui all --output "$RUNNER_TEMP/native"
fi
run cargo xtask package --output "$RUNNER_TEMP/package"
# Linux runs the same coverage threshold and exclusions as CI.
if [[ "$(uname -s)" == Linux ]]; then
    run cargo llvm-cov --workspace --all-features --locked --summary-only \
        --fail-under-lines 50 \
        -- --skip signals::tests::hangup_ends_a_spawned_process_group \
           --skip navigation_tests::reconnect_clears_transport_errors_but_preserves_failed_operations
fi
if [[ -n "$(git status --porcelain)" ]]; then
    echo "Validation changed source files." >&2
    exit 1
fi
