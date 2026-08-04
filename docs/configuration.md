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
| `CITYHALL_SECRET_KEY` | _(unset)_                  | Base64 32-byte key; encrypts stored secrets at rest. |
| `CITYHALL_SECRET_KEY_PREVIOUS` | _(unset)_       | Comma-separated former keys, used to decrypt only (see [Rotating the key](#rotating-the-key)). |
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
| `WORKSPACE_BACKEND` | `docker`                     | Workspace backend: `docker`, `kubernetes`, or `process`. |
| `CONTAINER_CLI` | `docker`                         | Container CLI used by the docker backend (e.g. `podman`). |
| `WORKSPACE_DOCKER_NETWORK` | _(unset)_             | Docker network workspaces join (no published ports); for CityHall-in-compose. |
| `WORKSPACE_BUNDLE_ORIGIN` | _(unset)_              | Origin a *workspace* uses to reach CityHall, for fetching its config bundle (see [Workspaces](workspaces.md#workspace-configuration)). Unset means workspaces start unconfigured. |
| `WORKSPACE_TELEMETRY_POLICY` | _(the stored setting)_ | Pins aoe telemetry for every workspace: `user_choice`, `force_on`, or `force_off` (see [Workspaces](workspaces.md#telemetry)). Any other value fails startup. |
| `WORKSPACE_K8S_NAMESPACE` | _(pod namespace)_      | Namespace workspace objects are created in. |
| `WORKSPACE_K8S_VOLUME_SIZE` | `5Gi`                | PVC size per workspace. |
| `WORKSPACE_K8S_STORAGE_CLASS` | _(cluster default)_ | Storage class for workspace PVCs. |
| `WORKSPACE_PROCESS_DIR` | `/var/lib/cityhall/workspaces` | Data root of the process backend (per-user HOMEs, version binaries). |
| `GITHUB_TOKEN`  | _(unset)_                        | Authenticates aoe release discovery (higher GitHub rate limit). |

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

## Dashboard system metrics

The Dashboard's system card reports the machine as the CityHall process sees it.
When CityHall runs in a container that is usually the host's CPU and memory
rather than its own, so the card is labelled with what it is showing.

`SYSTEM_METRICS_SCOPE` overrides that label with `host` or `container`.
CityHall detects it from `/.dockerenv` and PID 1's cgroup, which covers docker,
compose, podman, containerd, and kubernetes; set it when the guess is wrong.

`SYSTEM_METRICS_DISK_PATH` names a filesystem to report, for example
`/var/lib/docker`. There is deliberately no default and the disk figure is
omitted when it is unset: under an overlay filesystem the obvious choice, `/`,
measures CityHall's own image layers rather than the storage workspaces use, so
a default would put a confidently wrong number on the page. The mount point
actually holding the path is shown beside it. Measuring a docker volume's
backing storage from inside a container requires mounting that host path into
CityHall.

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

Five kinds of secret are stored in the database: the SMTP password, the OIDC
client secret, each user's git credential and git SSH key, and each user's agent
credentials.
All of them are encrypted with AES-256-GCM using `CITYHALL_SECRET_KEY`, a
base64-encoded 32-byte key. Generate one with:

```sh
openssl rand -base64 32
```

Without the key set, saving any of them is rejected. Values supplied through the
environment instead (`SMTP_PASSWORD`, `OIDC_CLIENT_SECRET`) are read straight
from it and do not need the key.

Each value written with the authenticated envelope is bound to the row that holds
it, so a value copied to another row, or to another user, no longer decrypts.
Without that binding, anyone who could write to the database could move one user's
encrypted provider key into another user's row and CityHall would hand it over.
Values written by a CityHall older than the envelope are not bound until
`cityhall secrets rotate` rewrites them; see
[Upgrading secrets stored by an older CityHall](#upgrading-secrets-stored-by-an-older-cityhall).

Check what the current key can read at any time:

```sh
cityhall secrets status
```

### Rotating the key

Changing `CITYHALL_SECRET_KEY` does not re-encrypt anything by itself, so on its
own it makes every stored secret unreadable. `CITYHALL_SECRET_KEY_PREVIOUS` holds
former keys, used only to decrypt, which is what makes a change survivable.

1. **Back up the new key first.** Once secrets are re-encrypted under it, losing
   it destroys values the old key could still have recovered.
2. Set `CITYHALL_SECRET_KEY` to the new key and `CITYHALL_SECRET_KEY_PREVIOUS` to
   the old one, then restart **every** replica. This matters: a process still
   running with the old key keeps writing secrets that the next step will not
   find.
3. Re-encrypt everything under the new key:

   ```sh
   cityhall secrets rotate
   ```

4. Confirm it finished:

   ```sh
   cityhall secrets status
   ```

   Continue only when `NEEDS-PREVIOUS` and `LEGACY` are both zero. A non-zero
   `NEEDS-PREVIOUS` means a writer was missed in step 2; fix that and rotate
   again.

5. Remove `CITYHALL_SECRET_KEY_PREVIOUS` and restart.

`CITYHALL_SECRET_KEY_PREVIOUS` accepts several keys separated by commas, so more
than one old key can be kept readable at once.

A secret nothing in the ring can read is reported as `UNREADABLE` and named by
`cityhall secrets rotate`. Its plaintext is gone, so the only fix is to enter it
again.

### Upgrading secrets stored by an older CityHall

Secrets written before CityHall bound values to their rows are still readable, so
upgrading changes nothing on its own. They are **not** protected against being
moved between rows until they are rewritten, which the same command does with no
key change:

```sh
cityhall secrets rotate
```

CityHall warns at startup while any remain, and `cityhall secrets status` counts
them under `LEGACY`. Saving a secret through the UI also rewrites it, so the count
falls on its own over time.

One caveat: rotating an old value that had **already** been copied into the wrong
row binds it to where it now sits. The old format records no owner, so nothing can
tell where it came from. Rotation protects ownership from that point on; it cannot
establish it retroactively.

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
