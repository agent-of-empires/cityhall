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
| `CITYHALL_SECRET_KEY` | _(unset)_                  | Base64 32-byte key; encrypts secrets (SMTP password) at rest. |
| `CITYHALL_BASE_URL` | _(request host)_             | Public base URL used to build links in emails (e.g. password reset). |
| `SMTP_HOST`     | _(unset)_                        | SMTP host. Setting it makes SMTP env-managed (see below). |
| `SMTP_PORT`     | _(per encryption)_               | SMTP port; defaults to 25/587/465 for none/starttls/tls. |
| `SMTP_ENCRYPTION` | `starttls`                     | `none`, `starttls`, or `tls`.                       |
| `SMTP_USERNAME` | _(unset)_                        | SMTP auth username (optional).                      |
| `SMTP_PASSWORD` | _(unset)_                        | SMTP auth password (optional).                      |
| `SMTP_FROM_ADDRESS` | _(username)_                 | From address for outgoing mail.                     |
| `SMTP_FROM_NAME` | _(unset)_                       | Display name for the from address (optional).       |
| `OIDC_ISSUER`   | _(unset)_                        | OIDC issuer URL. Setting it makes SSO env-managed (see below). |
| `OIDC_CLIENT_ID` | _(unset)_                      | OIDC client id (required when `OIDC_ISSUER` is set). |
| `OIDC_CLIENT_SECRET` | _(unset)_                  | OIDC client secret (omit for public clients).       |
| `OIDC_SCOPES`   | `openid email profile`           | Space-separated scopes to request.                  |
| `OIDC_ALLOWED_DOMAINS` | _(any)_                   | Comma-separated email domains allowed to auto-provision. |
| `WORKSPACE_PROXY_BIND_ADDR` | `127.0.0.1:3001`     | Workspace proxy listener (see [Workspaces](workspaces.md)). |
| `WORKSPACE_PROXY_PUBLIC_ORIGIN` | _(derived)_      | Public origin of the workspace proxy behind a reverse proxy. |
| `CONTAINER_CLI` | `docker`                         | Container CLI used to manage workspaces (e.g. `podman`). |

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

## Email (SMTP)

CityHall can send email (for future flows such as password reset). SMTP is
configured in one of two ways, resolved at send time:

1. **Environment variables.** If `SMTP_HOST` is set, the whole SMTP
   configuration comes from the `SMTP_*` variables and the settings page is
   read-only. This is the recommended path for containerized deployments.
2. **Settings page.** If `SMTP_HOST` is unset, SMTP is configured in the web UI
   under **Settings**, and stored in the database.

Environment variables win as a block: it is env-managed or database-managed, not
a mix.

### Encryption

`SMTP_ENCRYPTION` (or the settings-page selector) chooses the transport
security, which also determines the default port:

- `none`: no transport security, port 25. Development only.
- `starttls`: upgrade a plaintext connection with STARTTLS, port 587.
- `tls`: implicit TLS from the first byte, port 465.

### Secret key

When a password is set through the settings page, it is encrypted at rest with
AES-256-GCM using `CITYHALL_SECRET_KEY` (a base64-encoded 32-byte key). Generate
one with:

```sh
openssl rand -base64 32
```

Without the key set, saving an SMTP password is rejected. Passwords supplied
through `SMTP_PASSWORD` are read straight from the environment and do not need
the key. Losing or changing the key makes a previously stored password
undecryptable; re-enter it in the settings page after rotating the key.

### Reset links

Password-reset and account-setup emails contain a link back to CityHall. Its
base URL is `CITYHALL_BASE_URL` when set (e.g. `https://cityhall.example.com`),
otherwise it is derived from the incoming request (honoring `X-Forwarded-Proto`
behind a reverse proxy). Set `CITYHALL_BASE_URL` explicitly for deployments
behind a proxy so links point at the public address.

## Single sign-on (OIDC)

CityHall supports single sign-on with any OpenID Connect provider (Google,
Microsoft/Entra, Okta, Auth0, Keycloak, GitLab, Authentik, and so on) through
one generic configuration. The flow is authorization code with PKCE.

Like SMTP, OIDC is configured either through `OIDC_*` environment variables
(env-managed, settings page read-only) or through the settings page (stored in
the database). Setting `OIDC_ISSUER` switches it to env-managed. The client
secret set through the settings page is encrypted at rest with
`CITYHALL_SECRET_KEY` (see [Secret key](#secret-key)).

### Redirect URI

Register `{base_url}/api/auth/oidc/callback` with your provider, where
`base_url` is `CITYHALL_BASE_URL` (or the request host). The settings page
shows the exact URL to register.

### Provisioning

On SSO login CityHall links the identity to a local account by the OIDC `sub`
claim, or by matching email to an existing account. Creating a **new** account
on first login is gated by the [self-signup](#self-signup) toggle: when signup
is off, SSO only logs in accounts that already exist (or that an admin created
with a matching email); when on, first-time SSO login provisions the account
with the signup default role. `OIDC_ALLOWED_DOMAINS` (or the settings field)
further restricts which email domains may auto-provision; empty allows any. SSO
accounts have no usable password until they set one through the reset flow.

## Self-signup

Public registration is **off by default**. An admin enables it under
**Settings**, where they also set an optional email-domain allow-list and the
role new accounts receive (defaults to `member`). There are no environment
variables for signup; it is entirely a settings-page toggle. Because the
password path emails a verification link, **SMTP must be configured before
signup can be enabled** (enabling it otherwise returns `400`).

When enabled, `POST /api/auth/register` creates an unverified account and emails
a verification link. The account cannot log in until the link is opened
(`POST /api/auth/verify-email`). This toggle is also the master switch for new
external accounts in general: it governs whether first-time SSO login may create
an account (see [Single sign-on](#single-sign-on-oidc)). Accounts created by an
admin or already linked are considered verified and are unaffected.
