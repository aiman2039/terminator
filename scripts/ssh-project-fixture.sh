#!/usr/bin/env bash
set -euo pipefail
repo_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
fixture_keys="${SSH_PROJECT_FIXTURE_KEY_DIR:-${TMPDIR:-/tmp}/terminator-ssh-project-keys}"
export SSH_PROJECT_FIXTURE_KEY_DIR="$fixture_keys"
case "${1:-start}" in
  start)
    mkdir -p "$fixture_keys"
    chmod 700 "$fixture_keys"
    if [[ ! -f "$fixture_keys/id_ed25519" ]]; then
      ssh-keygen -q -t ed25519 -N '' -f "$fixture_keys/id_ed25519"
    fi
    cp "$fixture_keys/id_ed25519.pub" "$fixture_keys/authorized_keys"
    docker compose -p terminator-ssh-project -f "$repo_dir/fixtures/ssh-project/compose.yaml" up --build --wait -d
    docker compose -p terminator-ssh-project -f "$repo_dir/fixtures/ssh-project/compose.yaml" exec -T ssh-project cat /var/lib/ssh-host-keys/ssh_host_ed25519_key.pub | awk '{print "[127.0.0.1]:15932 " $1 " " $2}' > "$fixture_keys/known_hosts"
    chmod 600 "$fixture_keys/known_hosts"
    printf 'SSH host: 127.0.0.1\nPort: 15932\nUser: developer\nRemote root: /workspace/project\nIdentity: %s/id_ed25519\nKnown hosts: %s/known_hosts\n' "$fixture_keys" "$fixture_keys"
    ;;
  status|stop)
    if [[ "$1" == status ]]; then
      docker compose -p terminator-ssh-project -f "$repo_dir/fixtures/ssh-project/compose.yaml" ps
    else
      docker compose -p terminator-ssh-project -f "$repo_dir/fixtures/ssh-project/compose.yaml" stop
    fi
    ;;
  *) printf 'Usage: %s start|status|stop\n' "$0" >&2; exit 2 ;;
esac
