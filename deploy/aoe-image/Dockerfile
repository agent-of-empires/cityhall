# syntax=docker/dockerfile:1
# Reference image for CityHall workspaces: the aoe binary serving its web
# dashboard, one container per user. Build one image per aoe version:
#
#   docker build --build-arg AOE_VERSION=v0.5.0 -t cityhall/aoe:v0.5.0 deploy/aoe-image/
#
# The tag must match the workspace settings image template (default
# `cityhall/aoe:{version}`) rendered with the version users are pinned to.
# CityHall starts containers from this image with:
#   aoe serve --host 0.0.0.0 --port 8080 --auth none --behind-proxy \
#     --allowed-host <proxy-host> --cityhall
# and mounts a per-user volume at /home/aoe/.config/agent-of-empires.
#
# The image ships no coding agent. A user installs the one they want from
# inside their workspace, and `entrypoint.sh` keeps that install and the
# agent's login on the volume so both survive a container recreate. An operator
# who wants a fixed set builds a derived image; see docs/workspaces.md.

# Node is the runtime for the agents aoe launches through npm (claude, codex,
# gemini, pi). aoe's floor is Node 20 and its own container image uses 22.
FROM node:22-bookworm-slim

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
    # The tarball contains a single arch-suffixed binary (aoe-linux-<arch>).
    && curl -fsSL "https://github.com/agent-of-empires/agent-of-empires/releases/download/${AOE_VERSION}/aoe-linux-${AOE_ARCH}.tar.gz" \
       | tar -xz -C /usr/local/bin \
    && mv "/usr/local/bin/aoe-linux-${AOE_ARCH}" /usr/local/bin/aoe \
    && chmod +x /usr/local/bin/aoe

# The node base image already owns uid 1000. `aoe` has to keep that uid: it is
# what every volume written by an earlier build of this image is owned by, and a
# uid change would make an existing workspace's data unwritable.
RUN userdel node \
    && rm -rf /home/node \
    && useradd -m -d /home/aoe -u 1000 aoe \
    # Pre-create the app dir so the named volume inherits aoe's ownership on
    # first mount.
    && mkdir -p /home/aoe/.config/agent-of-empires \
    && chown -R aoe:aoe /home/aoe

# The entrypoint is written inline rather than COPYd because the docker
# workspace backend builds this file on stdin, with no build context, so a
# COPY would have nothing to copy from. Keeping it here also keeps it a single
# copy: a sibling script plus this heredoc would be two that can drift.
#
# CityHall mounts one per-user volume, at the aoe data dir, and nothing else.
# Every coding agent keeps its own state elsewhere in $HOME, so without this a
# `codex login` or an installed CLI is gone the next time the container is
# recreated, which happens on a version or credential change. Move those
# locations onto the volume with symlinks so an agent the user installs, and the
# subscription they log into, both survive a restart.
#
# Symlinks rather than CODEX_HOME-style relocation variables: aoe clears the
# environment when it spawns a structured-view agent and forwards a fixed
# allowlist those variables are not on, so an env-based approach would work in a
# terminal session and quietly fail in structured view.
COPY <<"SH" /usr/local/bin/entrypoint.sh
#!/bin/sh
set -e

STORE="$HOME/.config/agent-of-empires/cityhall-agents"

# link <name-under-STORE> <path-under-$HOME> <dir|file>
#
# Already a symlink means a previous boot did this, so there is nothing to do.
# Anything real sitting at the destination came from the image, since the
# container filesystem is fresh on every create and only the volume persists; a
# derived image that bakes an agent can ship default config that way. The
# volume's copy is the user's, so it wins, and the image's copy is moved aside
# rather than deleted. A `file` target is left absent for the agent to create
# through the symlink on first write.
link() {
    src="$STORE/$1"
    dst="$HOME/$2"
    if [ -L "$dst" ]; then
        return 0
    fi
    mkdir -p "$(dirname "$dst")" "$(dirname "$src")"
    if [ -e "$dst" ]; then
        rm -rf "$dst.image-default"
        mv "$dst" "$dst.image-default"
    fi
    if [ "$3" = dir ]; then
        mkdir -p "$src"
    fi
    ln -s "$src" "$dst"
}

link claude        .claude               dir
link claude.json   .claude.json          file
link codex         .codex                dir
link gemini        .gemini               dir
link opencode      .config/opencode      dir
link opencode-data .local/share/opencode dir
# The three install prefixes an agent lands in: npm's global prefix, and the two
# an agent's own curl installer uses. NPM_CONFIG_PREFIX and PATH are set in the
# Dockerfile rather than here, so `docker exec` sees them too.
link npm           .npm-global           dir
link local-bin     .local/bin            dir
link opencode-home .opencode             dir

exec "$@"
SH
RUN chmod +x /usr/local/bin/entrypoint.sh

USER aoe
ENV HOME=/home/aoe

# Where an installed agent lands. `entrypoint.sh` keeps all three on the
# volume. First on PATH so a user can upgrade an agent an operator baked in.
ENV NPM_CONFIG_PREFIX=/home/aoe/.npm-global
ENV PATH=/home/aoe/.npm-global/bin:/home/aoe/.local/bin:/home/aoe/.opencode/bin:$PATH

# CityHall passes the full `aoe serve ...` command as arguments, so the
# entrypoint runs first and then execs it.
ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]

EXPOSE 8080
