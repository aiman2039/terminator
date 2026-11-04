---
name: verify
description: Recipe to verify SSH/Mosh remote terminals end to end with a local test sshd and an isolated daemon.
---

# Verify SSH / Mosh terminals

1. `cargo build --workspace --locked`.
2. Test host: user-level `/usr/sbin/sshd -D -e -f <config>` on 127.0.0.1:2299 with temp host/client keys
   (`UsePAM no`, `StrictModes no`, `AuthorizedKeysFile <temp>`). Write `[127.0.0.1]:2299 <host.pub>` to a temp known_hosts.
3. Isolated daemon: `env TERMINATOR_DATA_DIR=<short /tmp dir> TERMINATOR_NO_NOTIFICATIONS=1 target/debug/terminator-daemon &`.
   Poll `terminator-hook ctl --data-dir <dir> list` until it answers.
4. `ctl add-remote-project --host 127.0.0.1 --port 2299 --root $HOME --name t --identity <key> --known-hosts <kh>`.
   Mosh: no ctl flag; send `{"UpdateRemoteProject":{"project":<json with "mosh":true>}}` via `terminator-hook rpc` (with `TERMINATOR_DATA_DIR`).
5. `ctl create --background t` → session `id`; `ctl send --enter <id> 'echo X'`; `ctl read --screen <id>`; `pgrep -fl mosh-client`.

Gotchas:
- Inside a Terminator terminal, unset every inherited `TERMINATOR_*` var first, or the daemon refuses to start
  ("Inherited generation/catalog routing does not match").
- The agent's Bash `ssh` may be refused by permission rules; give the user a script to run with `! sh <script>`.
- Keep the data dir under /tmp (Unix socket path length).
