# syntax=docker/dockerfile:1
# Portable Linux build: AlmaLinux 8 (glibc 2.28, RHEL 8 compatible), no WebKitGTK.
ARG RUST_VERSION=1.97.1

FROM almalinux:8 AS toolchain
ARG RUST_VERSION
ENV CARGO_HOME=/root/.cargo RUSTUP_HOME=/root/.rustup \
    PATH=/root/.cargo/bin:/opt/rh/gcc-toolset-13/root/usr/bin:$PATH
RUN dnf -y install dnf-plugins-core epel-release \
    && dnf config-manager --set-enabled powertools \
    && dnf -y install gcc-toolset-13 make cmake pkgconf-pkg-config git curl \
       fontconfig-devel libX11-devel libXi-devel libXcursor-devel libXrandr-devel \
       libXinerama-devel mesa-libGL-devel wayland-devel libxkbcommon-devel \
       libxkbcommon-x11 alsa-lib-devel \
    && dnf clean all
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --profile minimal --default-toolchain ${RUST_VERSION}
WORKDIR /src

# A clean RHEL 8 style host with the desktop libraries the app links, and no WebKit.
FROM almalinux:8 AS runtime
RUN dnf -y install binutils file fontconfig libX11 libXi libXcursor libXrandr libXinerama \
       mesa-libGL libwayland-client libxkbcommon libxkbcommon-x11 alsa-lib \
    && dnf clean all
