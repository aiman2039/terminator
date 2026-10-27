#!/bin/sh
set -eu
mkdir -p /run/sshd /home/developer/.ssh /var/lib/ssh-host-keys
chmod 700 /home/developer/.ssh
cp /run/fixture/authorized_keys /home/developer/.ssh/authorized_keys
chmod 600 /home/developer/.ssh/authorized_keys
chown -R developer:developer /home/developer/.ssh
if [ ! -f /var/lib/ssh-host-keys/ssh_host_ed25519_key ]; then
    ssh-keygen -q -t ed25519 -N '' -f /var/lib/ssh-host-keys/ssh_host_ed25519_key
fi
chown developer:developer /workspace/project
if [ ! -d /workspace/project/.git ]; then
    runuser -u developer -- sh -c '
        set -eu
        cd /workspace/project
        mkdir -p src "path with spaces"
        printf "# Remote SSH fixture\n\nThis project exists only inside Docker.\n" > README.md
        printf "fn main() { println!(\"remote fixture\"); }\n" > src/main.rs
        printf "spaces and Unicode: café\n" > "path with spaces/note.txt"
        git init -q
        git -c user.name=Fixture -c user.email=fixture@example.invalid add .
        git -c user.name=Fixture -c user.email=fixture@example.invalid commit -qm "Initial fixture"
        printf "\nWorking-tree change.\n" >> README.md
        printf "untracked file\n" > untracked.txt
    '
fi
# A loopback-only HTTP service for port-forward checks; never published.
if [ -n "${FIXTURE_HTTP_PORT:-}" ]; then
    mkdir -p /srv/fixture-http
    printf 'terminator-forward-ok\n' > /srv/fixture-http/marker.txt
    runuser -u developer -- python3 -m http.server "$FIXTURE_HTTP_PORT" --bind 127.0.0.1 --directory /srv/fixture-http >/dev/null 2>&1 &
fi
exec /usr/sbin/sshd -D -e -f "${SSHD_CONFIG:-/etc/ssh/sshd_config.fixture}"
