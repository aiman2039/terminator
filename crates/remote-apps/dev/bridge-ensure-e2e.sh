#!/bin/sh
# End-to-end test of `terminator-remote-bridge --ensure` in the dev image:
# the bridge starts wprsd itself (no xauth tool, its own cookie), records
# the session, a "shell" reads it and starts xclock, a second bridge
# reuses the same wprsd, and a newer bridge takes over from a live one.
# Run from the repository root inside the dev image:
#   cargo build -p terminator-remote-apps --features bridge --bins --examples
#   sh crates/remote-apps/dev/bridge-ensure-e2e.sh
set -eu
out=${CARGO_TARGET_DIR:-target}/debug
probe=$out/examples/bridge_probe
# wprsd and xwayland-xdg-shell come from PATH here, as with nix-portable.
PATH=${WPRS_BIN:-/build/release}:$PATH
TERMINATOR_REMOTE_DIR=$(mktemp -d)/terminator-remote
export TERMINATOR_REMOTE_DIR
session=$TERMINATOR_REMOTE_DIR/session.json
pid_of() { sed -n 's/.*"pid": *\([0-9]*\).*/\1/p' "$1"; }
cleanup() {
    [ -f "$TERMINATOR_REMOTE_DIR/wprsd.json" ] && kill "$(pid_of "$TERMINATOR_REMOTE_DIR/wprsd.json")" 2>/dev/null
    kill ${app:-} ${hold:-} 2>/dev/null || true
}
trap cleanup EXIT

"$probe" "$out/terminator-remote-bridge" --ensure --prime
[ -f "$session" ] || { echo "FAIL: no session.json"; exit 1; }
[ -f "$TERMINATOR_REMOTE_DIR/wprsd.json" ] || { echo "FAIL: no wprsd.json"; exit 1; }
first_pid=$(pid_of "$session")
display=$(sed -n 's/.*"display": *"\([^"]*\)".*/\1/p' "$session")
xauthority=$(sed -n 's/.*"xauthority": *"\([^"]*\)".*/\1/p' "$session")
echo "session: DISPLAY=$display pid $first_pid"

DISPLAY=$display XAUTHORITY=$xauthority xclock -update 1 &
app=$!
sleep 2
kill -0 "$app" 2>/dev/null || { echo "FAIL: xclock exited"; exit 1; }
rc=0
env DISPLAY="$display" XAUTHORITY=/dev/null timeout 3 xclock 2>/dev/null || rc=$?
[ "$rc" != 124 ] || { echo "FAIL: a client without the cookie can connect"; exit 1; }

"$probe" "$out/terminator-remote-bridge" --ensure
pid=$(pid_of "$session")
[ "$pid" = "$first_pid" ] || { echo "FAIL: second bridge started a new wprsd"; exit 1; }

# A bridge that still holds wprsd (a GUI on another computer) is taken over.
"$probe" "$out/terminator-remote-bridge" --ensure --hold &
hold=$!
sleep 3
"$probe" "$out/terminator-remote-bridge" --ensure --prime
wait "$hold" || { echo "FAIL: the older bridge was not taken over"; exit 1; }
hold=
pid=$(pid_of "$session")
[ "$pid" = "$first_pid" ] || { echo "FAIL: takeover started a new wprsd"; exit 1; }
echo "PASS: --ensure started wprsd once, served its window and took over"
