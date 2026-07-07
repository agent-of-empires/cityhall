# Development

## Layout

```
api/    Rust backend (axum + SeaORM)
  src/
    entities/    SeaORM models (user, session)
    migration/   Embedded migrations
    handlers/    HTTP handlers (auth, users)
    auth.rs      Password hashing, sessions, the AuthUser extractor
    service.rs   User operations shared by the API and CLI
    server.rs    Router + static file serving
    cli.rs       clap CLI (serve, user ...)
    db.rs        Connection + migration runner
    seed.rs      First-launch admin seed
web/    React frontend (Vite + TypeScript + Tailwind)
  src/
    components/  Pages and UI (Login, ChangePassword, Users, dialogs)
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

## Database and migrations

CityHall targets SQLite, Postgres, and MySQL through SeaORM; the backend is
chosen at runtime from `DATABASE_URL`. Migrations live in `api/src/migration/`
and run automatically on startup and before every CLI command.

To add a schema change:

1. Add a `m0003_*.rs` module in `api/src/migration/` implementing
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
