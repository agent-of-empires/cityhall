# CityHall

CityHall is the distributed server for [Agent of Empires](https://github.com/agent-of-empires/agent-of-empires) (AoE).

Where AoE normally runs as an isolated local instance per person, CityHall is a
self-hostable backend that a team can point multiple AoE clients at, so accounts
and shared state live in one place instead of on each machine. It ships as a
single Rust binary that serves both the JSON API and the React web frontend, and
the same binary doubles as an administrative CLI.

## What it does today

This first release focuses on **user management**, the foundation the rest of the
control plane builds on:

- User accounts: create, list, edit, delete (API, web UI, and CLI).
- Cookie-based authentication with server-side sessions.
- A seeded `admin` account on first launch, with a random password that must be
  changed on first login.

## How it fits together

- **Backend** (`api/`): [axum](https://github.com/tokio-rs/axum) +
  [SeaORM](https://www.sea-ql.org/SeaORM/). Serves the API under `/api` and the
  built frontend for everything else.
- **Frontend** (`web/`): React + Vite + TypeScript + Tailwind, styled to match
  the AoE web dashboard.
- **Database**: any relational database SeaORM supports (SQLite, Postgres,
  MySQL), chosen at runtime so CityHall drops into a Docker, Compose, or
  Kubernetes stack without code changes.

## Next steps

- [Quick start](quick-start.md): run it and sign in.
- [Configuration](configuration.md): database, bind address, logging, and email.
- [Deployment](deployment.md): Docker, Compose, Kubernetes, VPS, HTTPS, and databases.
- [CLI reference](cli.md): manage users from the terminal.
- [API reference](api.md): the HTTP endpoints.
- [Development](development.md): build, test, and extend CityHall.
