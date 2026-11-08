#!/bin/sh
# End-to-end test of `terminator-remote-bridge --ensure` in the dev image:
# the bridge starts wprsd itself (no xauth tool, its own cookie), records
# the session, a "shell" reads it and starts xclock, and a second bridge
# reuses the same wprsd. Run from the repository root inside the dev image:
#   cargo build -p terminator-remote-apps --features bridge --bins --examples
#   sh crates/remote-apps/dev/bridge-ensure-e2e.sh
set -eu
out=${CARGO_TARGET_DIR:-target}/debug
probe=$out/examples/bridge_probe
# wprsd and xwayland-xdg-shell come from PATH here, as with nix-portable.
PATH=${WPRS_BIN:-/build/release}:$PATH
export XDG_RUNTIME_DIR=$(mktemp -d)
chmod 700 "$XDG_RUNTIME_DIR"
session=$XDG_RUNTIME_DIR/terminator-remote/session.json
cleanup() {
    [ -f "$session" ] && kill "$(sed -n 's/.*"pid": *\([0-9]*\).*/\1/p' "$session")" 2>/dev/null
    kill ${app:-} 2>/dev/null || true
}
trap cleanup EXIT

"$probe" "$out/terminator-remote-bridge" --ensure --prime
[ -f "$session" ] || { echo "FAIL: no session.json"; exit 1; }
first_pid=$(sed -n 's/.*"pid": *\([0-9]*\).*/\1/p' "$session")
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
pid=$(sed -n 's/.*"pid": *\([0-9]*\).*/\1/p' "$session")
[ "$pid" = "$first_pid" ] || { echo "FAIL: second bridge started a new wprsd"; exit 1; }
echo "PASS: --ensure started wprsd once and served its window"
