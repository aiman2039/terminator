#!/bin/sh
# Run the existing fixture binary without resolving a new Cargo feature graph.
set -eu
cd "$(dirname "$0")/.."

xtask="${CARGO_TARGET_DIR:-target}/debug/xtask"
if [ ! -x "$xtask" ] && [ -x "$xtask.exe" ]; then
    xtask="$xtask.exe"
fi
if [ ! -x "$xtask" ]; then
    echo "Missing built xtask: $xtask. Build the required fixture binaries first." >&2
    exit 1
fi

exec "$xtask" "$@"
