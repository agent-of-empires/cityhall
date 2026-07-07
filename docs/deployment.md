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

- [Build the image](#build-the-image) once, then either
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
| `CITYHALL_SECRET_KEY` | Base64 32-byte key encrypting stored secrets (SMTP/OIDC). Generate with `openssl rand -base64 32`. Keep it stable, back it up. |
| `CITYHALL_BASE_URL` | Public URL (e.g. `https://cityhall.example.com`). Makes email links point at the real address instead of the internal host. |
| TLS | Session and OIDC flow cookies are `HttpOnly`; terminate HTTPS at a proxy and forward `X-Forwarded-Proto: https`. |

The initial `admin` password is printed **once** in the logs on first launch
against an empty database. Capture it (`docker compose logs cityhall`,
`kubectl logs`, `journalctl -u cityhall`) and change it on first login.

## Build the image

There is no published image yet, so build it from the repo `Dockerfile` (it
builds the frontend and backend and produces a ~small Debian-based runtime
image):

```sh
docker build -t cityhall:latest .
```

Push it to your own registry for multi-node or Kubernetes deployments:

```sh
docker tag cityhall:latest registry.example.com/cityhall:latest
docker push registry.example.com/cityhall:latest
```

The examples below use `cityhall:latest`; replace it with your registry path
where relevant.

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

## Kubernetes

[`deploy/k8s/`](../deploy/k8s) is a minimal Kustomize bundle: a namespace, a
Secret, a single-replica Postgres with a `PersistentVolumeClaim`, the CityHall
Deployment + Service, and an Ingress (nginx-ingress + cert-manager annotations
for TLS).

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

### Helm

Prefer Helm? The same resources ship as a chart in
[`deploy/helm/cityhall`](../deploy/helm/cityhall):

```sh
helm install cityhall deploy/helm/cityhall \
  --namespace cityhall --create-namespace \
  --set image.repository=registry.example.com/cityhall \
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
- `ingress.*` mirrors the Kustomize Ingress (className, host, cert-manager
  annotations, TLS secret).

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
- **The secret key is part of your backup story**: SMTP and OIDC secrets in the
  database are encrypted with `CITYHALL_SECRET_KEY`. Restoring a database without
  the matching key leaves those secrets undecryptable (re-enter them in
  Settings). Back up the key alongside the database, stored separately.

MySQL is also supported (`mysql://…`); the same notes apply.

See [Configuration](configuration.md) for every environment variable, and the
[API reference](api.md) for the endpoints.
