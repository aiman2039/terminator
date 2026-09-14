#!/bin/sh
# Shared local and CI quality checks. Clippy is the Rust linter.
set -eu
cd "$(dirname "$0")/.."
case "${1:-all}" in
    fmt) cargo fmt --all --check ;;
    lint) cargo check --workspace --all-targets --all-features --locked ;;
    build) cargo build --workspace --all-targets --all-features --locked ;;
    clippy) cargo clippy --workspace --all-targets --all-features --locked -- -D warnings ;;
    test) cargo test --workspace --all-features --locked ;;
    all)
        for check in fmt lint build clippy; do
            sh "$0" "$check"
        done
        ;;
    *) echo "Usage: $0 [all|fmt|lint|build|clippy|test]" >&2; exit 2 ;;
esac
