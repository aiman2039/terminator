#!/bin/sh
# Translates MSVC `lib.exe` archive syntax (emitted by cc-rs for `-msvc`
# targets) to GNU `ar`, for the zig Windows-target check in zig-cc-win.sh.
# Only `-out:` and object files are expected here; anything else is dropped
# loudly by the resulting `ar` failure.
set -eu
zig="${TERMINATOR_ZIG:-zig}"
out=""
objs=""
for a in "$@"; do
    case "$a" in
        -out:*) out="${a#-out:}" ;;
        -*) continue ;;
        *)
            objs="$objs
$a"
            ;;
    esac
done
old_ifs="$IFS"; IFS="
"; set -- $objs; IFS="$old_ifs"
exec "$zig" ar crs "$out" "$@"
