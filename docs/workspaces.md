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
from `aoe serve`.

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
  `cityhall/aoe:v0.5.0`.

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
  manually). Processes survive CityHall restarts. This isolates data, not
  security: every workspace runs as the CityHall OS user.

## Current limitations

- Agent credentials are not forwarded into workspaces yet
  ([#16](https://github.com/agent-of-empires/cityhall/issues/16)).
