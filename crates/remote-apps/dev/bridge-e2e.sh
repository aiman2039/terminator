#!/bin/sh
# End-to-end bridge test in the dev image: wprsd + Xwayland + xclock, then
# bridge_probe drives terminator-remote-bridge like the local daemon would.
# Run from the repository root inside the dev image:
#   cargo build -p terminator-remote-apps --bins --examples
#   sh crates/remote-apps/dev/bridge-e2e.sh
set -eu
bin=${WPRS_BIN:-/build/release}
out=${CARGO_TARGET_DIR:-target}/debug

export XDG_RUNTIME_DIR=$(mktemp -d)
chmod 700 "$XDG_RUNTIME_DIR"
mkdir -p /tmp/.X11-unix
n=100
while [ -e /tmp/.X11-unix/X$n ] || [ -e /tmp/.X$n-lock ]; do n=$((n + 1)); done
export XAUTHORITY=$XDG_RUNTIME_DIR/Xauthority
: >"$XAUTHORITY"
xauth -q -f "$XAUTHORITY" add ":$n" . "$(od -An -tx1 -N16 /dev/urandom | tr -d ' \n')"
mkdir "$XDG_RUNTIME_DIR/bin"
printf '#!/bin/sh\nexec "%s" -auth "%s" "$@"\n' "$(command -v Xwayland)" "$XAUTHORITY" \
    >"$XDG_RUNTIME_DIR/bin/Xwayland"
chmod 755 "$XDG_RUNTIME_DIR/bin/Xwayland"
PATH=$XDG_RUNTIME_DIR/bin:$PATH

"$bin/wprsd" --enable-xwayland=true \
    --xwayland-xdg-shell-path="$bin/xwayland-xdg-shell" \
    --stderr-log-level=${LOG_LEVEL:-warn} \
    --xwayland-xdg-shell-args="--stderr-log-level=${LOG_LEVEL:-warn},--display=$n" \
    2>"$XDG_RUNTIME_DIR/wprsd.log" &
wprsd=$!
trap 'kill $wprsd 2>/dev/null || true' EXIT
for _ in $(seq 100); do
    [ -e /tmp/.X11-unix/X$n ] && break
    sleep 0.2
done

probe=$out/examples/bridge_probe
bridge=$out/terminator-remote-bridge
sock=$XDG_RUNTIME_DIR/wprs.sock

# X windows map only after wprsd has an output; a first bridge connection
# gives it one, as the host launcher must do before it exports DISPLAY.
"$probe" "$bridge" "$sock" --prime

# The app starts while no bridge is connected: the next bridge must get the
# windows that already exist (reattach).
DISPLAY=:$n xclock -update 1 &
app=$!
trap 'kill $app $wprsd 2>/dev/null || true' EXIT
sleep 2
kill -0 "$app" 2>/dev/null || { echo "FAIL: xclock exited"; exit 1; }

"$probe" "$bridge" "$sock"
