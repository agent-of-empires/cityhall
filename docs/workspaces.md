# Workspaces

A workspace is one long-lived [aoe](https://github.com/agent-of-empires/agent-of-empires)
instance per user, spawned and managed by CityHall (docker containers by
default; kubernetes and bare-process backends are available, see
[Backends](#backends)). Each workspace has a persistent data volume, so aoe
sessions and configuration survive stops, restarts, and version changes.

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

On a first startup the default version is pre-filled with the latest aoe
release (skipped when offline); adjust it under **Settings → Workspaces** if
needed. Starting a workspace with no default version set fails with a
descriptive error.

Members hold the `workspaces.use` permission by default and can open their own
workspace. `workspaces.read` / `workspaces.write` gate the admin Workspaces
page and its actions.

## The workspace proxy

Workspaces are served through a dedicated listener (default
`127.0.0.1:3001`), separate from the main CityHall origin, because the aoe
dashboard owns root-absolute paths. Every proxied request is authenticated
with the regular CityHall session cookie; the container itself runs
`aoe serve --auth=none --behind-proxy` and is only reachable through a
loopback-published port, so CityHall is the sole auth boundary. In development
nothing needs configuring: the cookie set by `127.0.0.1:3000` is also sent to
`127.0.0.1:3001` (cookies ignore ports).

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
