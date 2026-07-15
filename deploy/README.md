# Deployment examples

Copy-paste starting points for running CityHall in production. Full walkthrough:
[`docs/deployment.md`](../docs/deployment.md).

| Path | What it is |
| ---- | ---------- |
| `docker-compose.prod.yml` | CityHall + Postgres + Caddy (automatic HTTPS). The simplest self-host. |
| `docker-compose.workspaces.yml` | Overlay enabling per-user aoe workspaces for the Compose stack (socket mount + shared network). |
| `.env.example` | Variables for the Compose stack. Copy to `.env`. |
| `reverse-proxy/Caddyfile` | Caddy config (used by the Compose stack). |
| `reverse-proxy/nginx.conf` | nginx TLS termination for a single host. |
| `reverse-proxy/docker-compose.traefik.yml` | Traefik edge with automatic HTTPS via labels. |
| `systemd/cityhall.service` | Run the binary on a bare VPS under systemd. |
| `systemd/cityhall.env.example` | Config for the systemd unit (`/etc/cityhall.env`). |
| `k8s/` | Kustomize bundle: namespace, secret, Postgres, deployment, service, ingress, workspace RBAC + NetworkPolicy. |
| `helm/cityhall/` | Helm chart: the same resources, templated with a `values.yaml`. |

Quick starts:

```sh
# Docker Compose
cd deploy && cp .env.example .env   # edit it
docker compose -f docker-compose.prod.yml up -d --build

# Kubernetes (edit secret.yaml and ingress.yaml first)
kubectl apply -k deploy/k8s

# Kubernetes via Helm
helm install cityhall deploy/helm/cityhall \
  --namespace cityhall --create-namespace \
  --set config.secretKey="$(openssl rand -base64 32)" \
  --set config.baseUrl=https://cityhall.example.com \
  --set ingress.host=cityhall.example.com
```

Every path needs the same essentials: a `DATABASE_URL`, a stable
`CITYHALL_SECRET_KEY` (`openssl rand -base64 32`), `CITYHALL_BASE_URL`, and TLS
terminated by a reverse proxy. See [Configuration](../docs/configuration.md).
