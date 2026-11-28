# syntax=docker/dockerfile:1
# Reference image for CityHall workspaces: the aoe binary serving its web
# dashboard, one container per user. Build one image per aoe version:
#
#   docker build --build-arg AOE_VERSION=v0.5.0 -t ghcr.io/agent-of-empires/cityhall-aoe:v0.5.0 deploy/aoe-image/
#
# To run an aoe that has no release yet, compile a commit instead of
# downloading a tarball, and tag it as a version you then pin users to:
#
#   docker build --build-arg AOE_SOURCE=git --build-arg AOE_GIT_SHA=<full-commit-sha> \
#     -t ghcr.io/agent-of-empires/cityhall-aoe:main-20260804 deploy/aoe-image/
#
# A full commit id, not a branch or a tag: the build refuses anything else, so
# one image tag keeps meaning one build.
#
# CityHall never runs that path itself; it only ever pulls an image or builds
# the release path below. A source build is slow and memory hungry, so it is a
# thing an operator does deliberately on a machine they control. See
# docs/workspaces.md.
#
# The tag must match the workspace settings image template (default
# `ghcr.io/agent-of-empires/cityhall-aoe:{version}`) rendered with the version users are pinned to.
# CityHall starts containers from this image with:
#   aoe serve --host 0.0.0.0 --port 8080 --auth token --behind-proxy \
#     --allowed-host <proxy-host> [--cityhall]
# (--cityhall only for Composer mode; Full mode omits it)
# and mounts a per-user volume at /home/aoe (the whole $HOME).
#
# For version 1.22.0 or later it also passes --dangerously-allow-plaintext-remote;
# aoe's plaintext gate answers 426 without it. The proxy-to-workspace hop is
# plain HTTP inside the docker or pod network (loopback for the process
# backend); a TLS sidecar is the later answer (issue #127).
#
# The image ships no coding agent. A user installs the one they want from
# inside their workspace. The whole $HOME is the volume, so that install and the
# agent's login survive a container recreate. Do not bake files under /home/aoe:
# the mount hides them (docker copies them into a brand new volume once).
#
# An operator can instead name a set in CityHall's workspace settings, which
# arrives as `CITYHALL_AGENTS` and is installed at boot by
# `provision-agents.sh` below. That still installs at run time; an operator who
# wants the set baked in, with no boot-time network at all, builds a derived
# image. See docs/workspaces.md.

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
    && ASSET="aoe-linux-${AOE_ARCH}.tar.gz" \
    && BASE="https://github.com/agent-of-empires/agent-of-empires/releases/download/${AOE_VERSION}" \
    # Verified against the `.sha256` every aoe release publishes beside its
    # assets, so a truncated or substituted tarball fails here instead of being
    # unpacked and run as the workspace. No digest can be hardcoded the way the
    # CityHall image does it: AOE_VERSION is a build arg, so the version is not
    # known when this file is written. That leaves the checksum coming from the
    # same release as the bytes it checks, which catches a damaged download and a
    # partially replaced asset, but not a release an attacker controls outright.
    # Closing that needs signed releases upstream, not a change here.
    #
    # Downloaded to a file rather than piped into tar: bytes that go straight
    # into an extractor cannot be checked first.
    && curl -fsSL -o "/tmp/${ASSET}" "${BASE}/${ASSET}" \
    && curl -fsSL -o "/tmp/${ASSET}.sha256" "${BASE}/${ASSET}.sha256" \
    && printf '%s  /tmp/%s\n' "$(cut -d' ' -f1 "/tmp/${ASSET}.sha256")" "${ASSET}" \
       | sha256sum -c - \
    # The tarball contains a single arch-suffixed binary (aoe-linux-<arch>).
    && tar -xz -C /out -f "/tmp/${ASSET}" \
    && rm "/tmp/${ASSET}" "/tmp/${ASSET}.sha256" \
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
# the image tag and the version drift label name one immutable build. That is
# enforced rather than assumed, because `checkout` would happily take a branch
# or a tag from an operator building this by hand, and then the same image tag
# means something different next week. Requiring the resolved object id to equal
# what was passed rejects branches, tags, and abbreviated revisions in one test.
# --filter=blob:none fetches every commit (any sha stays checkout-able) while
# leaving the history's file contents on the server.
RUN test -n "$AOE_GIT_SHA" || (echo "AOE_GIT_SHA build arg is required when AOE_SOURCE=git" && exit 1) \
    && git clone --filter=blob:none --no-checkout \
       https://github.com/agent-of-empires/agent-of-empires.git /src \
    && AOE_GIT_COMMIT="$(git -C /src rev-parse --verify "${AOE_GIT_SHA}^{commit}")" \
    && { test "$AOE_GIT_COMMIT" = "$AOE_GIT_SHA" \
         || { echo "AOE_GIT_SHA must be a full 40-character commit id, not a branch," \
                   "a tag, or an abbreviated sha. Got '$AOE_GIT_SHA', which resolves" \
                   "to '$AOE_GIT_COMMIT'." >&2; exit 1; }; } \
    && git -C /src checkout --detach "$AOE_GIT_COMMIT"

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

