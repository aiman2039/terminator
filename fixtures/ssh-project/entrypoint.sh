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
exec /usr/sbin/sshd -D -e -f /etc/ssh/sshd_config.fixture
