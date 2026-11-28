# Deployment

CityHall is a single self-contained binary that serves the API and the web UI on
one port (`3000` by default). Deploying it is therefore mostly about three
things: giving it a database, putting HTTPS in front of it, and setting a handful
of [environment variables](configuration.md).

Ready-to-copy example files live in [`deploy/`](../deploy):

```
deploy/
  docker-compose.prod.yml        Production Compose stack (CityHall + Postgres + Caddy)
  reverse-proxy/
    Caddyfile                    Caddy with automatic HTTPS
    nginx.conf                   nginx TLS termination
    docker-compose.traefik.yml   Traefik with automatic HTTPS
  systemd/
    cityhall.service             Run the binary on a bare VPS
  k8s/                           Kubernetes manifests (Kustomize)
    namespace.yaml
    secret.yaml
    postgres.yaml
    deployment.yaml
    service.yaml
    ingress.yaml
    kustomization.yaml
  helm/cityhall/                 Helm chart (same resources, templated)
```

Pick the section that matches how you run things:

- [Get the image](#the-image) once, then either
- [Docker Compose](#docker-compose-production) (simplest self-host), or
- [Kubernetes](#kubernetes), or
- [Bare VPS with systemd](#bare-vps-with-systemd).
- Everything terminates TLS through a [reverse proxy](#https-and-reverse-proxy).
- [Database](#database) covers Postgres in production.

## Before you start: the essentials

Whatever the target, CityHall needs:

| Requirement | Why |
| ----------- | --- |
| `DATABASE_URL` | A persistent database. SQLite on a volume works; Postgres is recommended for anything shared. See [Database](#database). |
| `BIND_ADDR=0.0.0.0:3000` | Listen on all interfaces so a proxy/orchestrator can reach it. The image sets this already. |
| `CITYHALL_SECRET_KEY` | Base64 32-byte key encrypting stored secrets (SMTP, OIDC, git and agent credentials). Generate with `openssl rand -base64 32`. Back it up; it can be changed, but only through the [rotation procedure](configuration.md#rotating-the-key). |
| `CITYHALL_BASE_URL` | Public URL (e.g. `https://cityhall.example.com`). Makes email links point at the real address instead of the internal host. |
| TLS | Session and OIDC flow cookies are `HttpOnly`; terminate HTTPS at a proxy and forward `X-Forwarded-Proto: https`. |

The initial `admin` password is printed **once** in the logs on first launch
against an empty database. Capture it (`docker compose logs cityhall`,
`kubectl logs`, `journalctl -u cityhall`) and change it on first login.

## The image

CI publishes two public images to ghcr.io on every push to `main` and on `v*`
tags:

- `ghcr.io/agent-of-empires/cityhall`, the app. Tags: `sha-<full commit sha>`,
  `main` (moves), and `X.Y.Z` on release tags. Both `linux/amd64` and
  `linux/arm64`.
- `ghcr.io/agent-of-empires/cityhall-aoe`, the reference
  [workspace image](workspaces.md#setup). One tag per aoe version, for example
  `v1.26.0`, which is what the default image template
  `ghcr.io/agent-of-empires/cityhall-aoe:{version}` resolves to. A
  `<version>-sha-<full commit sha>` tag pins the exact build.

Pin a `sha-` or version tag in deployments, never `main`:

```sh
docker pull ghcr.io/agent-of-empires/cityhall:sha-<full commit sha>
```

To build it yourself (for a fork, or to push to a private registry):

```sh
docker build -t cityhall:latest .
docker tag cityhall:latest registry.example.com/cityhall:latest
docker push registry.example.com/cityhall:latest
```

The Kubernetes and Helm examples below take the ghcr path above or your
registry path. The Compose stack builds from the checkout and tags the result
`cityhall:latest`; to run a published image there instead, remove the service's
`build:` block, set `image:` to the pinned ghcr tag, and start it without
`--build`.

Workspace images are built only for aoe versions CI has been set up for (the
`AOE_VERSION` in `images.yml`, or a manual run). On Kubernetes, which cannot
build images, set the default workspace version to one that has a tag.

## Docker Compose (production)

[`deploy/docker-compose.prod.yml`](../deploy/docker-compose.prod.yml) runs
CityHall behind [Caddy](https://caddyserver.com/) (automatic Let's Encrypt
HTTPS) with a Postgres database. Unlike the repo-root `docker-compose.yml` (a
local dev stack with Mailpit), this one is meant for a real host.

```sh
cd deploy
cp .env.example .env          # then edit: domain, secret key, DB password
docker compose -f docker-compose.prod.yml up -d --build
docker compose -f docker-compose.prod.yml logs cityhall   # grab the admin password
```

Point your domain's DNS `A`/`AAAA` record at the host first; Caddy needs it to
resolve to obtain a certificate. Set `CITYHALL_DOMAIN` and `ACME_EMAIL` in
`.env`. CityHall itself is not published on a host port; only Caddy's `80`/`443`
are exposed.

Data lives in named volumes (`pgdata`, `caddy_data`); they survive
`docker compose down`. Use `down -v` only when you intend to wipe everything.

To run per-user aoe workspaces from this stack, add the
[`deploy/docker-compose.workspaces.yml`](../deploy/docker-compose.workspaces.yml)
overlay (docker socket mount + shared workspace network) and expose the
workspace proxy as a second origin; see [Workspaces](workspaces.md#backends).

## Kubernetes

[`deploy/k8s/`](../deploy/k8s) is a minimal Kustomize bundle: a namespace, a
Secret, a single-replica Postgres with a `PersistentVolumeClaim`, the CityHall
Deployment + Service, and an Ingress (an nginx-ingress + cert-manager example;
adjust the class and annotations for another controller).

```sh
# 1. Edit deploy/k8s/secret.yaml (DB password, CITYHALL_SECRET_KEY) and
#    deploy/k8s/ingress.yaml (host + cert-manager issuer).
# 2. Point deployment.yaml image: at your pushed registry path.
kubectl apply -k deploy/k8s
kubectl -n cityhall logs deploy/cityhall   # grab the admin password
```

Notes:

- The Deployment sets `BIND_ADDR=0.0.0.0:3000` and mounts the Secret as env.
  Liveness/readiness probes hit `GET /api/health`.
- CityHall runs migrations on startup, so it is safe to roll out; keep it at
  **one replica** unless you move sessions off in-process state (sessions are
  stored in the database, so scaling horizontally works, but do it deliberately
  and behind a sticky-free load balancer).
- The bundled Postgres is a `StatefulSet`-lite (single Deployment + PVC) for
  simplicity. For production, prefer a managed Postgres or an operator (CNPG,
  Zalando) and just point `DATABASE_URL` at it; then delete `postgres.yaml`
  from the kustomization.
- Store `CITYHALL_SECRET_KEY` in a real secret manager (Sealed Secrets, SOPS,
  External Secrets), not in git.
- `db-networkpolicy.yaml` allows ingress to `cityhall-db:5432` only from the
  CityHall pod. It needs a CNI that enforces NetworkPolicy. Remove it together
  with `postgres.yaml`.
- **Team members get at most `view` in the namespace.** In the CityHall
  namespace, `secrets` get, list or watch; `pods/exec` and
  `pods/ephemeralcontainers`; and create, update or patch of `pods` or workload
  controllers such as `deployments` are each equivalent to every user's
  credentials, so treat them as cluster-admin verbs. The stock `edit` and
  `admin` ClusterRoles include all three. For a human role, start from
  [`example-human-role.yaml`](../deploy/k8s/example-human-role.yaml) (not applied
  by the kustomization) or `--set humanRole.enabled=true` in the chart, then
  bind it yourself.

### Helm

Prefer Helm? The same resources ship as a chart in
[`deploy/helm/cityhall`](../deploy/helm/cityhall):

```sh
helm install cityhall deploy/helm/cityhall \
  --namespace cityhall --create-namespace \
  --set image.tag=sha-<full commit sha> \
  --set config.baseUrl=https://cityhall.example.com \
  --set config.secretKey="$(openssl rand -base64 32)" \
  --set ingress.host=cityhall.example.com \
  --set postgres.password=a-strong-password
```

Key values (see [`values.yaml`](../deploy/helm/cityhall/values.yaml) for all):

- `postgres.enabled` (default `true`) deploys the bundled single-instance
  Postgres. Set it to `false` and provide `config.databaseUrl` (or
  `existingSecret`) to use an external/managed database.
- `existingSecret` points at a Secret you manage (with keys `DATABASE_URL`,
  `CITYHALL_SECRET_KEY`, `CITYHALL_BASE_URL`) instead of the chart creating one
  from `config.*` — the recommended path with a secret manager.
- `config.extraEnv` passes arbitrary env vars through (e.g. `SMTP_*`, `OIDC_*`)
  for env-managed [SMTP](configuration.md#email-smtp) or
  [SSO](configuration.md#single-sign-on-oidc).
- `postgres.enabled` also creates a NetworkPolicy that limits the database to
  the CityHall pod.
- `humanRole.enabled` (default `false`) renders the example team-member Role
  described above. The chart renders the bundled Postgres password and every
  `config.extraEnv` entry as a literal env value, so reading a Deployment shows
  them. To keep them from role holders, use an external database and leave
  `extraEnv` empty: put those variables (e.g. `SMTP_PASSWORD`) in the Secret named
  by `existingSecret`, which is loaded with `envFrom`.
- `ingress.*` mirrors the Kustomize Ingress (className, host, annotations, TLS
  secret). The defaults assume no controller (empty `className` uses the cluster
  default class, no annotations, `tls: false`); see
  [Ingress controllers](#ingress-controllers). `tls: true` requires
  `tlsSecretName`.
- `workspaces.proxy.ingress.*` exposes the workspace proxy (port 3001) on its
  own host, off by default. Set `enabled=true` and `host` (required), plus
  `tlsSecretName` when `tls` is on. The host also sets
  `WORKSPACE_PROXY_PUBLIC_ORIGIN`. Set a 3600 s idle timeout through the
  controller's annotations so idle WebSocket connections survive an hour (see
  the examples below); shorter timeouts work but drop idle connections sooner.
- `config.cookieDomain` sets `CITYHALL_COOKIE_DOMAIN`. Use a dedicated parent
  of both hosts so the session cookie reaches the proxy host. The API ignores
  this variable until #157 lands; until then the cookie stays host-only and
  workspace requests on the proxy host are rejected.

#### Ingress controllers

The chart does not assume a controller. Pass the example values that match
yours with `-f` (paths from the repository root):

- [`examples/values-nginx.yaml`](../deploy/helm/cityhall/examples/values-nginx.yaml):
  ingress-nginx and cert-manager, with 3600 s `proxy-read-timeout` and
  `proxy-send-timeout` on the proxy Ingress.
- [`examples/values-alb.yaml`](../deploy/helm/cityhall/examples/values-alb.yaml):
  the AWS Load Balancer Controller. One ALB serves both hosts through an
  IngressGroup (`alb.ingress.kubernetes.io/group.name`). TLS ends at the ALB
  with an ACM certificate (`certificate-arn`, which must cover both hosts), so
  both Ingresses keep `tls: false`. `scheme` is `internal` or `internet-facing`
  and must match on every Ingress in the group. `target-type: ip` registers pod
  IPs, because the Service is ClusterIP. `idle_timeout.timeout_seconds=3600`
  keeps WebSocket connections open; the ALB maximum is 4000 s, and the attribute
  applies to the whole ALB.

Notes for the ALB:

- Health checks. The app target group checks `/api/health` (200). The proxy
  port has no health path: every unauthenticated request gets 401. The example
  sets `success-codes: "401"` on the proxy Ingress, which only proves that the
  proxy answers HTTP, not that workspaces work.
- The ALB sends `X-Forwarded-Proto` to its targets with the client protocol
  ([AWS docs](https://docs.aws.amazon.com/elasticloadbalancing/latest/application/x-forwarded-headers.html)),
  so the proxy admin-access cookie gets the `Secure` flag behind it.
- Annotation names: see the
  [controller annotations](https://kubernetes-sigs.github.io/aws-load-balancer-controller/latest/guide/ingress/annotations/).

**Breaking change.** The chart defaults used to be `className: nginx`, the
cert-manager annotation, `tls: true` and (app Ingress) `tlsSecretName:
cityhall-tls`. Existing nginx users must add
`-f deploy/helm/cityhall/examples/values-nginx.yaml` (or set those values) when they upgrade; otherwise the
Ingress loses its class, annotations and TLS block.

Upgrades: `helm upgrade cityhall deploy/helm/cityhall …` with the same values;
CityHall runs its own migrations on startup.

## Bare VPS with systemd

No containers: build the binary, drop it on the host, run it under systemd
behind a reverse proxy.

```sh
# On a build host (needs Rust + Node), or build in CI and scp the artifacts:
cargo build --release            # target/release/cityhall
cd web && npm ci && npm run build   # web/dist

# On the VPS:
sudo useradd --system --home /opt/cityhall --shell /usr/sbin/nologin cityhall
sudo mkdir -p /opt/cityhall/web
sudo cp target/release/cityhall /opt/cityhall/
sudo cp -r web/dist /opt/cityhall/web/dist
sudo cp deploy/systemd/cityhall.service /etc/systemd/system/
sudo install -m600 -o cityhall -g cityhall /dev/null /etc/cityhall.env  # then edit it
```

Fill `/etc/cityhall.env` (referenced by the unit) with your `DATABASE_URL`,
`CITYHALL_SECRET_KEY`, `CITYHALL_BASE_URL`, and `BIND_ADDR=127.0.0.1:3000`
(bind to loopback since the proxy is on the same host). Then:

```sh
sudo systemctl daemon-reload
sudo systemctl enable --now cityhall
sudo journalctl -u cityhall -f       # grab the admin password on first start
```

The unit ([`deploy/systemd/cityhall.service`](../deploy/systemd/cityhall.service))
runs as the `cityhall` user with common hardening (`ProtectSystem`,
`NoNewPrivileges`, a private `/tmp`) and auto-restart. Put Caddy or nginx in
front (see below).

## HTTPS and reverse proxy

CityHall speaks plain HTTP; **always terminate TLS in front of it.** The proxy
must:

1. Forward to CityHall on `:3000`.
2. Set `X-Forwarded-Proto: https` so generated email/OIDC links use `https`
   (or just set `CITYHALL_BASE_URL`, which wins regardless).
3. Preserve the `Host` header.

WebSockets are not required today, but the examples pass upgrade headers anyway
so future features work without changes.

### Caddy (recommended: automatic HTTPS)

[`deploy/reverse-proxy/Caddyfile`](../deploy/reverse-proxy/Caddyfile):

```caddy
cityhall.example.com {
    reverse_proxy cityhall:3000
}
```

That is the whole config. Caddy obtains and renews a Let's Encrypt certificate
automatically and sets `X-Forwarded-*` headers for you. Replace the domain and,
when not running in Compose, `cityhall:3000` with the actual host:port.

### nginx

[`deploy/reverse-proxy/nginx.conf`](../deploy/reverse-proxy/nginx.conf) is a TLS
server block proxying to `127.0.0.1:3000`. Obtain the certificate with certbot
(`certbot --nginx -d cityhall.example.com`) or point `ssl_certificate` at your
own. It forwards `Host`, `X-Forwarded-Proto`, and `X-Forwarded-For`.

### Traefik

[`deploy/reverse-proxy/docker-compose.traefik.yml`](../deploy/reverse-proxy/docker-compose.traefik.yml)
runs Traefik as the edge with automatic HTTPS via labels on the CityHall
service. Use it instead of the Caddy stack if Traefik is already your ingress.

## Database

SQLite (the default) is fine for a single small instance if the file sits on a
persistent volume: `DATABASE_URL=sqlite:///data/cityhall.db?mode=rwc`. For
anything shared, backed up, or scaled, use Postgres:

```sh
DATABASE_URL=postgres://cityhall:password@db:5432/cityhall
```

- **Migrations** run automatically on every startup, so a fresh database is
  provisioned and upgrades apply themselves. No manual migration step.
- **Backups**: for Postgres, `pg_dump` on a schedule (or your provider's managed
  backups). For SQLite, snapshot the database file while the process is stopped,
  or use the SQLite online-backup approach.
- **The secret key is part of your backup story**: SMTP, OIDC, git, and agent
  secrets in the database are encrypted with `CITYHALL_SECRET_KEY`. Restoring a
  database without the matching key leaves those secrets undecryptable (re-enter
  them in Settings). Back up the key alongside the database, stored separately.
  `cityhall secrets status` reports how many secrets the configured key can read,
  which is worth running after a restore. Old keys can be kept readable through
  `CITYHALL_SECRET_KEY_PREVIOUS`, so a restore from before a rotation still works;
  see [Rotating the key](configuration.md#rotating-the-key).

MySQL is also supported (`mysql://…`); the same notes apply.

See [Configuration](configuration.md) for every environment variable, and the
[API reference](api.md) for the endpoints.
