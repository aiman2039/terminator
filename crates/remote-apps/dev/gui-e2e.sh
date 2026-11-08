#!/bin/sh
# Live GUI check: Docker SSH host with wprsd, isolated daemon + GUI, a seeded
# remote-app tab (so the GUI connects by itself), then xclock on the host.
# Prints the GUI's remote-app tabs and saves a screenshot of the window.
# macOS, from the repository root, after the builds in daemon-e2e.sh plus
# `cargo build -p terminator --example layout`.
for v in $(env | sed -n 's/^\(TERMINATOR_[A-Z_]*\)=.*/\1/p'); do unset "$v"; done
pad=$(cd "$(dirname "$0")" && pwd)
bin=$(pwd)/target/debug
ROOT=$(mktemp -d /tmp/rg.XXXX)
export TERMINATOR_DATA_DIR=$ROOT TERMINATOR_CONFIG_DIR=$ROOT/appearance
export XDG_CONFIG_HOME=$ROOT/c XDG_DATA_HOME=$ROOT/d XDG_STATE_HOME=$ROOT/s
export TERMINATOR_NO_NOTIFICATIONS=1 TERMINATOR_RENDERER=glow
echo '{"version":2,"ide_mode":false}' >"$ROOT/ui-preferences.json"
ssh-keygen -q -t ed25519 -N '' -f "$ROOT/key"
cp "$ROOT/key.pub" "$ROOT/authorized_keys"
docker run -d --rm --name tra-gui -p 127.0.0.1:2301:22 \
    -v terminator-remote-apps-build:/build -v "$ROOT:/keys" terminator-remote-apps-sshd >/dev/null
"$bin/terminator-daemon" 2>"$ROOT/daemon.log" &
daemon=$!
cleanup() { kill ${gui:-} $daemon 2>/dev/null; docker rm -f tra-gui >/dev/null 2>&1; }
trap cleanup EXIT
for _ in $(seq 40); do
    ssh-keyscan -p 2301 127.0.0.1 2>/dev/null | grep ed25519 >"$ROOT/kh" && [ -s "$ROOT/kh" ] \
        && "$bin/terminator-hook" ctl --data-dir "$ROOT" list >/dev/null 2>&1 && break
    sleep 0.5
done
project=$("$bin/terminator-hook" ctl --data-dir "$ROOT" add-remote-project --host 127.0.0.1 \
    --port 2301 --user root --root /root --name xhost --identity "$ROOT/key" --known-hosts "$ROOT/kh")
apps='env PATH=/build/release:/build/debug:/usr/bin:/bin terminator-remote-bridge --ensure'
project=$(echo "$project" | jq -c --arg c "$apps" '.apps_command = $c')
id=$(echo "$project" | jq -r .id)
rpc() { "$bin/terminator-hook" rpc "$1"; }
rpc "{\"UpdateRemoteProject\":{\"project\":$project}}" >/dev/null
rpc "{\"SelectProject\":{\"project\":\"$id\"}}" >/dev/null
seed="{\"RemoteApp\":{\"project\":\"$id\",\"window\":1,\"title\":\"seed\"}}"
dock=$("$bin/examples/layout" X | jq -c --argjson t "$seed" '.surfaces[0].Main.nodes[0].Leaf.tabs = [$t]')
ws=$(jq -cn --argjson d "$dock" --argjson t "$seed" '{version:12,active:"t1",tabs:[{id:"t1",primary:$t,layout:$d}]}')
rpc "{\"SaveLayout\":{\"project\":\"$id\",\"layout\":$ws}}" >/dev/null
"$bin/terminator" 2>"$ROOT/gui.log" &
gui=$!
for _ in $(seq 60); do
    docker exec tra-gui test -f /tmp/terminator-remote-0/session.json && break
    sleep 0.5
done
display=$(docker exec tra-gui sh -c 'sed -n "s/.*\"display\": *\"\([^\"]*\)\".*/\1/p" /tmp/terminator-remote-0/session.json')
echo "host display: $display"
docker exec -d tra-gui sh -c "DISPLAY=$display XAUTHORITY=/tmp/terminator-remote-0/Xauthority xclock -update 1"
sleep 6
python3 "$pad/gui_snapshot.py" "$ROOT"
wid=$(swift "$pad/window_id.swift" "$gui" 2>/dev/null)
screencapture -x -o -l "$wid" "$ROOT/gui-xclock.png" && echo "screenshot: $ROOT/gui-xclock.png"
grep -i -E "panic|error" "$ROOT/gui.log" | head -5
