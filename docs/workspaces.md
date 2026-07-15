# Workspaces

A workspace is one long-lived [aoe](https://github.com/agent-of-empires/agent-of-empires)
instance per user, spawned and managed by CityHall (docker containers today;
the orchestration seam is backend-agnostic). Each workspace has a persistent
data volume, so aoe sessions and configuration survive stops, restarts, and
version changes.

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

## Enabling workspaces

1. Build the workspace image for the version you want to serve (no official
   aoe server image is published yet):

   ```sh
   docker build --build-arg AOE_VERSION=v0.5.0 -t cityhall/aoe:v0.5.0 deploy/aoe-image/
   ```

2. In **Settings → Workspaces**, set the default version (e.g. `v0.5.0`) and
   enable workspaces. On a first startup the default version is pre-filled
   with the latest aoe release (skipped when offline; the field stays empty).

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
| `CONTAINER_CLI` | `docker` | Container CLI binary (e.g. `podman`). |

## Current limitations

- CityHall must run natively on the docker host (the backend dials
  loopback-published ports). CityHall-in-docker/compose/kubernetes topologies
  and non-docker backends are tracked in
  [#18](https://github.com/agent-of-empires/cityhall/issues/18).
- Workspace images are operator-built ([#17](https://github.com/agent-of-empires/cityhall/issues/17)
  tracks auto-pull/build).
- Agent credentials are not forwarded into workspaces yet
  ([#16](https://github.com/agent-of-empires/cityhall/issues/16)).
