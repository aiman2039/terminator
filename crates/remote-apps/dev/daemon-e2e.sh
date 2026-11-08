#!/bin/sh
# End-to-end: local isolated daemon -> SSH -> Docker host with wprsd and the
# bridge -> xclock. Checks connect, the window's shared-memory frame, resize,
# close and disconnect. Run on this computer from the repository root after:
#   cargo build --workspace
#   docker build -t terminator-remote-apps-sshd -f crates/remote-apps/dev/sshd.Dockerfile crates/remote-apps/dev
#   (in the dev image) cargo build -p terminator-remote-apps --features bridge --bins
set -u
for v in $(env | sed -n 's/^\(TERMINATOR_[A-Z_]*\)=.*/\1/p'); do unset "$v"; done
bin=$(pwd)/target/debug
port=${PORT:-2300}
t=$(mktemp -d /tmp/rae.XXXX)
ssh-keygen -q -t ed25519 -N '' -f "$t/key"
cp "$t/key.pub" "$t/authorized_keys"
docker run -d --rm --name tra-sshd -p "127.0.0.1:$port:22" \
    -v terminator-remote-apps-build:/build -v "$t:/keys" terminator-remote-apps-sshd >/dev/null
data=$t/d
mkdir "$data"
env TERMINATOR_DATA_DIR="$data" TERMINATOR_NO_NOTIFICATIONS=1 "$bin/terminator-daemon" 2>"$t/daemon.log" &
daemon=$!
trap 'kill $daemon 2>/dev/null; docker rm -f tra-sshd >/dev/null 2>&1' EXIT
fail() { echo "FAIL: $1"; echo "$2" | cut -c1-600; exit 1; }
rpc() { env TERMINATOR_DATA_DIR="$data" "$bin/terminator-hook" rpc "$1"; }
for _ in $(seq 40); do
    ssh-keyscan -p "$port" 127.0.0.1 2>/dev/null | grep ed25519 >"$t/kh" && [ -s "$t/kh" ] \
        && "$bin/terminator-hook" ctl --data-dir "$data" list >/dev/null 2>&1 && break
    sleep 0.5
done

project=$("$bin/terminator-hook" ctl --data-dir "$data" add-remote-project --host 127.0.0.1 \
    --port "$port" --user root --root /root --name e2e --identity "$t/key" --known-hosts "$t/kh")
# The host has no Terminator install: tools come from the build volume.
apps='env PATH=/build/release:/build/debug:/usr/bin:/bin terminator-remote-bridge --ensure'
project=$(echo "$project" | jq -c --arg c "$apps" '.apps_command = $c')
id=$(echo "$project" | jq -r .id)
rpc "{\"UpdateRemoteProject\":{\"project\":$project}}" >/dev/null || fail "update project" ""

state=$(rpc "{\"RemoteAppsConnect\":{\"project\":\"$id\"}}")
session=$(echo "$state" | jq -r .RemoteApps.session)
[ "$session" != null ] || fail "connect" "$state"

# Polls the session until the jq filter is true; prints the last state.
wait_for() {
    for _ in $(seq 60); do
        state=$(rpc "{\"RemoteAppsState\":{\"session\":\"$session\",\"since\":0}}")
        echo "$state" | jq -e ".RemoteApps | $1" >/dev/null 2>&1 && return 0
        echo "$state" | jq -e '.RemoteApps.status == "failed"' >/dev/null 2>&1 && return 1
        sleep 0.5
    done
    return 1
}
wait_for '.status == "connected"' || fail "not connected" "$state"
display=$(echo "$state" | jq -r .RemoteApps.display)
echo "connected, DISPLAY=$display"

docker exec -d tra-sshd sh -c \
    "DISPLAY=$display XAUTHORITY=/tmp/terminator-remote-0/Xauthority xclock -update 1"
wait_for '.windows | length > 0 and .[0].width > 0' || fail "no window" "$state"
echo "$state" | jq -c '.RemoteApps.windows[0] | {id, title, width, height}'
shm=$(echo "$state" | jq -r '.RemoteApps.windows[0].shm')
[ -f "$shm" ] || fail "no shared-memory file $shm" "$state"
win=$(echo "$state" | jq '.RemoteApps.windows[0].id')

rpc "{\"RemoteAppsInput\":{\"session\":\"$session\",\"messages\":[{\"type\":\"configure\",\"id\":$win,\"width\":400,\"height\":300,\"scale\":1,\"focused\":true}]}}" >/dev/null
wait_for '.windows[0].width == 400 and .windows[0].height == 300' || fail "no resize" "$state"
shm=$(echo "$state" | jq -r '.RemoteApps.windows[0].shm')
size=$(wc -c <"$shm" | tr -d ' ')
[ "$size" = $((64 + 400 * 300 * 4)) ] || fail "shared frame is $size bytes" ""
echo "resized to 400x300, shared frame $size bytes"

# A dropped link: the bridge dies, the daemon retries, and the window comes
# back with the same ID because xclock kept running in wprsd.
docker exec tra-sshd pkill -f terminator-remote-bridge
wait_for '.status == "connecting" and .retry_attempt > 0' || fail "no retry" "$state"
echo "$state" | jq -c '.RemoteApps | {status, error, retry_attempt, retry_in_seconds}'
wait_for ".status == \"connected\" and (.windows | map(.id) | index($win)) != null" \
    || fail "window did not come back" "$state"
echo "reconnected, window $win is back"

rpc "{\"RemoteAppsInput\":{\"session\":\"$session\",\"messages\":[{\"type\":\"close_requested\",\"id\":$win}]}}" >/dev/null
wait_for '.windows | length == 0' || fail "window did not close" "$state"
[ ! -e "$shm" ] || fail "shared frame not removed" ""
rpc "{\"RemoteAppsDisconnect\":{\"session\":\"$session\"}}" >/dev/null
wait_for '.status == "disconnected"' || fail "not disconnected" "$state"
echo "PASS: daemon connected over SSH, showed, resized and closed xclock, then disconnected"
