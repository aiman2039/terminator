# TigerVNC server for `cargo xtask gui vnc`. Xvnc listens on 5901 with
# VncAuth and the fixed fixture password; xsetroot paints a known colour and
# sets a root cursor so the viewer receives a cursor shape.
FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends tigervnc-standalone-server tigervnc-tools x11-xserver-utils \
    && rm -rf /var/lib/apt/lists/*
RUN useradd -m vnc
USER vnc
WORKDIR /home/vnc
RUN mkdir -p .vnc && echo terminator-vnc-fixture | vncpasswd -f > .vnc/passwd && chmod 600 .vnc/passwd
EXPOSE 5901
CMD ["sh", "-c", "Xvnc :1 -rfbport 5901 -SecurityTypes VncAuth -rfbauth /home/vnc/.vnc/passwd -geometry 1024x768 -depth 24 -AlwaysShared & sleep 1; DISPLAY=:1 xsetroot -solid '#2a6f97' -cursor_name left_ptr; wait"]
