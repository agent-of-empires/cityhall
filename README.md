# CityHall

CityHall is the distributed server for [Agent of Empires](https://github.com/agent-of-empires/agent-of-empires):
a self-hostable control-plane service that lets teams run and coordinate AoE
across many machines from one shared backend, instead of each person running an
isolated local instance.

It ships as a single Rust binary that serves both the API and the web frontend,
and doubles as a CLI. This first release covers **user management** (accounts,
authentication, forced first-login password change); more of the shared
control plane lands on top of it.

## Layout

```
api/    Rust backend (axum + SeaORM)
web/    React frontend (Vite + TypeScript + Tailwind)
```

## Database

Any relational database SeaORM supports, chosen via `DATABASE_URL`:

```sh
# SQLite (default when DATABASE_URL is unset) -- zero config, good for local dev
DATABASE_URL=sqlite://cityhall.db?mode=rwc

# Postgres / MySQL -- for docker, compose, or kube deployments
DATABASE_URL=postgres://user:pass@host/cityhall
DATABASE_URL=mysql://user:pass@host/cityhall
```

Migrations run automatically on startup and before any CLI command. On the
first launch against an empty database, an `admin` user is seeded with a
random password (logged once to stdout) that must be changed on first login.

## Running

```sh
cargo run              # runs migrations, seeds admin, serves API + frontend
```

Serves on `http://127.0.0.1:3000` (override with `BIND_ADDR`). The API lives
under `/api` (health check: `GET /api/health`); everything else serves the
frontend bundle from `STATIC_DIR` (default `web/dist`).

The build script builds `web/dist` automatically (needs Node.js/npm on
`PATH`), so a plain `cargo run` serves the frontend. Set
`SKIP_FRONTEND_BUILD=1` when the bundle is produced separately (docker
multi-stage build, CI artifact, backend-only iteration).

### CLI

```sh
cargo run -- serve                                   # same as no subcommand
cargo run -- user list
cargo run -- user create --username bob --email bob@example.com
cargo run -- user passwd  --username bob
cargo run -- user delete  --username bob
```

Omit `--password` on `create`/`passwd` to generate a random one (printed once).

## Docker Compose

A full local stack (CityHall + Postgres + [Mailpit](https://mailpit.axllent.org/)
for testing email) is provided:

```sh
docker compose up --build
```

CityHall serves on `http://localhost:3000` (admin password is printed in the
logs on first launch); Mailpit's UI is at `http://localhost:8025`. SMTP is
pre-configured to point at Mailpit, so sending email works out of the box and
messages land in the Mailpit UI.

## Frontend development

```sh
cd web
npm install
npm run dev            # Vite dev server, proxies /api to the backend on :3000
npm run build          # produces web/dist for the backend to serve
```

## Documentation

Full docs live in [`docs/`](docs/index.md):

- [Overview](docs/index.md)
- [Quick start](docs/quick-start.md)
- [Configuration](docs/configuration.md) (database, bind address, logging, email)
- [CLI reference](docs/cli.md)
- [API reference](docs/api.md)
- [Development](docs/development.md)

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). PR titles follow
[Conventional Commits](https://www.conventionalcommits.org/); a CI check enforces it.

Install the git hooks once with [pre-commit](https://pre-commit.com/):

```sh
pre-commit install
```

## License

MIT. See [LICENSE](LICENSE).
