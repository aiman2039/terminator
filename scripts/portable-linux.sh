#!/bin/sh
# Builds the portable Linux package on AlmaLinux 8 for glibc 2.17 and checks it on a clean CentOS 7 image.
set -eu
cd "$(dirname "$0")/.."
out="$PWD/target/portable-package"
mkdir -p "$out"
# CI builds both images with the Buildx layer cache first (#68).
if [ "${PORTABLE_IMAGES_BUILT:-0}" != 1 ]; then
    docker build -f scripts/portable-linux.Dockerfile --target toolchain -t terminator-portable-build scripts
    docker build -f scripts/portable-linux.Dockerfile --target runtime -t terminator-portable-run scripts
fi
# PORTABLE_CACHE_DIR keeps the Cargo target and registry on the host, so CI
# can save and restore them; local runs keep a named Docker volume.
if [ -n "${PORTABLE_CACHE_DIR:-}" ]; then
    mkdir -p "$PORTABLE_CACHE_DIR/target" "$PORTABLE_CACHE_DIR/registry" "$PORTABLE_CACHE_DIR/git"
    cache="-v $PORTABLE_CACHE_DIR/target:/target -v $PORTABLE_CACHE_DIR/registry:/root/.cargo/registry -v $PORTABLE_CACHE_DIR/git:/root/.cargo/git"
else
    cache="-v terminator-portable-target:/target"
fi
sccache="$(sh scripts/sccache-docker-args.sh)"
# shellcheck disable=SC2086 # $cache and $sccache are docker arguments without spaces in paths.
docker run --rm -v "$PWD":/src -v "$out":/out $cache $sccache \
    -e CARGO_TARGET_DIR=/target -e TERMINATOR_DATA_DIR=/tmp/terminator-data \
    terminator-portable-build cargo run --locked -p xtask -- package --portable --output /out
rm -rf "$out/verify"
mkdir -p "$out/verify"
tar -xzf "$out/terminator-linux.tar.gz" -C "$out/verify"
docker run --rm -v "$out/verify":/verify:ro -v "$PWD/scripts/verify-portable.sh":/verify.sh:ro \
    terminator-portable-run sh /verify.sh /verify/terminator
