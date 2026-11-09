#!/bin/sh
# The container half of build-host.sh (Alpine, x86_64).
set -eu
apk add -q --no-cache musl-dev pkgconf libxkbcommon-dev libxkbcommon-static
export PKG_CONFIG_ALL_STATIC=1
cargo build --release --locked -p terminator-remote-apps --features bridge \
    --bin terminator-remote-bridge
# wprsd and xwayland-xdg-shell from the same vendored wprs as the bridge
# (same protocol, and wprsd sends ReplayDone). The bridge finds them beside
# itself before PATH.
(cd /src/vendor/wprs && cargo build --release --locked \
    --bin wprsd --bin xwayland-xdg-shell)
for bin in terminator-remote-bridge wprsd xwayland-xdg-shell; do
    out=/build/release/$bin
    if ldd "$out" 2>&1 | grep -q '=>'; then
        echo "FAIL: $out is not static:"
        ldd "$out"
        exit 1
    fi
    cp "$out" "/src/target/host-x86_64/$bin"
done
