#!/bin/sh
# Checks if a host can run Terminator remote apps (docs/REMOTE_APPS.md).
# Run from this computer:  ssh HOST sh -s < crates/remote-apps/dev/host-check.sh
# It only reads; it changes nothing on the host.

fail=0
ok()   { printf 'OK    %s\n' "$1"; }
bad()  { printf 'FAIL  %s\n' "$1"; fail=1; }
warn() { printf 'WARN  %s\n' "$1"; }

has_lib() { ldconfig -p 2>/dev/null | grep -q "$1" || ls /usr/lib*/"$1"* /usr/lib/*/"$1"* >/dev/null 2>&1; }

[ "$(uname -s)" = Linux ] && ok "OS: Linux" || bad "OS: $(uname -s) (Linux only)"

case $(uname -m) in
    x86_64|aarch64) ok "CPU: $(uname -m)" ;;
    *) bad "CPU: $(uname -m) (x86_64 or aarch64 only)" ;;
esac

# nix-portable brings Xwayland, wprs and their libraries (own glibc), so the
# system packages below do not matter when it works.
np=$(command -v nix-portable 2>/dev/null || echo "$HOME/bin/nix-portable")
if [ -x "$np" ] && "$np" nix run nixpkgs#xwayland -- -version 2>&1 | grep -q Xwayland; then
    ok "nix-portable: $np (provides Xwayland and wprs; NP_LOCATION=${NP_LOCATION:-default})"
    grep -q avx2 /proc/cpuinfo && ok "AVX2 (needed by the nix wprs build)" \
        || bad "no AVX2: the nix wprs build needs it"
else
    [ -x "$np" ] && warn "nix-portable at $np cannot run nixpkgs#xwayland"
    if command -v Xwayland >/dev/null 2>&1; then
        ok "Xwayland: $(command -v Xwayland)"
    else
        bad "Xwayland missing (install package: xwayland / xorg-x11-server-Xwayland, or use nix-portable)"
    fi
    has_lib libEGL.so.1 && ok "libEGL" || bad "libEGL missing; Xwayland aborts (install: libegl1 / mesa-libEGL)"
    has_lib libwayland-server.so.0 && ok "libwayland-server" || bad "libwayland-server missing (install: libwayland-server0 / wayland)"
    has_lib libxkbcommon.so.0 && ok "libxkbcommon" || bad "libxkbcommon missing (install: libxkbcommon0 / libxkbcommon)"
fi

if [ -d /tmp/.X11-unix ]; then
    [ -w /tmp/.X11-unix ] && ok "/tmp/.X11-unix writable" || bad "/tmp/.X11-unix not writable"
elif [ -w /tmp ]; then
    ok "/tmp/.X11-unix missing, but /tmp is writable (Terminator creates it)"
else
    bad "/tmp/.X11-unix missing and /tmp not writable"
fi

if [ -n "${XDG_RUNTIME_DIR:-}" ] && [ -w "$XDG_RUNTIME_DIR" ]; then
    ok "XDG_RUNTIME_DIR: $XDG_RUNTIME_DIR"
else
    warn "XDG_RUNTIME_DIR not set (Terminator uses a private directory in /tmp)"
fi

for dir in "$HOME" /tmp; do
    if grep -qs " $(df -P "$dir" 2>/dev/null | awk 'NR==2 {print $6}') .*noexec" /proc/mounts; then
        warn "$dir is mounted noexec"
    fi
done

case $(df -P "$HOME" 2>/dev/null | awk 'NR==2 {print $1}') in
    *:*) warn "home is on NFS (sockets will not be put there)" ;;
esac

used=$(ls /tmp/.X11-unix 2>/dev/null | tr '\n' ' ')
[ -n "$used" ] && warn "X displays in use: $used(Terminator picks a free one)"

[ "$fail" = 0 ] && echo "RESULT: host is supported" || echo "RESULT: host is NOT supported (see FAIL lines)"
exit "$fail"
