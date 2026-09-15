#!/bin/bash
# Isolated debug GUI beside an installed/DMG Terminator.
# Run in Terminal.app or iTerm. Agent shells often SIGKILL long cargo jobs.
#
#   sh scripts/debug-local.sh           # workspace bins + GUI
#   sh scripts/debug-local.sh --app     # ad-hoc .app in target/debug-local
set -euo pipefail
cd "$(dirname "$0")/.."

# Do not inherit a live install's data/runtime/config from the shell.
DATA="${TERMINATOR_DEBUG_DATA_DIR:-/tmp/terminator-ux}"
mkdir -p "$DATA/run"
export TERMINATOR_DATA_DIR="$DATA"
export TERMINATOR_RUNTIME_DIR="$DATA/run"
export TERMINATOR_CONFIG_DIR="$DATA"

LOCK="$TERMINATOR_RUNTIME_DIR/ui.lock"
if [ -e "$LOCK" ]; then
    HOLDERS="$(lsof -t "$LOCK" 2>/dev/null || true)"
    if [ -n "$HOLDERS" ]; then
        echo "A Terminator GUI already holds $LOCK (pid $HOLDERS)." >&2
        echo "Use that window, or quit it and retry. Do not kill the DMG app." >&2
        ps -p $HOLDERS -o pid=,command= >&2 || true
        exit 1
    fi
fi

echo "Isolated data: $DATA" >&2
echo "Installed Terminator can keep running; this GUI uses its own daemon." >&2

if [ "${1:-}" = "--app" ]; then
    # Unique output so package does not rename/delete target/package/Terminator.app.
    # Exec the inner binary (not `open`) and keep TERMINATOR_DATA_DIR set so
    # preflight does not treat this target/*.app as an uninstalled disk image.
    OUT="${PWD}/target/debug-local"
    mkdir -p "$OUT"
    cargo xtask package --debug --output "$OUT"
    BIN="$OUT/Terminator.app/Contents/MacOS/terminator"
    if [ ! -x "$BIN" ]; then
        echo "missing $BIN" >&2
        exit 1
    fi
    exec "$BIN"
fi

# cargo run -p terminator does not guarantee sibling daemon/hook binaries.
cargo build --workspace --bins --locked
BIN="${CARGO_TARGET_DIR:-target}/debug/terminator"
if [ ! -x "$BIN" ]; then
    echo "missing $BIN" >&2
    exit 1
fi
exec "$BIN"
