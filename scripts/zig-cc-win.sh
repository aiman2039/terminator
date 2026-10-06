#!/bin/sh
# C compiler shim for Windows-target `cargo check`/clippy from macOS/Linux.
#
# Uses zig's bundled MinGW headers as the Windows SDK stand-in. That is only
# sound because these commands never link: `cargo check` and clippy stop at
# metadata, and the C archives cc-rs produces are never consumed by a linker.
# Real linking still happens on `windows-2025` CI with MSVC.
#
# Drops cc-rs `--target=` (zig rejects the triple with the `pc` vendor
# field); `-target` below already selects the triple. Set TERMINATOR_ZIG to
# the zig binary when it is not on PATH (needs zig 0.14.x).
set -eu
zig="${TERMINATOR_ZIG:-zig}"
filtered=""
for a in "$@"; do
    case "$a" in --target=*) continue ;; esac
    filtered="$filtered
$a"
done
old_ifs="$IFS"; IFS="
"; set -- $filtered; IFS="$old_ifs"
exec "$zig" cc -target x86_64-windows-gnu "$@"
