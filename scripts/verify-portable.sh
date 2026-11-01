#!/bin/sh
# Fails when a binary needs glibc newer than 2.28 or a library that is not installed.
set -eu
dir="$1"
max_glibc="2.28"
for binary in terminator terminator-daemon terminator-hook; do
    path="$dir/$binary"
    ldd "$path"
    if ldd "$path" 2>&1 | grep -q 'not found'; then
        echo "FAIL: $binary has unresolved libraries" >&2
        exit 1
    fi
    if ldd "$path" 2>&1 | grep -qi webkit; then
        echo "FAIL: $binary links WebKit" >&2
        exit 1
    fi
    newest=$(objdump -T "$path" | grep -o 'GLIBC_[0-9.]*' | sed 's/GLIBC_//' | sort -Vu | tail -n 1)
    echo "$binary newest glibc symbol: $newest"
    highest=$(printf '%s\n%s\n' "$newest" "$max_glibc" | sort -V | tail -n 1)
    if [ "$highest" != "$max_glibc" ]; then
        echo "FAIL: $binary needs glibc $newest (limit $max_glibc)" >&2
        exit 1
    fi
done
echo "portable package OK"
