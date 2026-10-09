# syntax=docker/dockerfile:1
ARG RUST_VERSION=1.97.1
ARG TARGETARCH
FROM rust:${RUST_VERSION}-slim-bookworm AS rust-toolchain

# Keep the same Linux distribution as CI, importing only the official Rust tools.
FROM ubuntu:24.04 AS toolchain
ENV DEBIAN_FRONTEND=noninteractive CARGO_HUSKY_DONT_INSTALL_HOOKS=1 \
    CARGO_HOME=/root/.cargo RUSTUP_HOME=/root/.rustup
ENV PATH=/root/.cargo/bin:/opt/nvim/bin:$PATH
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential pkg-config libssl-dev libfontconfig1-dev \
    libx11-dev libxi-dev libxcursor-dev libxrandr-dev libxinerama-dev \
    libgl1-mesa-dev libgl1-mesa-dri libwayland-dev libxkbcommon-dev libxkbcommon-x11-0 \
    libasound2-dev libwebkit2gtk-4.1-dev xvfb xauth dbus-x11 openbox \
    curl ca-certificates git unzip file procps lsof zsh fish \
    xdg-desktop-portal xdg-desktop-portal-gtk \
    && rm -rf /var/lib/apt/lists/*
COPY --from=rust-toolchain /usr/local/cargo/ /root/.cargo/
COPY --from=rust-toolchain /usr/local/rustup/ /root/.rustup/
# xtask supplies the repository's components plus the coverage requirement.
ARG RUST_COMPONENTS="clippy llvm-tools-preview rust-analyzer rustfmt"
RUN rustup component add ${RUST_COMPONENTS}

# Select checksum-verified upstream binaries for the native Docker architecture.
FROM toolchain AS audit-amd64
ADD --checksum=sha256:ab28a1bdb54db4d5d8ad5981cf1f959410370b3d28250dbd35f6a44248620e39 https://github.com/rustsec/rustsec/releases/download/cargo-audit/v0.22.2/cargo-audit-x86_64-unknown-linux-gnu-v0.22.2.tgz /tmp/audit.tgz
RUN tar -xzf /tmp/audit.tgz --strip-components=1 -C /root/.cargo/bin cargo-audit-x86_64-unknown-linux-gnu-v0.22.2/cargo-audit

FROM toolchain AS audit-arm64
ADD --checksum=sha256:c6603814ddaa45e51263dafd31c0ac98808f688d26f7395804f9670b0fd599dd https://github.com/rustsec/rustsec/releases/download/cargo-audit/v0.22.2/cargo-audit-aarch64-unknown-linux-gnu-v0.22.2.tgz /tmp/audit.tgz
RUN tar -xzf /tmp/audit.tgz --strip-components=1 -C /root/.cargo/bin cargo-audit-aarch64-unknown-linux-gnu-v0.22.2/cargo-audit

FROM audit-${TARGETARCH} AS audit

FROM toolchain AS deny-amd64
ADD --checksum=sha256:9f12ed4c49936e09b48bf862b595cde2fe64fcbd9d74dfacac6131ca824c8d5f https://github.com/EmbarkStudios/cargo-deny/releases/download/0.20.2/cargo-deny-0.20.2-x86_64-unknown-linux-musl.tar.gz /tmp/deny.tgz
RUN tar -xzf /tmp/deny.tgz --strip-components=1 -C /root/.cargo/bin cargo-deny-0.20.2-x86_64-unknown-linux-musl/cargo-deny

FROM toolchain AS deny-arm64
ADD --checksum=sha256:995c82be0defc7a025cae49a2aa2644ce8245c9a3318fc4103907c6a285e8c7d https://github.com/EmbarkStudios/cargo-deny/releases/download/0.20.2/cargo-deny-0.20.2-aarch64-unknown-linux-musl.tar.gz /tmp/deny.tgz
RUN tar -xzf /tmp/deny.tgz --strip-components=1 -C /root/.cargo/bin cargo-deny-0.20.2-aarch64-unknown-linux-musl/cargo-deny

FROM deny-${TARGETARCH} AS deny

FROM toolchain AS coverage-amd64
ADD --checksum=sha256:9a75fe29538d3800b3da57f6f6efb64cba5c720a257bf0cb8b51f39d495a9168 https://github.com/taiki-e/cargo-llvm-cov/releases/download/v0.8.7/cargo-llvm-cov-x86_64-unknown-linux-gnu.tar.gz /tmp/coverage.tgz
RUN tar -xzf /tmp/coverage.tgz -C /root/.cargo/bin cargo-llvm-cov

FROM toolchain AS coverage-arm64
ADD --checksum=sha256:8f399d84993d13998b63fbe1084377713c719b00655c7d88d5b56c8c29105d90 https://github.com/taiki-e/cargo-llvm-cov/releases/download/v0.8.7/cargo-llvm-cov-aarch64-unknown-linux-gnu.tar.gz /tmp/coverage.tgz
RUN tar -xzf /tmp/coverage.tgz -C /root/.cargo/bin cargo-llvm-cov

FROM coverage-${TARGETARCH} AS coverage

FROM toolchain AS nvim-amd64
ADD --checksum=sha256:2fc90b962327f73a78afbfb8203fd19db8db9cdf4ee5e2bef84704339add89cc https://github.com/neovim/neovim/releases/download/v0.11.6/nvim-linux-x86_64.tar.gz /tmp/nvim.tar.gz
RUN tar -xzf /tmp/nvim.tar.gz -C /opt && mv /opt/nvim-linux-x86_64 /opt/nvim

FROM toolchain AS nvim-arm64
ADD --checksum=sha256:8ddc0c101846145e830b17bbca50782ca9307eee4fab539d9e2ddaf8793c06f1 https://github.com/neovim/neovim/releases/download/v0.11.6/nvim-linux-arm64.tar.gz /tmp/nvim.tar.gz
RUN tar -xzf /tmp/nvim.tar.gz -C /opt && mv /opt/nvim-linux-arm64 /opt/nvim

FROM nvim-${TARGETARCH} AS nvim

FROM toolchain AS final
COPY --from=audit /root/.cargo/bin/cargo-audit /root/.cargo/bin/cargo-audit
COPY --from=deny /root/.cargo/bin/cargo-deny /root/.cargo/bin/cargo-deny
COPY --from=coverage /root/.cargo/bin/cargo-llvm-cov /root/.cargo/bin/cargo-llvm-cov
COPY --from=nvim /opt/nvim /opt/nvim
WORKDIR /src
