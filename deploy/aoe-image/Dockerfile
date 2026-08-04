# syntax=docker/dockerfile:1
# Reference image for CityHall workspaces: the aoe binary serving its web
# dashboard, one container per user. Build one image per aoe version:
#
#   docker build --build-arg AOE_VERSION=v0.5.0 -t cityhall/aoe:v0.5.0 deploy/aoe-image/
#
# To run an aoe that has no release yet, compile a commit instead of
# downloading a tarball, and tag it as a version you then pin users to:
#
#   docker build --build-arg AOE_SOURCE=git --build-arg AOE_GIT_SHA=<sha> \
#     -t cityhall/aoe:main-20260804 deploy/aoe-image/
#
# CityHall never runs that path itself; it only ever pulls an image or builds
# the release path below. A source build is slow and memory hungry, so it is a
# thing an operator does deliberately on a machine they control. See
# docs/workspaces.md.
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

# Which stage supplies /out/aoe: `release` downloads a published tarball,
# `git` compiles a commit from source. Declared before the first FROM so a
# stage name can interpolate it. BuildKit builds only the stage that is
# actually reached, so a release build never pays for the toolchain stages and
# a source build never runs the download.
ARG AOE_SOURCE=release

# TARGETARCH is set automatically by buildx; plain `docker build` falls back
# to the build host's architecture.
FROM debian:bookworm-slim AS aoe-release
ARG AOE_VERSION
ARG TARGETARCH

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*

RUN test -n "$AOE_VERSION" || (echo "AOE_VERSION build arg is required" && exit 1) \
    && case "${TARGETARCH:-$(dpkg --print-architecture)}" in \
         amd64) AOE_ARCH=amd64 ;; \
         arm64) AOE_ARCH=arm64 ;; \
         *) echo "unsupported arch: $TARGETARCH" && exit 1 ;; \
       esac \
    && mkdir -p /out \
    # The tarball contains a single arch-suffixed binary (aoe-linux-<arch>).
    && curl -fsSL "https://github.com/agent-of-empires/agent-of-empires/releases/download/${AOE_VERSION}/aoe-linux-${AOE_ARCH}.tar.gz" \
       | tar -xz -C /out \
    && mv "/out/aoe-linux-${AOE_ARCH}" /out/aoe

# Source builds happen in two stages so neither toolchain has to carry the
# other: this one produces the dashboard bundle, the next compiles the binary
# against it. Node, not Rust, because aoe's build.rs shells out to npm.
FROM node:22-bookworm-slim AS aoe-web
ARG AOE_GIT_SHA

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates git \
    && rm -rf /var/lib/apt/lists/*

# A commit, not a ref: CityHall resolves the ref before it ever gets here, so
# the image tag and the version drift label name one immutable build.
# --filter=blob:none fetches every commit (any sha stays checkout-able) while
# leaving the history's file contents on the server.
RUN test -n "$AOE_GIT_SHA" || (echo "AOE_GIT_SHA build arg is required when AOE_SOURCE=git" && exit 1) \
    && git clone --filter=blob:none --no-checkout \
       https://github.com/agent-of-empires/agent-of-empires.git /src \
    && git -C /src checkout --detach "$AOE_GIT_SHA"

RUN cd /src/web && npm ci && npm run build

FROM rust:1-bookworm AS aoe-git

# git2 vendors OpenSSL, which builds with perl; aws-lc-sys builds with cmake;
# the vendored libgit2 links zlib.
RUN apt-get update \
    && apt-get install -y --no-install-recommends cmake perl pkg-config zlib1g-dev \
    && rm -rf /var/lib/apt/lists/*

COPY --from=aoe-web /src /src

# AOE_WEB_DIST makes aoe's build.rs copy a prebuilt bundle and return before it
# looks for npm, so this stage needs no Node. It must not point at
# /src/web/dist: build.rs deletes that directory before copying into it.
COPY --from=aoe-web /src/web/dist /web-dist
ENV AOE_WEB_DIST=/web-dist

# Both defaults are set by what fits a stock 4 GB Docker VM, measured rather
# than guessed, and are args because a bigger builder can afford more.
#
# Two separate memory problems, which is why it takes two knobs. The checkout's
# own .cargo/config.toml asks for 8 parallel rustc jobs, so the dependency graph
# alone can run 8 at once. That one is the job count. The `agent-of-empires` lib
# crate is then a single rustc that no job count can shrink, and at any
# optimization level above 0 it gets OOM-killed on its own; that one is the
# optimization level. aoe's `dev-release` profile does not set opt-level, so it
# inherits release's 3, and a cargo env var is what overrides the checkout's
# config.toml.
ARG CARGO_BUILD_JOBS=2
ARG CARGO_PROFILE_DEV_RELEASE_OPT_LEVEL=0
ENV CARGO_BUILD_JOBS=$CARGO_BUILD_JOBS
ENV CARGO_PROFILE_DEV_RELEASE_OPT_LEVEL=$CARGO_PROFILE_DEV_RELEASE_OPT_LEVEL

# `dev-release` is aoe's own profile for a build without LTO. `serve` is the
# feature that ships the dashboard CityHall proxies to.
#
# An OOM kill arrives as a bare "cannot allocate memory" at the end of a
# truncated log tail, which tells an admin nothing, so name the likely cause
# here where it is still attached to the failure.
RUN cd /src \
    && { cargo build --profile dev-release --features serve \
         || { echo "aoe build failed. If a rustc process was killed (signal 9)," \
                   "the builder ran out of memory: give the docker VM more RAM," \
                   "or lower CARGO_BUILD_JOBS." >&2; exit 1; }; } \
    && mkdir -p /out \
    && cp target/dev-release/aoe /out/aoe

# Resolves to aoe-release or aoe-git. An unknown AOE_SOURCE fails the build
# here, naming the stage it could not find.
FROM aoe-${AOE_SOURCE} AS aoe-binary

# Node is the runtime for the agents aoe launches through npm (claude, codex,
# gemini, pi). aoe's floor is Node 20 and its own container image uses 22.
FROM node:22-bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl git tmux \
    && rm -rf /var/lib/apt/lists/*

COPY --from=aoe-binary /out/aoe /usr/local/bin/aoe
RUN chmod +x /usr/local/bin/aoe

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
