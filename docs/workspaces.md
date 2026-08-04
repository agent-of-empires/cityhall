# Workspaces

A workspace is one long-lived [aoe](https://github.com/agent-of-empires/agent-of-empires)
instance per user, spawned and managed by CityHall (docker containers by
default; kubernetes and bare-process backends are available, see
[Backends](#backends)). Each workspace has a persistent data volume, so aoe
sessions and configuration survive stops, restarts, and version changes.

Every workspace runs with `--cityhall`, aoe's locked-down client mode: users
get the message composer and the structured (chat) view only, with terminal
and diff panes, project management, and advanced settings hidden in the UI and
refused server-side. The flag requires an aoe version that supports it;
starting a workspace on an older version fails with an unknown-argument error
from `aoe serve`. If no release has it yet, see
[Unreleased aoe](#unreleased-aoe).

## How it works

- **Request-driven start.** Opening the workspace (the "Open workspace" link,
  or any request to the workspace proxy) starts the container if needed and
  resumes it if stopped. There is no manual start step for users.
- **Idle stop.** A workspace with no traffic for the configured idle window
  (default 30 minutes) is stopped automatically. Open WebSocket connections
  (live terminals) count as activity, so an open dashboard is never cut off.
  Stopping keeps the volume; the next request resumes with all data intact.
- **Destroy** (admin action) removes the container AND its volume. This
  deletes the user's aoe data permanently.
- **Versions.** Admins set a default aoe version and can pin individual users
  (or a selected group) to a specific version. A version change recreates the
  container on its next start, keeping the volume. The image used is the
  settings' image template with `{version}` substituted, e.g.
  `cityhall/aoe:v0.5.0`. A version is normally a release tag, and can also be
  an unreleased commit ([Unreleased aoe](#unreleased-aoe)).

## Setup

Workspaces are always on and provision themselves. On the docker backend a
missing image is pulled from the registry the image template points at, and
when that fails (the default `cityhall/aoe:{version}` template is not a
published image) it is built locally from the reference Dockerfile; on the
process backend a missing binary is downloaded from the version's release
tarball. The first start of a new version therefore takes a few minutes; the
admin Workspaces page shows the progress, and requests get a retry-shortly
error until the artifact is ready. The kubernetes backend cannot be
auto-built: point the image template at a registry the cluster can pull.

Pre-building is still possible to skip the first-start wait, or to push to a
registry:

```sh
docker build --build-arg AOE_VERSION=v0.5.0 -t cityhall/aoe:v0.5.0 deploy/aoe-image/
```

Version fields offer the discovered stable aoe releases, fetched from the
GitHub API and cached for an hour (the last known list is served when GitHub
is unreachable; set `GITHUB_TOKEN` if the unauthenticated per-IP rate limit
is a problem). On a first startup the default version is pre-filled with the
latest release (skipped when offline); adjust it under **Settings →
Workspaces** if needed. Starting a workspace with no default version set
fails with a descriptive error.

### Unreleased aoe

A version field also accepts `git:<ref>`, where the ref is a branch, tag, or
commit sha in the aoe repository. CityHall then provisions that workspace by
compiling aoe rather than downloading a release, which is what makes a change
that has landed on `main` but is not released yet testable. Experimental,
docker backend only, and slow.

The ref is resolved to the commit it points at when you save, and what gets
stored is `git-<sha>`. That sha, not the ref, is the version identity: it is the
image tag, the container's version label, and what the admin Workspaces page
shows, so it is always visible that a workspace is not on a release.
Consequences worth knowing:

- The ref is not tracked. Advancing `main` changes nothing on its own; save
  `git:main` again and the newly resolved sha recreates the container on its
  next start. Nothing rebuilds a workspace someone is using behind their back.
- Re-saving an already-resolved `git-<sha>` does not resolve or rebuild
  anything, so saving a form twice is free.
- Only public commits resolve. CityHall reads the ref through the GitHub API
  (with `GITHUB_TOKEN` when one is set) and the build clones over HTTPS with no
  credentials.

The build runs in the same background provisioning flow as an image pull, so it
survives a closed browser tab, runs once per image no matter how many requests
arrive, and reports progress on the admin Workspaces page. Expect roughly four
minutes on a 12 core, 4 GB Docker VM.

It compiles unoptimized (`opt-level=0`, no LTO) with two parallel rustc jobs,
which is what fits a stock 4 GB VM. Both are needed and for different reasons:
aoe's own `.cargo/config.toml` asks for eight parallel jobs, and separately the
`agent-of-empires` lib crate is one rustc process that no job count can shrink
and that gets OOM-killed at any optimization level above 0. The resulting binary
is slower than a release build, so this is for testing a change, not for a
deployment to settle on. Raise either knob if the builder has memory to spare:

```sh
SHA=$(git -C ../agent-of-empires rev-parse main)
docker build --build-arg AOE_SOURCE=git --build-arg AOE_GIT_SHA="$SHA" \
  --build-arg CARGO_BUILD_JOBS=8 --build-arg CARGO_PROFILE_DEV_RELEASE_OPT_LEVEL=1 \
  -t "cityhall/aoe:git-$SHA" deploy/aoe-image/
```

If a build fails with `cannot allocate memory`, or a log tail ending in
`signal: 9`, the builder ran out of memory rather than hitting a compile error.

The other backends do not build. The process backend refuses a `git-` version,
since a commit has no release tarball to download. The kubernetes backend can
run one, but only if you build the image with the command above and push it
where the cluster can pull it.

Members hold the `workspaces.use` permission by default and can open their own
workspace. `workspaces.read` / `workspaces.write` gate the admin Workspaces
page and its actions.

`workspaces.impersonate` (never implied by `workspaces.read`) lets an admin
open another user's workspace for support: the Open action mints a short-lived
access link, and exchanging it scopes that browser's workspace origin to the
target user for up to 30 minutes (every workspace tab, not just the new one).
Each grant and exit is written to the server log as an audit line. The
top-bar "Open workspace" link always exits admin access first; note that
WebSocket connections opened during access survive until they disconnect,
even past expiry or permission revocation. Requires `CITYHALL_SECRET_KEY`.
The target's idle accounting keeps running while an admin browses.

## Workspace configuration

A locked-down workspace cannot configure itself. In CityHall client mode aoe
closes `PATCH /api/settings`, the project CRUD routes, and `POST /api/git/clone`,
and the project registry starts empty, so a fresh workspace has no project for
the user to launch a session against. CityHall fills that gap with a **config
bundle**: one TOML document holding the aoe settings and the project list every
workspace should have.

Produce one from a configured aoe install (`aoe cityhall export --out
cityhall.toml`, or its dashboard under **Settings → CityHall**) and paste or
upload it under **Settings → Workspace config**. It looks like this:

```toml
schema_version = 1

[settings.acp]
default_agent = "claude-code"

[[projects]]
name = "cityhall"
remote = "https://github.com/agent-of-empires/cityhall.git"
default_base_branch = "main"
```

Projects carry a **git remote**, not a path: the admin's local checkout path
means nothing inside a container, so aoe clones each remote into the workspace's
data volume and registers it. Settings are a sparse patch, so only the fields
the admin actually changed are carried.

An aoe install is a convenience here, not a prerequisite. The page offers two
views of the same document, and switching between them carries unsaved edits:

- **Form** edits the project list and a curated set of settings with labels and
  descriptions, so no TOML has to be written. It is deliberately not every aoe
  setting: CityHall has no aoe process to ask for the schema, and the aoe version
  a workspace runs is per-user, so any field list it claimed to be complete could
  describe a version nobody runs. Editing here rewrites the document from its
  values, which drops comments; the page warns when there are any.
- **TOML** is the whole document. It is how an uploaded export gets in, how a
  field the form does not render gets set, and where comments survive. **Start
  from scratch** seeds a commented skeleton to fill in by hand.

Either way, `schema_version = 1` on its own is a valid document; it just
provisions nothing. Nothing the form does not render is dropped on save: unknown
sections, `[meta]`, and extra keys on a project are all preserved.

To deliver it, set `WORKSPACE_BUNDLE_ORIGIN` to the origin a *workspace* uses to
reach CityHall. That is not the public origin: on the docker backend it is the
compose service name on the shared network (`http://cityhall:3000`), on
kubernetes the in-cluster Service. There is no safe default, because CityHall's
own container hostname is not resolvable by its peers, so leaving it unset simply
turns config provisioning off and workspaces start unconfigured. CityHall then
passes each workspace `AOE_CITYHALL_BUNDLE_URL` and a per-workspace
`AOE_CITYHALL_BUNDLE_TOKEN`, and aoe fetches and applies the document at startup.

Applying is idempotent, because it happens on every start: an existing checkout
is left untouched so a user's uncommitted work survives a restart, and a repo
that fails to clone is reported without taking the other projects down. Editing
the bundle takes effect the next time a workspace starts; restart one from the
admin Workspaces page to apply it immediately. Nothing is restarted
automatically, because that would interrupt every user's in-flight agent turn at
once.

CityHall shape-checks a submitted bundle (valid TOML, a known `schema_version`,
`projects` an array, every project with a name and a remote) but deliberately
does **not** validate setting keys against aoe's schema: it does not have that
schema, and duplicating it would drift with every aoe release. aoe rejects an
unknown key when it applies the document, which surfaces as a workspace that will
not start.

A bundle carrying a `[git]` section is rejected outright. That section is
CityHall's to compose, per user, at the moment a workspace fetches its document:
an admin-supplied one would put one person's git identity and token into
everybody's bundle.

### Git credentials

The stored bundle never holds a secret. Each user sets their own git credential
under **Account**, and CityHall composes it into the document per user when a
workspace fetches it, along with that user's name and email so commits made
inside a workspace are attributed correctly. Tokens are encrypted with
`CITYHALL_SECRET_KEY`, are never returned to a client once stored, and are
removed with the user's account.

Per user rather than one shared deployment credential: attribution is correct at
the push level, and deleting an account revokes exactly that person's access. A
user who has not set one can still work with public repos; a private clone fails
with git's own error in the workspace.

| Variable | Default | Purpose |
| -------- | ------- | ------- |
| `WORKSPACE_BUNDLE_ORIGIN` | _(unset)_ | Origin a workspace uses to reach CityHall. Unset disables config provisioning. |

### Coding agents

**The reference image ships no coding agent.** It carries the aoe binary, git,
tmux, and a Node runtime, and nothing else. Which agent a workspace runs is the
user's choice, made per session in aoe's own session wizard, so CityHall does not
duplicate that as a setting.

A user installs the agent they want from a terminal session inside their
workspace. aoe prints the exact command when a chosen agent is missing, so
nothing here needs memorising:

```sh
npm install -g @agentclientprotocol/claude-agent-acp@latest   # Claude
npm install -g @agentclientprotocol/codex-acp@latest          # Codex
npm install -g @google/gemini-cli                             # Gemini
curl -fsSL https://opencode.ai/install | bash                 # OpenCode
```

**That install survives a restart.** Only the per-user volume persists, and it is
mounted at the aoe data dir, so every agent's own state directory would otherwise
be lost each time the container is recreated, which happens on a version change
or a credential change. The image entrypoint relocates them onto the volume with
symlinks: `~/.claude`, `~/.claude.json`, `~/.codex`, `~/.gemini`,
`~/.config/opencode`, `~/.local/share/opencode`, plus the three install prefixes
(`~/.npm-global`, `~/.local/bin`, `~/.opencode`). Symlinks rather than
`CODEX_HOME`-style variables because aoe clears the environment when it spawns a
structured-view agent, so an env-based approach would work in a terminal session
and quietly fail in structured view.

CityHall does not own agent versions. An install without a pinned version tracks
whatever the registry serves.

To set the default agent for every workspace, put it in the workspace config
document (**Settings → Workspaces**):

```toml
[settings.acp]
default_agent = "claude"
```

**An operator cannot force a workspace to use a particular agent.** aoe applies
no allowlist to the agents a session may pick, and a terminal session can run
whatever is installed regardless, so the set of installed binaries is the only
real lever. Build a derived image to fix that set:

```dockerfile
FROM cityhall/aoe:v0.5.0
RUN npm install -g @agentclientprotocol/claude-agent-acp@0.64.2
```

Point the image template at it, and users get exactly those agents. Having
CityHall install an admin-chosen set instead of a derived image is tracked in
[#57](https://github.com/agent-of-empires/cityhall/issues/57); the aoe-side
allowlist that would make a restriction actually hold is
[agent-of-empires#3241](https://github.com/agent-of-empires/agent-of-empires/issues/3241).

The `process` backend has no image and no entrypoint, so agents are installed on
the host by the operator; each user's `HOME` is already a persistent directory,
so logins persist there without any of the above.

### Agent credentials

Coding agents inside a workspace need their own provider credentials. A user
sets theirs under **Account**; an admin can also set them for someone else from
**Workspaces**, which is how a workspace can be handed over ready to use. They
are optional: a workspace with none configured starts exactly as before, and the
user can add theirs the first time they need one.

Only these variables can be stored, and nothing else:

| Variable | Agent | Reaches structured-view agents |
| -------- | ----- | ------------------------------ |
| `ANTHROPIC_API_KEY` | Claude, API billing | yes |
| `ANTHROPIC_AUTH_TOKEN` | Claude through a gateway | yes |
| `CLAUDE_CODE_OAUTH_TOKEN` | Claude, subscription login | yes |
| `OPENAI_API_KEY` | Codex | no |
| `GEMINI_API_KEY` | Gemini | no |
| `OPENROUTER_API_KEY` | OpenCode, OpenRouter | no |

The list is closed rather than free-form because these values become the
workspace's environment, so an arbitrary name would be a way to reconfigure the
workspace from a credential form.

**Subscriptions work too, by two different routes.** For a Claude Pro or Max
subscription, run `claude setup-token` anywhere you are already logged in and
store the result as `CLAUDE_CODE_OAUTH_TOKEN` above; an admin can set it on a
user's behalf like any other credential. Every other agent authenticates a
subscription through its own interactive login, run from a terminal session
inside the workspace:

```sh
codex login          # ChatGPT subscription
gemini              # Google account, prompts on first run
opencode auth login  # per-provider
```

Those write to the agent's own config directory, which the image entrypoint keeps
on the volume, so the login survives a restart and does not need repeating.
CityHall never sees these credentials, stores nothing, and cannot set them on a
user's behalf, which is the trade for not handling the secret at all.

**The last three reach terminal sessions but not structured-view agents.** aoe
starts a structured-view agent with a cleared environment and forwards a fixed
set of variables into it, which currently covers the Claude ones only. A key
outside that set is still available to anything you run in a terminal session.
The UI marks these, and widening the set is tracked in
[agent-of-empires#3238](https://github.com/agent-of-empires/agent-of-empires/issues/3238).

Values are encrypted with `CITYHALL_SECRET_KEY`, are never returned to a client
once stored, and are removed with the user's account. Changing that key leaves
stored credentials unreadable; the account page then shows them as needing to be
re-entered, and a workspace starts without them rather than failing.

**A change applies when the workspace is next created**, because credentials are
part of a container's environment rather than something injected into a running
one. Saving one deliberately does not disturb a running workspace, so a form
save cannot end a session mid-task. **Restart workspace** on the account page
applies pending changes; an idle stop or a first launch picks them up too.

Where the values are visible, stated plainly so a deployment can judge it:
CityHall passes them to the docker CLI through a mode-`0600` file that is
deleted as soon as the command returns, which keeps them out of argv, the host
process list, and CityHall's logs, but `docker inspect` on a running container
still shows them. On kubernetes they live in a per-user Secret referenced with
`envFrom`, so they stay out of the Deployment; note a Secret is not encrypted at
rest unless the cluster is configured for that. With the `process` backend they
are readable through `/proc/<pid>/environ` by the CityHall OS user, which is the
same user every workspace runs as. In every case, anyone who can administer the
runtime can read a workspace's credentials.

## The workspace proxy

Workspaces are served through a dedicated listener (default
`127.0.0.1:3001`), separate from the main CityHall origin, because the aoe
dashboard owns root-absolute paths. Every proxied request is authenticated
with the regular CityHall session cookie; the container itself runs
`aoe serve --auth=none --behind-proxy --allowed-host <proxy-host> --cityhall`
and is only reachable through a
loopback-published port, so CityHall is the sole auth boundary. In development
nothing needs configuring: the cookie set by `127.0.0.1:3000` is also sent to
`127.0.0.1:3001` (cookies ignore ports).

The allowed host is the host part of `WORKSPACE_PROXY_PUBLIC_ORIGIN` (defaulting
to `localhost`), because the proxy forwards the public `Host` it was called on
and aoe's DNS-rebinding gate refuses `--behind-proxy` without an allowlist
entry. Set that variable to the origin browsers actually use, or workspaces
answer 403 to every proxied request.

For production, expose the proxy listener through your reverse proxy as either
a subdomain or a second external port, and set `WORKSPACE_PROXY_PUBLIC_ORIGIN`
so the "Open workspace" link points at the public address. A subdomain needs
the session cookie to be visible there; today the cookie is host-only, so use
the same hostname with a second port, or terminate both origins on the same
host. WebSocket upgrade forwarding must be enabled on the external proxy.

| Variable | Default | Purpose |
| -------- | ------- | ------- |
| `WORKSPACE_PROXY_BIND_ADDR` | `127.0.0.1:3001` | Address of the workspace proxy listener. |
| `WORKSPACE_PROXY_PUBLIC_ORIGIN` | _(derived)_ | Public origin browsers use to reach the proxy. |

## Backends

`WORKSPACE_BACKEND` selects how workspaces run (see
[Configuration](configuration.md) for every variable):

- **`docker`** (default). One container + named volume per user. CityHall on
  the docker host dials loopback-published ports; with
  `WORKSPACE_DOCKER_NETWORK` set, workspaces instead join that docker network
  with no published ports and are dialed by container DNS, which is how
  CityHall itself runs in docker/compose (socket mounted, see
  `deploy/docker-compose.workspaces.yml`). Mounting the docker socket gives
  CityHall effective root on the host; use a restricted socket proxy if that
  matters.
- **`kubernetes`**. One Deployment + Service + PVC per user, managed with
  `kubectl` in the CityHall pod's namespace (override with
  `WORKSPACE_K8S_NAMESPACE`). Stop scales to zero keeping the PVC; destroy
  deletes all three. The image template must point at a registry the cluster
  can pull. Requires the RBAC and NetworkPolicy shipped in `deploy/k8s/` and
  the helm chart; without the NetworkPolicy any pod in the cluster can reach
  the auth-none workspaces.
- **`process`** (unix). One detached `aoe serve` per user with an isolated
  HOME under `WORKSPACE_PROCESS_DIR`, for VPS hosts without docker. Version
  binaries live at `$WORKSPACE_PROCESS_DIR/versions/<version>/aoe`,
  downloaded automatically from the release tarball (or installed there
  manually), so released versions only. Processes survive CityHall restarts.
  This isolates data, not security: every workspace runs as the CityHall OS
  user.

## Current limitations

- Agent credentials only reach structured-view agents for Claude. See
  [Agent credentials](#agent-credentials).
- An operator cannot restrict which agent a workspace runs, and cannot have
  CityHall install a chosen set; the image is the only lever
  ([#57](https://github.com/agent-of-empires/cityhall/issues/57)). See
  [Coding agents](#coding-agents).
- Git credentials are HTTPS tokens only; there is no way to supply an SSH key
  ([#52](https://github.com/agent-of-empires/cityhall/issues/52)).
- A workspace built from an unreleased commit does not follow the ref it came
  from, and only the docker backend can build one. See
  [Unreleased aoe](#unreleased-aoe).
