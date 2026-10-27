#!/usr/bin/env bash
set -euo pipefail
repo_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
fixture_keys="${SSH_PROJECT_FIXTURE_KEY_DIR:-${TMPDIR:-/tmp}/terminator-ssh-project-keys}"
export SSH_PROJECT_FIXTURE_KEY_DIR="$fixture_keys"
# Overridable so `cargo xtask ssh` can run an isolated copy beside a manual one.
project="${SSH_PROJECT_FIXTURE_PROJECT:-terminator-ssh-project}"
export SSH_PROJECT_FIXTURE_PORT="${SSH_PROJECT_FIXTURE_PORT:-15932}"
export SSH_PROJECT_FIXTURE_PW_PORT="${SSH_PROJECT_FIXTURE_PW_PORT:-15933}"
compose() {
  docker compose -p "$project" -f "$repo_dir/fixtures/ssh-project/compose.yaml" "$@"
}
host_key_line() { # service port
  compose exec -T "$1" cat /var/lib/ssh-host-keys/ssh_host_ed25519_key.pub | awk -v port="$2" '{print "[127.0.0.1]:" port " " $1 " " $2}'
}
case "${1:-start}" in
  start)
    mkdir -p "$fixture_keys"
    chmod 700 "$fixture_keys"
    if [[ ! -f "$fixture_keys/id_ed25519" ]]; then
      ssh-keygen -q -t ed25519 -N '' -f "$fixture_keys/id_ed25519"
    fi
    cp "$fixture_keys/id_ed25519.pub" "$fixture_keys/authorized_keys"
    compose up --build --wait -d
    { host_key_line ssh-project "$SSH_PROJECT_FIXTURE_PORT"; host_key_line ssh-project-pw "$SSH_PROJECT_FIXTURE_PW_PORT"; } > "$fixture_keys/known_hosts"
    chmod 600 "$fixture_keys/known_hosts"
    printf 'SSH host: 127.0.0.1\nPort: %s\nUser: developer\nRemote root: /workspace/project\nIdentity: %s/id_ed25519\nKnown hosts: %s/known_hosts\n' "$SSH_PROJECT_FIXTURE_PORT" "$fixture_keys" "$fixture_keys"
    printf 'Password variant: port %s, user pw, password pw (developer key also works; AllowTcpForwarding local; 127.0.0.1:8000 serves marker.txt)\n' "$SSH_PROJECT_FIXTURE_PW_PORT"
    ;;
  rekey)
    # Replaces the host key so clients that trusted the old one see
    # ChangedHostKey. known_hosts in the key directory is left stale on purpose.
    service="${2:-ssh-project}"
    compose exec -T "$service" sh -c 'rm -f /var/lib/ssh-host-keys/ssh_host_ed25519_key /var/lib/ssh-host-keys/ssh_host_ed25519_key.pub && ssh-keygen -q -t ed25519 -N "" -f /var/lib/ssh-host-keys/ssh_host_ed25519_key'
    compose restart "$service"
    compose up --wait -d "$service"
    compose exec -T "$service" ssh-keygen -l -E sha256 -f /var/lib/ssh-host-keys/ssh_host_ed25519_key.pub
    ;;
  status|stop)
    if [[ "$1" == status ]]; then
      compose ps
    else
      compose stop
    fi
    ;;
  down)
    # Removes containers and volumes (project data and host keys).
    compose down -v --remove-orphans
    ;;
  *) printf 'Usage: %s start|status|stop|down|rekey [ssh-project|ssh-project-pw]\n' "$0" >&2; exit 2 ;;
esac
