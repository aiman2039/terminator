FROM ubuntu:24.04
ARG RUST_VERSION=1.97.1
ENV DEBIAN_FRONTEND=noninteractive CARGO_HUSKY_DONT_INSTALL_HOOKS=1
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential pkg-config libssl-dev libfontconfig1-dev \
    libx11-dev libxi-dev libxcursor-dev libxrandr-dev libxinerama-dev \
    libgl1-mesa-dev libgl1-mesa-dri libwayland-dev libxkbcommon-dev libxkbcommon-x11-0 \
    libasound2-dev libwebkit2gtk-4.1-dev xvfb xauth dbus-x11 openbox \
    curl ca-certificates git python3 unzip file procps lsof \
    xdg-desktop-portal xdg-desktop-portal-gtk \
    && rm -rf /var/lib/apt/lists/*
ENV PATH=/root/.cargo/bin:/opt/nvim-linux-x86_64/bin:$PATH
RUN curl --fail --location https://sh.rustup.rs -o /tmp/rustup.sh \
    && sh /tmp/rustup.sh -y --profile minimal --default-toolchain "$RUST_VERSION" --component rustfmt,clippy,llvm-tools-preview \
    && cargo install cargo-audit cargo-deny cargo-llvm-cov --locked \
    && rm /tmp/rustup.sh
RUN curl --fail --location https://github.com/neovim/neovim/releases/download/v0.11.6/nvim-linux-x86_64.tar.gz -o /tmp/nvim.tar.gz \
    && tar -xzf /tmp/nvim.tar.gz -C /opt && rm /tmp/nvim.tar.gz
WORKDIR /src
