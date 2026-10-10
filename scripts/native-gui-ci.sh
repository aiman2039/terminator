#!/usr/bin/env bash
# Run one release-gating native fixture and keep its logs and screenshots.
set -euo pipefail
cd "$(dirname "$0")/.."

case "${1:-}" in
    focus-editor-close|file-close) fixture_case="$1" ;;
    *) echo "Usage: bash scripts/native-gui-ci.sh {focus-editor-close|file-close}" >&2; exit 2 ;;
esac
: "${RUNNER_TEMP:?Set RUNNER_TEMP to the CI artifact parent directory}"
artifact_dir="$RUNNER_TEMP/terminator-native-gui"
mkdir -p "$artifact_dir"

run_fixture() {
    case "$(uname -s)" in
        Linux)
            # The inner Bash process expands its own variables and EXIT trap.
            # shellcheck disable=SC2016
            xvfb-run --auto-servernum --server-args="-screen 0 1440x900x24" \
                dbus-run-session -- bash -euo pipefail -c '
                    openbox > "$RUNNER_TEMP/terminator-native-gui/window-manager.log" 2>&1 &
                    manager_pid=$!
                    trap '\''kill "$manager_pid" 2>/dev/null || true'\'' EXIT
                    "$@"
                ' _ sh scripts/run-built-xtask.sh gui "$fixture_case" --output "$artifact_dir"
            ;;
        Darwin)
            sh scripts/run-built-xtask.sh gui "$fixture_case" --output "$artifact_dir"
            ;;
        *) echo "Native CI fixtures require macOS or Linux" >&2; return 2 ;;
    esac
}

# pipefail keeps a fixture failure fatal even when tee succeeds.
run_fixture 2>&1 | tee "$artifact_dir/$fixture_case.log"
