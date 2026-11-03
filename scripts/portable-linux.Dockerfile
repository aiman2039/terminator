# syntax=docker/dockerfile:1
# Portable Linux build: AlmaLinux 8 toolchain, linked for glibc 2.17 (RHEL/CentOS 7)
# through cargo-zigbuild, no WebKitGTK.
ARG RUST_VERSION=1.97.1

FROM almalinux:8 AS toolchain
ARG RUST_VERSION
ENV CARGO_HOME=/root/.cargo RUSTUP_HOME=/root/.rustup \
    PATH=/root/.cargo/bin:/opt/rh/gcc-toolset-13/root/usr/bin:$PATH
RUN dnf -y install dnf-plugins-core epel-release \
    && dnf config-manager --set-enabled powertools \
    && dnf -y install gcc-toolset-13 make cmake pkgconf-pkg-config git curl \
       python3.11-pip \
       fontconfig-devel libX11-devel libXi-devel libXcursor-devel libXrandr-devel \
       libXinerama-devel mesa-libGL-devel wayland-devel libxkbcommon-devel \
       libxkbcommon-x11 alsa-lib-devel \
    && dnf clean all
# The PyPI package ships cargo-zigbuild and pulls in a matching Zig.
RUN pip3.11 install --no-cache-dir cargo-zigbuild \
    && ln -s "$(python3.11 -c 'import ziglang, os; print(os.path.dirname(ziglang.__file__))')/zig" /usr/local/bin/zig \
    && zig version
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --profile minimal --default-toolchain ${RUST_VERSION}
WORKDIR /src

# A clean CentOS 7 host (glibc 2.17) with the desktop libraries the app links, and no WebKit.
FROM centos:7 AS runtime
RUN sed -i -e 's/^mirrorlist=/#mirrorlist=/' \
       -e 's|^#baseurl=http://mirror.centos.org/centos/$releasever|baseurl=http://vault.centos.org/7.9.2009|' \
       -e 's|^#baseurl=http://mirror.centos.org/altarch/$releasever|baseurl=http://vault.centos.org/altarch/7.9.2009|' \
       /etc/yum.repos.d/CentOS-*.repo \
    && yum -y install binutils file fontconfig libX11 libXi libXcursor libXrandr libXinerama \
       mesa-libGL libwayland-client libxkbcommon libxkbcommon-x11 alsa-lib \
    && yum clean all
