#!/bin/sh
# CityHall mounts one per-user volume, at the aoe data dir, and nothing else.
# Every coding agent keeps its own state somewhere else in $HOME, so without
# this a `codex login` or an installed CLI is gone the next time the container
# is recreated, which happens on a version change or a credential change. Move
# those locations onto the volume with symlinks so an agent the user installs,
# and the subscription they log into, both survive a restart.
#
# Symlinks rather than CODEX_HOME-style relocation variables: aoe clears the
# environment when it spawns a structured-view agent and forwards a fixed
# allowlist, which those variables are not on, so an env-based approach would
# work in a terminal session and quietly fail in structured view.
set -e

STORE="$HOME/.config/agent-of-empires/cityhall-agents"

# link <name-under-STORE> <path-under-$HOME> <dir|file>
#
# Already a symlink means a previous boot did this, so there is nothing to do.
# Anything real sitting at the destination came from the image, since the
# container filesystem is fresh on every create and only the volume persists;
# a derived image that bakes an agent can ship default config that way. The
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
# The three install prefixes an agent lands in: npm's global prefix, and the
# two an agent's own curl installer uses. NPM_CONFIG_PREFIX and PATH are set in
# the Dockerfile rather than here, so `docker exec` sees them too.
link npm           .npm-global           dir
link local-bin     .local/bin            dir
link opencode-home .opencode             dir

exec "$@"
