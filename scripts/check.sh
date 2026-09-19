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
case "${1:-all}" in
    async-boundary) cargo xtask async-boundary ;;
    fmt) cargo fmt --all --check ;;
    lint) cargo check --workspace --all-targets --all-features --locked ;;
    build) cargo build --workspace --all-targets --all-features --locked ;;
    clippy) cargo clippy --workspace --all-targets --all-features --locked -- -D warnings ;;
    test) cargo test --workspace --all-features --locked ;;
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
    *) echo "Usage: $0 [all|async-boundary|fmt|lint|build|clippy|test|audit|deny]" >&2; exit 2 ;;
esac
