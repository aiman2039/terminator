# Linux host image for remote-apps development: builds wprsd,
# xwayland-xdg-shell and the host bridge, and runs X11 test apps.
# The repository is mounted at run time (scripts/remote-apps-dev.sh), so the
# image holds only the toolchain and system packages.
FROM rust:1.97.1-trixie

RUN apt-get update && apt-get install -y --no-install-recommends \
        libxkbcommon-dev libwayland-dev pkg-config \
        xwayland x11-apps xauth \
        # Xwayland aborts at start without libEGL (it probes EGL through
        # libepoxy even with no GPU).
        libegl1 \
    && rm -rf /var/lib/apt/lists/*

ENV CARGO_TARGET_DIR=/build
WORKDIR /src
