# Reference image for CityHall workspaces: the aoe binary serving its web
# dashboard, one container per user. Build one image per aoe version:
#
#   docker build --build-arg AOE_VERSION=v0.5.0 -t cityhall/aoe:v0.5.0 deploy/aoe-image/
#
# The tag must match the workspace settings image template (default
# `cityhall/aoe:{version}`) rendered with the version users are pinned to.
# CityHall starts containers from this image with:
#   aoe serve --host 0.0.0.0 --port 8080 --auth none --behind-proxy
# and mounts a per-user volume at /home/aoe/.config/agent-of-empires.

FROM debian:bookworm-slim

# TARGETARCH is set automatically by buildx; plain `docker build` falls back
# to the build host's architecture.
ARG AOE_VERSION
ARG TARGETARCH

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl git tmux \
    && rm -rf /var/lib/apt/lists/*

RUN test -n "$AOE_VERSION" || (echo "AOE_VERSION build arg is required" && exit 1) \
    && case "${TARGETARCH:-$(dpkg --print-architecture)}" in \
         amd64) AOE_ARCH=amd64 ;; \
         arm64) AOE_ARCH=arm64 ;; \
         *) echo "unsupported arch: $TARGETARCH" && exit 1 ;; \
       esac \
    && curl -fsSL "https://github.com/agent-of-empires/agent-of-empires/releases/download/${AOE_VERSION}/aoe-linux-${AOE_ARCH}.tar.gz" \
       | tar -xz -C /usr/local/bin \
    && chmod +x /usr/local/bin/aoe

RUN useradd -m -d /home/aoe aoe \
    # Pre-create the app dir so the named volume inherits aoe's ownership on
    # first mount.
    && mkdir -p /home/aoe/.config/agent-of-empires \
    && chown -R aoe:aoe /home/aoe

USER aoe
ENV HOME=/home/aoe

EXPOSE 8080
