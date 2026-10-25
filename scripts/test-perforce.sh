#!/usr/bin/env bash
# Real CLI/server checks. Uses only a temporary server over local stdin/stdout.
set -euo pipefail
p4_binary="${P4_BIN:-$(command -v p4)}"
p4d_binary="${P4D_BIN:-$(command -v p4d)}"
fixture="$(mktemp -d "${TMPDIR:-/tmp}/terminator-p4.XXXXXX")"
trap 'rm -rf "$fixture"' EXIT
mkdir -p "$fixture/server" "$fixture/workspace" "$fixture/bin"
ln -s "$p4_binary" "$fixture/bin/p4"
export PATH="$fixture/bin:$PATH"
# Drop inherited Perforce routing before applying fixture-only settings.
while IFS= read -r name; do unset "$name"; done < <(env | sed -n 's/^\(P4[A-Za-z0-9_]*\)=.*/\1/p')
export P4PORT="rsh:\"$p4d_binary\" -r \"$fixture/server\" -L \"$fixture/server.log\" -v security=0 -i"
export P4CLIENT=terminator-fixture P4USER=fixture P4CONFIG=
export P4ENVIRO="$fixture/env" P4TICKETS="$fixture/tickets" P4TRUST="$fixture/trust"
# Disable 2026.1 account bootstrapping restrictions in this private fixture.
"$p4d_binary" -r "$fixture/server" '-cset security=0'
"$p4d_binary" -r "$fixture/server" '-cset dm.user.noautocreate=0'
p4 -d "$fixture/workspace" user -i <<'FORM'
User: fixture
Email: fixture@example.invalid
FullName: Fixture
FORM
p4 -d "$fixture/workspace" client -i <<FORM
Client: terminator-fixture
Owner: fixture
Root: $fixture/workspace
LineEnd: unix
View:
    //depot/... //terminator-fixture/...
FORM
for name in edit.txt delete.txt offline.txt 'special@file#1.txt'; do
    printf 'original\n' > "$fixture/workspace/$name"
    p4 -d "$fixture/workspace" add -f "$fixture/workspace/$name"
done
p4 -d "$fixture/workspace" submit -d 'Initial fixture'
p4 -d "$fixture/workspace" edit "$fixture/workspace/edit.txt"
printf 'opened edit\n' > "$fixture/workspace/edit.txt"
p4 -d "$fixture/workspace" delete "$fixture/workspace/delete.txt"
chmod u+w "$fixture/workspace/offline.txt"
printf 'offline edit\n' > "$fixture/workspace/offline.txt"
printf 'new file\n' > "$fixture/workspace/new file.txt"
export TERMINATOR_P4_FIXTURE="$fixture/workspace"
cargo test -p terminator --lib perforce::tests::isolated_server --locked -- --ignored --nocapture
