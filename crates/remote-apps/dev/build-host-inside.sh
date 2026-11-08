#!/bin/sh
# The container half of build-host.sh (Alpine, x86_64).
set -eu
apk add -q --no-cache musl-dev pkgconf libxkbcommon-dev libxkbcommon-static
export PKG_CONFIG_ALL_STATIC=1
cargo build --release --locked -p terminator-remote-apps --features bridge \
    --bin terminator-remote-bridge
out=/build/release/terminator-remote-bridge
if ldd "$out" 2>&1 | grep -q '=>'; then
    echo "FAIL: $out is not static:"
    ldd "$out"
    exit 1
fi
cp "$out" /src/target/host-x86_64/terminator-remote-bridge
