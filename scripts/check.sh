#!/bin/sh
# Shared local and CI quality checks. Clippy is the Rust linter.
set -eu
cd "$(dirname "$0")/.."
need_plugin() {
    if ! cargo "$1" --version >/dev/null 2>&1; then
        echo "missing cargo-$1; install with: cargo install cargo-$1 --locked" >&2
        exit 1
    fi
}
windows_toolchain() {
    # Cross-checking Windows from macOS/Linux needs zig's bundled Windows
    # headers as the C toolchain (check-only; real linking stays on
    # windows-2025 CI with MSVC). Fails fast when zig is missing.
    if ! command -v "${TERMINATOR_ZIG:-zig}" >/dev/null 2>&1; then
        echo "Windows-target checks need zig 0.14.x (set TERMINATOR_ZIG=/path/to/zig if not on PATH)" >&2
        exit 1
    fi
    zig_cache="${TMPDIR:-/tmp}/terminator-zigcache"
    mkdir -p "$zig_cache"
    env \
        "CC_x86_64-pc-windows-msvc=$PWD/scripts/zig-cc-win.sh" \
        "CC_x86_64_pc_windows_msvc=$PWD/scripts/zig-cc-win.sh" \
        "AR_x86_64-pc-windows-msvc=$PWD/scripts/zig-ar-msvc.sh" \
        "AR_x86_64_pc_windows_msvc=$PWD/scripts/zig-ar-msvc.sh" \
        "ZIG_LOCAL_CACHE_DIR=$zig_cache" \
        "ZIG_GLOBAL_CACHE_DIR=$zig_cache" \
        "$@"
}
case "${1:-all}" in
    async-boundary) cargo xtask async-boundary ;;
    fmt) cargo fmt --all --check ;;
    lint) cargo check --workspace --all-targets --all-features --locked ;;
    build) cargo build --workspace --all-targets --all-features --locked ;;
    clippy) cargo clippy --workspace --all-targets --all-features --locked -- -D warnings ;;
    test)
        need_plugin nextest
        cargo nextest run --workspace --all-features --locked --profile ci
        cargo test --workspace --all-features --locked --doc
        ;;
    windows-check) windows_toolchain cargo check --workspace --target x86_64-pc-windows-msvc --locked ;;
    windows-clippy) windows_toolchain cargo clippy --workspace --all-targets --all-features --target x86_64-pc-windows-msvc --locked -- -D warnings ;;
    audit)
        need_plugin audit
        cargo audit
        ;;
    deny)
        need_plugin deny
        cargo deny check bans licenses sources
        ;;
    all)
        for check in async-boundary fmt lint build clippy audit deny; do
            sh "$0" "$check"
        done
        ;;
    *) echo "Usage: $0 [all|async-boundary|fmt|lint|build|clippy|test|audit|deny|windows-check|windows-clippy]" >&2; exit 2 ;;
esac
