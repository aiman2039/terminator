#!/bin/sh
# Proves Cargo builds use the sccache storage: a crate unique to this run must
# upload on its first build and be a cache hit after `cargo clean`.
#
# Expects RUSTC_WRAPPER=sccache and a running server, as set up by
# .github/actions/sccache. Run by .github/workflows/sccache-smoke.yaml.
set -eu

die() {
    echo "::error title=sccache smoke test::$1"
    sccache --show-stats || true
    exit 1
}

stat() { sccache --show-stats --stats-format=json | jq -r "$1"; }

[ "${RUSTC_WRAPPER:-}" = sccache ] || die "RUSTC_WRAPPER is '${RUSTC_WRAPPER:-}', so builds bypass sccache."

dir="$(mktemp -d)"
trap 'rm -rf "$dir"' EXIT
cd "$dir"
export CARGO_INCREMENTAL=0

mkdir src
cat >Cargo.toml <<'EOF'
[package]
name = "sccache_smoke"
version = "0.0.0"
edition = "2021"

[workspace]
EOF
# The unique source guarantees the first build cannot hit an older entry.
printf 'pub const RUN: &str = "%s-%s-%s";\n' \
    "${GITHUB_RUN_ID:-local}" "${GITHUB_RUN_ATTEMPT:-0}" "$(basename "$dir")" >src/lib.rs

writes_before="$(stat '.stats.cache_writes')"
hits_before="$(stat '.stats.cache_hits.counts.Rust // 0')"

echo "First build: expect a miss and an upload"
cargo build -v

# sccache uploads in the background after the compile returns.
waited=0
while [ "$waited" -lt 60 ] && [ "$(stat '.stats.cache_writes')" -le "$writes_before" ]; do
    [ "$(stat '.stats.cache_write_errors')" -eq 0 ] || die "Upload to the cache failed. See the server log step."
    sleep 1
    waited=$((waited + 1))
done
[ "$(stat '.stats.cache_writes')" -gt "$writes_before" ] || die "Nothing was uploaded within 60s."

echo "Second build: expect a hit"
cargo clean
cargo build -v
[ "$(stat '.stats.cache_hits.counts.Rust // 0')" -gt "$hits_before" ] || die "Second build was not a cache hit."

sccache --show-stats
echo "sccache smoke test passed"
