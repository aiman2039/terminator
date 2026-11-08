#!/bin/sh
# Builds a static x86_64 Linux terminator-remote-bridge (musl, static
# libxkbcommon), which runs on any x86_64 Linux host, CentOS 7's glibc 2.17
# included. Run from the repository root on this computer (Docker; emulated
# on Apple Silicon, so the first build is slow):
#   sh crates/remote-apps/dev/build-host.sh
# Output: target/host-x86_64/terminator-remote-bridge
# wprsd, xwayland-xdg-shell and Xwayland come from the host (a Terminator
# install, or nix-portable: see docs/REMOTE_APPS.md).
set -eu
mkdir -p target/host-x86_64
docker run --rm --platform linux/amd64 \
    -v "$(pwd):/src" \
    -v terminator-remote-apps-amd64:/build \
    -v terminator-remote-apps-amd64-cargo:/usr/local/cargo/registry \
    -e CARGO_TARGET_DIR=/build -w /src \
    rust:1.97.1-alpine sh crates/remote-apps/dev/build-host-inside.sh
echo "built: target/host-x86_64/terminator-remote-bridge"
