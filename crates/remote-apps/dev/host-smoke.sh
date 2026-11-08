#!/bin/sh
# Host-side smoke test for remote apps: start wprsd with Xwayland, then start
# an X11 app on its DISPLAY. Finds wprsd in WPRS_BIN, then PATH, then the dev
# image build (/build/release), else runs itself again inside nix-portable
# (~/bin/nix-portable; set NP_LOCATION to local disk first).
# Pass: wprsd and xwayland-xdg-shell keep running and the app stays connected.
# Uses its own temporary runtime directory and stops what it started.
set -eu

if [ -n "${WPRS_BIN:-}" ]; then
    bin=$WPRS_BIN
elif command -v wprsd >/dev/null 2>&1; then
    bin=$(dirname "$(command -v wprsd)")
elif [ -x /build/release/wprsd ]; then
    bin=/build/release
else
    np=$(command -v nix-portable 2>/dev/null || echo "$HOME/bin/nix-portable")
    if [ -x "$np" ] && [ -z "${HOST_SMOKE_IN_NIX:-}" ]; then
        self=$(cd "$(dirname "$0")" && pwd)/$(basename "$0")
        echo "Using nix-portable: $np (first run downloads wprs)"
        exec env HOST_SMOKE_IN_NIX=1 "$np" nix shell nixpkgs#wprs \
            nixpkgs#xwayland nixpkgs#xorg.xauth nixpkgs#xorg.xclock -c sh "$self"
    fi
    echo "FAIL: wprsd not found (set WPRS_BIN, or install nix-portable in ~/bin)"
    exit 1
fi
export XDG_RUNTIME_DIR=$(mktemp -d)
chmod 700 "$XDG_RUNTIME_DIR"
# Headless hosts often have no X socket directory; smithay then fails with
# a bare "No such file or directory". The bridge must create it the same way.
if [ ! -d /tmp/.X11-unix ]; then
    mkdir -p /tmp/.X11-unix
    chmod 1777 /tmp/.X11-unix
fi
# Other X servers (VNC sessions) may already run; pick a free display.
n=100
while [ -e /tmp/.X11-unix/X$n ] || [ -e /tmp/.X$n-lock ]; do n=$((n + 1)); done
display=:$n

# Like vncserver: a private cookie. Without it any user on the host can
# connect, and under proot (nix-portable) the X server refuses local clients.
command -v xauth >/dev/null 2>&1 || { echo "FAIL: xauth not found"; exit 1; }
real_xwayland=$(command -v Xwayland) || { echo "FAIL: Xwayland not found"; exit 1; }
export XAUTHORITY=$XDG_RUNTIME_DIR/Xauthority
: >"$XAUTHORITY"
xauth -q -f "$XAUTHORITY" add "$display" . "$(od -An -tx1 -N16 /dev/urandom | tr -d ' \n')"
# xwayland-xdg-shell starts "Xwayland" from PATH with no way to add
# arguments, so a wrapper adds -auth.
mkdir "$XDG_RUNTIME_DIR/bin"
printf '#!/bin/sh\nexec "%s" -auth "%s" "$@"\n' "$real_xwayland" "$XAUTHORITY" \
    >"$XDG_RUNTIME_DIR/bin/Xwayland"
chmod 755 "$XDG_RUNTIME_DIR/bin/Xwayland"
PATH=$XDG_RUNTIME_DIR/bin:$PATH

"$bin/wprsd" --enable-xwayland=true \
    --xwayland-xdg-shell-path="$bin/xwayland-xdg-shell" \
    --stderr-log-level=warn \
    --xwayland-xdg-shell-args="--stderr-log-level=warn,--display=$n" \
    --log-file="$XDG_RUNTIME_DIR/wprsd.log" 2>"$XDG_RUNTIME_DIR/stderr.log" &
wprsd=$!
trap 'kill $wprsd 2>/dev/null || true' EXIT

for _ in $(seq 100); do
    [ -e /tmp/.X11-unix/X$n ] && break
    sleep 0.2
done
fail() {
    echo "FAIL: $1"
    grep -E "panicked|Fatal|rror|failed|Couldn't|cannot" "$XDG_RUNTIME_DIR/stderr.log" \
        | grep -v "^(EE) *[0-9]*:" | head -n 15 || true
    exit 1
}
[ -e /tmp/.X11-unix/X$n ] || fail "no X display socket for $display"
echo "wprsd pid $wprsd from $bin, DISPLAY=$display"

DISPLAY=$display xclock 2>>"$XDG_RUNTIME_DIR/stderr.log" &
app=$!
trap 'kill $app $wprsd 2>/dev/null || true' EXIT
sleep 3
kill -0 "$app" 2>/dev/null || fail "xclock exited"
kill -0 "$wprsd" 2>/dev/null || fail "wprsd exited"
# Other users must not get in: a client without the cookie is refused.
rc=0
env XAUTHORITY=/dev/null DISPLAY=$display timeout 3 xclock 2>/dev/null || rc=$?
[ "$rc" != 124 ] || fail "a client without the cookie can connect"
echo "PASS: xclock is running on $display through wprsd"