# tini is PID 1 so a coding-agent install kicked off at boot has something to
# reap it. It is deliberately orphaned before aoe starts (see
# `provision-agents.sh` below), and an orphan is adopted by PID 1, which aoe is
# not equipped to be.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl git tini tmux \
    && rm -rf /var/lib/apt/lists/*

COPY --from=aoe-binary /out/aoe /usr/local/bin/aoe
RUN chmod +x /usr/local/bin/aoe

# The node base image already owns uid 1000. `aoe` has to keep that uid: it is
# what every volume written by an earlier build of this image is owned by, and a
# uid change would make an existing workspace's data unwritable.
#
# No -m: the home directory stays empty, so a fresh named volume mounted over it
# starts empty too, which is how the entrypoint tells an old-layout volume from
# a new one. Shell defaults come from /etc/skel instead (see the entrypoint).
RUN userdel node \
    && rm -rf /home/node \
    && useradd -M -d /home/aoe -u 1000 aoe \
    && mkdir /home/aoe \
    && chown aoe:aoe /home/aoe

# PATH and NPM_CONFIG_PREFIX for shells that do not inherit the Dockerfile ENV
# below. Debian's /etc/profile resets PATH for a login shell, and a shell
# started by a terminal multiplexer may be either kind, so both the login path
# (/etc/profile.d) and the interactive non-login path (/etc/bash.bashrc) source
# this one file. User dirs come first so a user can override a baked tool.
COPY <<"SH" /etc/profile.d/cityhall.sh
export NPM_CONFIG_PREFIX="$HOME/.npm-global"
case ":$PATH:" in
*":$HOME/.local/bin:"*) ;;
*) PATH="$HOME/.local/bin:$HOME/.npm-global/bin:$HOME/.cargo/bin:$HOME/.opencode/bin:$PATH"; export PATH ;;
esac
SH
RUN echo '. /etc/profile.d/cityhall.sh' >> /etc/bash.bashrc

# The entrypoint is written inline rather than COPYd because the docker
# workspace backend builds this file on stdin, with no build context, so a
# COPY would have nothing to copy from. Keeping it here also keeps it a single
# copy: a sibling script plus this heredoc would be two that can drift.
#
# CityHall mounts one per-user volume at $HOME, so everything a user installs or
# logs into (agent CLIs, their logins, clones, rc files) survives a container
# recreate. This script therefore only seeds $HOME; it never links anything.
COPY <<"SH" /usr/local/bin/entrypoint.sh
#!/bin/sh
set -e

APP="$HOME/.config/agent-of-empires"
STORE="$APP/cityhall-agents"

# A volume from before $HOME was the volume holds the aoe data dir at its root,
# so under this mount that data sits in $HOME instead of in $APP and aoe would
# start empty. Move every top-level entry into $APP, nothing is deleted.
#
# An old layout is recognised by what only it has at the volume root: files aoe
# itself writes into its data dir (the serve token and lock) or the agent store
# this entrypoint keeps there. Not by an absent $APP: a user who deleted $APP on
# a new volume must not have their whole $HOME moved into it. A leftover staging
# dir means an earlier move was cut short, so it is finished too.
#
# The move targets a staging dir that is renamed into place last, so an
# interrupted move is finished by the next start instead of leaving a half-filled
# $APP that would hide the rest.
old_layout=
[ -d "$APP.migrating" ] && old_layout=1
for marker in serve.token serve.lock cityhall-agents; do
    [ -e "$HOME/$marker" ] && old_layout=1
done
if [ ! -d "$APP" ] && [ -n "$old_layout" ]; then
    echo "entrypoint: moving the old volume layout into $APP" >&2
    mkdir -p "$APP.migrating"
    for entry in "$HOME"/* "$HOME"/.[!.]* "$HOME"/..?*; do
        [ -e "$entry" ] || [ -L "$entry" ] || continue
        [ "$entry" = "$HOME/.config" ] && continue
        mv "$entry" "$APP.migrating/"
    done
    # That install state is not at the paths the agents now use, so the markers
    # claiming it is done would stop it being redone. Kept, under another name.
    # Done before the final rename so both steps land together.
    if [ -f "$APP.migrating/cityhall-agents/provisioned" ]; then
        mv "$APP.migrating/cityhall-agents/provisioned" "$APP.migrating/cityhall-agents/provisioned.old-layout"
    fi
    mv "$APP.migrating" "$APP"
fi

# The mount hides anything the image put in $HOME, so create what aoe needs,
# then seed the shell defaults. cp -n never overwrites a file the user has
# edited, and nothing writes into $HOME after this.
mkdir -p "$APP"
cp -rn /etc/skel/. "$HOME/"

# Install the coding agents CityHall was told this workspace should arrive with,
# if any. Deliberately not in the foreground: CityHall gives a workspace 15
# seconds to answer HTTP before it calls the start failed, and an `npm install
# -g` takes minutes, so installing here before `exec` would make every first
# start fail. Backgrounded, the workspace is usable immediately and the agents
# appear shortly after.
#
# The extra subshell is what makes it safe to leave running. `cmd &` alone would
# leave the installer a child of this shell, whose pid becomes aoe's after the
# `exec` below, and aoe does not reap children. Backgrounding inside a subshell
# that exits immediately orphans the installer, so PID 1 adopts it, and PID 1 is
# tini.
#
# The environment is scrubbed rather than inherited. These installers are
# third-party code, and this container's environment holds the user's provider
# API keys and the bearer token that fetches their whole CityHall config bundle,
# including their decrypted git token. None of that is needed to install a
# package.
if [ -n "${CITYHALL_AGENTS:-}" ]; then
    mkdir -p "$STORE"
    (
        env -i \
            HOME="$HOME" \
            PATH="$PATH" \
            NPM_CONFIG_PREFIX="${NPM_CONFIG_PREFIX:-}" \
            CITYHALL_AGENTS="$CITYHALL_AGENTS" \
            STORE="$STORE" \
            /usr/local/bin/provision-agents.sh \
            </dev/null >>"$STORE/provision.log" 2>&1 &
    )
fi

exec "$@"
SH

RUN chmod +x /usr/local/bin/entrypoint.sh

# Installs the agents named in CITYHALL_AGENTS. Runs detached from the
# entrypoint, so nothing here can delay or fail the workspace's start.
#
# A name is never interpolated into a command: the `case` below is the whole
# name-to-command map, and a name that matches no branch installs nothing. That
# is what makes it safe for the set to come from a CityHall setting.
#
# Success is recorded in a marker file on the volume, and only after the binary
# is actually on PATH. Two things follow. An install that was cut short (an idle
# stop mid-download) leaves no marker and is retried on the next start, where
# checking the binary alone could see a bin symlink over an unusable tree and
# skip it forever. And an agent a user later removes stays removed: this
# installs a default once, it does not police what the user does afterwards.
COPY <<"SH" /usr/local/bin/provision-agents.sh
#!/bin/sh
# No `set -e`: one agent failing to install must not skip the rest, and must
# never be the reason a workspace is short of the others.

MARKERS="$STORE/provisioned"

done_already() {
    [ -f "$MARKERS" ] && grep -qx "$1" "$MARKERS"
}

# Bounded so a hung download, or an installer that decides to prompt on a
# terminal that is not there, cannot sit in the container for its whole life.
run() {
    timeout 10m sh -c "$1" </dev/null
}

install_agent() {
    case "$1" in
    claude)
        binary=claude-agent-acp
        command='npm install -g @agentclientprotocol/claude-agent-acp@latest'
        ;;
    codex)
        binary=codex-acp
        command='npm install -g @agentclientprotocol/codex-acp@latest'
        ;;
    gemini)
        binary=gemini
        command='npm install -g @google/gemini-cli'
        ;;
    opencode)
        binary=opencode
        # Downloaded and then run, rather than piped into a shell: a pipe hides
        # whether the download itself failed, so a truncated script or an error
        # page would be executed as an installer. It is still a script fetched
        # at install time, which is what upstream publishes.
        #
        # bash, not sh: the script upstream serves uses `set -o pipefail`, which
        # dash rejects, so running it with /bin/sh fails on its second line.
        command='curl -fsSL -o /tmp/opencode-install.sh https://opencode.ai/install \
                 && bash /tmp/opencode-install.sh; rc=$?; rm -f /tmp/opencode-install.sh; exit $rc'
        ;;
    *)
        echo "unknown coding agent '$1', skipping it"
        return 0
        ;;
    esac

    if command -v "$binary" >/dev/null 2>&1; then
        # Already there, from an earlier boot, a derived image, or the user's own
        # install. Recorded so a later removal is not undone on the next start.
        echo "$1 is already installed"
        echo "$1" >>"$MARKERS"
        return 0
    fi

    echo "installing $1"
    if ! run "$command"; then
        echo "could not install $1; the workspace is running without it"
        return 0
    fi
    if ! command -v "$binary" >/dev/null 2>&1; then
        echo "installing $1 reported success but left no $binary on PATH"
        return 0
    fi
    echo "$1" >>"$MARKERS"
    echo "installed $1"
}

echo "--- $(date -u '+%Y-%m-%dT%H:%M:%SZ') provisioning: $CITYHALL_AGENTS"
IFS=,
for agent in $CITYHALL_AGENTS; do
    done_already "$agent" || install_agent "$agent"
done
SH

RUN chmod +x /usr/local/bin/provision-agents.sh

USER aoe
ENV HOME=/home/aoe

# CityHall refuses to start an image without this: an older entrypoint would hide
# an old-layout volume's data from the migration above.
LABEL cityhall.home-volume=1

# Where an installed agent lands, set here so `docker exec` sees it too; the
# files under /etc cover shells that reset PATH. First on PATH so a user can
# upgrade an agent an operator baked in.
ENV NPM_CONFIG_PREFIX=/home/aoe/.npm-global
ENV PATH=/home/aoe/.local/bin:/home/aoe/.npm-global/bin:/home/aoe/.cargo/bin:/home/aoe/.opencode/bin:$PATH

# CityHall passes the full `aoe serve ...` command as arguments, so the
# entrypoint runs first and then execs it.
#
# tini is in front so PID 1 reaps the orphaned agent install; `-g` sends a stop
# signal to the whole process group rather than aoe alone, so an install still
# running is torn down with the container instead of being left to race it.
ENTRYPOINT ["/usr/bin/tini", "-g", "--", "/usr/local/bin/entrypoint.sh"]

EXPOSE 8080
