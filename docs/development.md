# Development

## Layout

```
api/    Rust backend (axum + SeaORM)
  build.rs     Builds web/dist during `cargo build` (skip: SKIP_FRONTEND_BUILD=1)
  src/
    entities/    SeaORM models (user, session, smtp_settings)
    migration/   Embedded migrations
    handlers/    HTTP handlers (auth, users, settings)
    auth.rs      Password hashing, sessions, the AuthUser extractor
    crypto.rs    AES-256-GCM encryption for secrets at rest
    mailer.rs    SMTP config resolution (env vs database) and sending
    service.rs   User operations shared by the API and CLI
    server.rs    Router + static file serving
    cli.rs       clap CLI (serve, user ...)
    db.rs        Connection + migration runner
    seed.rs      First-launch admin seed
web/    React frontend (Vite + TypeScript + Tailwind)
  src/
    components/  Pages and UI (Login, ChangePassword, Users, Settings, dialogs)
    lib/api.ts   Typed API client
```

## Backend

```sh
cargo run                        # run migrations + serve
cargo test --workspace           # tests
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt                        # format (CI checks with --check)
```

Logs are controlled with `CITYHALL_LOG` / `RUST_LOG`; see
[Configuration](configuration.md).

### Frontend live reload

Run the backend and the Vite dev server side by side. Vite proxies `/api` to the
backend, so cookies and API calls work as in production:

```sh
# terminal 1
cargo run
# terminal 2
cd web
npm install
npm run dev
```

For a production-style run, build the frontend and let the backend serve it:

```sh
cd web && npm run build          # outputs web/dist
cargo run                        # serves web/dist via STATIC_DIR
```

Frontend checks:

```sh
cd web
npm run lint
npm run format:check
npm run build
```

## Docker Compose

A `docker-compose.yml` at the repo root brings up the full stack: CityHall
(built from the `Dockerfile`), Postgres, and [Mailpit](https://mailpit.axllent.org/)
as a local mail sink for testing email flows.

```sh
docker compose up --build
```

- CityHall: <http://localhost:3000> (admin password is printed in the
  `cityhall` logs on first launch).
- Mailpit web UI: <http://localhost:8025>.

By default SMTP is not env-managed, so configure it in **Settings** pointing at
Mailpit: host `mailpit`, port `1025`, encryption `none`. Sent mail appears in
the Mailpit UI. The compose file sets a dev `CITYHALL_SECRET_KEY`; replace it
for anything but local testing (`openssl rand -base64 32`).

## Database and migrations

CityHall targets SQLite, Postgres, and MySQL through SeaORM; the backend is
chosen at runtime from `DATABASE_URL`. Migrations live in `api/src/migration/`
and run automatically on startup and before every CLI command.

To add a schema change:

1. Add a `m0004_*.rs` module in `api/src/migration/` implementing
   `MigrationTrait`.
2. Register it in the `migrations()` list in `api/src/migration/mod.rs`.
3. Update or add the matching entity in `api/src/entities/`.

Keep migrations backend-agnostic (use the `sea_orm_migration::schema` helpers)
so they apply across all supported databases.

## Adding an endpoint

1. Add a handler in `api/src/handlers/`. Take `State<DatabaseConnection>` and, if
   it requires auth, the `AuthUser` extractor.
2. Reuse shared logic in `service.rs` where possible so the CLI and API stay in
   sync.
3. Wire the route into `api_router` in `api/src/server.rs`.
4. Add a matching method to `web/src/lib/api.ts` for the frontend.

## Conventions

See [CONTRIBUTING.md](../CONTRIBUTING.md). PR titles follow Conventional Commits,
and prose must not use em dashes.
