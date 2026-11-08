# Test SSH host for remote apps: the dev image (Xwayland, x11-apps) plus
# sshd. Mount the build volume at /build (wprsd in /build/release, the bridge
# in /build/debug) and a public key at /keys/authorized_keys.
FROM terminator-remote-apps-dev

RUN apt-get update && apt-get install -y --no-install-recommends openssh-server \
        xterm xclip \
    && mkdir -p /run/sshd \
    && rm -rf /var/lib/apt/lists/*

CMD ["/usr/sbin/sshd", "-D", "-e", "-o", "StrictModes=no", \
     "-o", "AuthorizedKeysFile=/keys/authorized_keys"]
