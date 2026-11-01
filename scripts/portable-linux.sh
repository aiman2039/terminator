#!/bin/sh
# Builds the portable Linux package on AlmaLinux 8 and checks it on a clean AlmaLinux 8 image.
set -eu
cd "$(dirname "$0")/.."
out="$PWD/target/portable-package"
mkdir -p "$out"
docker build -f scripts/portable-linux.Dockerfile --target toolchain -t terminator-portable-build scripts
docker build -f scripts/portable-linux.Dockerfile --target runtime -t terminator-portable-run scripts
docker run --rm -v "$PWD":/src -v "$out":/out -v terminator-portable-target:/target \
    -e CARGO_TARGET_DIR=/target -e TERMINATOR_DATA_DIR=/tmp/terminator-data \
    terminator-portable-build cargo run --locked -p xtask -- package --portable --output /out
rm -rf "$out/verify"
mkdir -p "$out/verify"
tar -xzf "$out/terminator-linux.tar.gz" -C "$out/verify"
docker run --rm -v "$out/verify":/verify:ro -v "$PWD/scripts/verify-portable.sh":/verify.sh:ro \
    terminator-portable-run sh /verify.sh /verify/terminator
