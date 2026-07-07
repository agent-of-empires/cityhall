# Configuration

CityHall is configured entirely through environment variables, so it fits a
Docker, Compose, or Kubernetes deployment without a config file.

| Variable        | Default                          | Purpose                                             |
| --------------- | -------------------------------- | --------------------------------------------------- |
| `DATABASE_URL`  | `sqlite://cityhall.db?mode=rwc`  | Database connection string (SQLite/Postgres/MySQL). |
| `BIND_ADDR`     | `127.0.0.1:3000`                 | Address the server listens on.                      |
| `STATIC_DIR`    | `web/dist`                       | Directory of the built frontend to serve.           |
| `CITYHALL_LOG`  | _(unset)_                        | Single log level for the app and its dependencies.  |
| `RUST_LOG`      | _(unset)_                        | Per-target log filter (overrides the default).      |

## Database

CityHall uses SeaORM and supports any of its relational backends. The backend is
chosen at runtime from the URL scheme; no rebuild is needed.

```sh
# SQLite (default) -- a file in the working directory, created on demand.
DATABASE_URL=sqlite://cityhall.db?mode=rwc

# Postgres
DATABASE_URL=postgres://user:pass@host:5432/cityhall

# MySQL
DATABASE_URL=mysql://user:pass@host:3306/cityhall
```

Migrations run automatically on startup and before every CLI command, so the
schema is always current. On an empty database, the initial `admin` user is
seeded (see [Quick start](quick-start.md)).

`mode=rwc` on the SQLite URL means "read-write, create if missing". In a
container, point it at a mounted volume, e.g.
`sqlite:///data/cityhall.db?mode=rwc`.

## Binding and static files

`BIND_ADDR` controls the listen address; use `0.0.0.0:3000` to accept
connections from outside the container. `STATIC_DIR` is where the server looks
for the built frontend (`index.html` plus assets); requests that do not match
`/api/*` fall back to `index.html` so client-side routes resolve on refresh.

## Logging

CityHall logs with [`tracing`](https://docs.rs/tracing). There are two ways to
control verbosity, in order of precedence:

1. **`CITYHALL_LOG` (or `--log-level`)** sets **one** level for the app and every
   dependency. Because it cascades, raising it also raises noisy sub-crates:

   ```sh
   CITYHALL_LOG=trace cargo run        # app AND sqlx queries at trace
   cargo run -- --log-level debug user list
   ```

   Accepts `error`, `warn`, `info`, `debug`, `trace`.

2. **`RUST_LOG`** gives full per-target control when you want the app verbose but
   a dependency quiet (standard [`EnvFilter`](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html)
   syntax):

   ```sh
   RUST_LOG=info,sqlx::query=debug cargo run
   ```

3. **Default** (neither set): `info,sqlx::query=warn`, which keeps SeaORM's
   per-query logging out of normal output.
