# syntax=docker/dockerfile:1
# Reference image for CityHall workspaces: the aoe binary serving its web
# dashboard, one container per user. Build one image per aoe version:
#
#   docker build --build-arg AOE_VERSION=v0.5.0 -t cityhall/aoe:v0.5.0 deploy/aoe-image/
#
# To run an aoe that has no release yet, compile a commit instead of
# downloading a tarball, and tag it as a version you then pin users to:
#
#   docker build --build-arg AOE_SOURCE=git --build-arg AOE_GIT_SHA=<full-commit-sha> \
#     -t cityhall/aoe:main-20260804 deploy/aoe-image/
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
# `cityhall/aoe:{version}`) rendered with the version users are pinned to.
# CityHall starts containers from this image with:
#   aoe serve --host 0.0.0.0 --port 8080 --auth none --behind-proxy \
#     --allowed-host <proxy-host> --cityhall
# and mounts a per-user volume at /home/aoe/.config/agent-of-empires.
#
# The image ships no coding agent. A user installs the one they want from
# inside their workspace, and `entrypoint.sh` keeps that install and the
# agent's login on the volume so both survive a container recreate.
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

# Where an installed agent lands. `entrypoint.sh` keeps all three on the
# volume. First on PATH so a user can upgrade an agent an operator baked in.
ENV NPM_CONFIG_PREFIX=/home/aoe/.npm-global
ENV PATH=/home/aoe/.npm-global/bin:/home/aoe/.local/bin:/home/aoe/.opencode/bin:$PATH

# CityHall passes the full `aoe serve ...` command as arguments, so the
# entrypoint runs first and then execs it.
#
# tini is in front so PID 1 reaps the orphaned agent install; `-g` sends a stop
# signal to the whole process group rather than aoe alone, so an install still
# running is torn down with the container instead of being left to race it.
ENTRYPOINT ["/usr/bin/tini", "-g", "--", "/usr/local/bin/entrypoint.sh"]

EXPOSE 8080
