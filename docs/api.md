# API Reference

All endpoints are served under `/api`. Requests and responses are JSON.

## Authentication

CityHall uses **server-side sessions**. A successful `POST /api/auth/login` sets
an `HttpOnly` session cookie (`cityhall_session`); send it on subsequent
requests. Browsers do this automatically; from a client, use a cookie jar (e.g.
`curl -c jar -b jar`).

Every endpoint except `GET /api/health` and `POST /api/auth/login` requires a
valid session and returns `401 Unauthorized` without one.

### Forced password change

While a user's `must_change_password` flag is set, all user-management endpoints
return `403 Forbidden` with `{"error":"password change required"}`. Only
`GET /api/auth/me`, `POST /api/auth/change-password`, and `POST /api/auth/logout`
are permitted until the password is changed.

### Authorization (RBAC)

Every user has a role, and a role holds a set of permission keys (the wildcard
`*` grants all). Endpoints require a specific permission; a caller lacking it
gets `403 Forbidden` with `{"error":"insufficient permissions"}`. The current
keys are `users.read`, `users.write`, `roles.read`, `roles.write`,
`settings.read`, and `settings.write`. `GET /api/auth/me` returns the caller's
effective permission list so a client can gate its UI. Built-in roles are
`admin` (all permissions) and `member` (`users.read`).

## Errors

Errors use HTTP status codes with a JSON body:

```json
{ "error": "human-readable message" }
```

Common codes: `400` (bad request), `401` (unauthenticated), `403` (password
change required or insufficient permissions), `404` (not found), `409`
(conflict, e.g. duplicate username).

## Endpoints

### `GET /api/health`

Liveness check. Returns `200 OK` with the body `ok`. No authentication.

### `POST /api/auth/login`

```json
{ "username": "admin", "password": "..." }
```

On success, sets the session cookie and returns:

```json
{ "must_change_password": true }
```

Invalid credentials return `401`.

### `POST /api/auth/logout`

Clears the session (server-side and cookie). Returns `200`.

### `GET /api/auth/me`

Returns the current user, including their role and effective permissions:

```json
{
  "id": 1,
  "username": "admin",
  "email": null,
  "must_change_password": false,
  "role_id": 1,
  "role": "admin",
  "permissions": ["users.read", "users.write", "roles.read", "roles.write", "settings.read", "settings.write"]
}
```

### `POST /api/auth/change-password`

```json
{ "current_password": "...", "new_password": "..." }
```

Verifies the current password and requires the new one to be at least 8
characters. Returns the updated user (with `must_change_password: false`).

### `POST /api/auth/forgot-password`

Public (no authentication).

```json
{ "email": "user@example.com" }
```

Always returns `200` regardless of whether the email matches an account, so it
cannot be used to enumerate addresses. When it matches a user and SMTP is
configured, a single-use reset link (valid 1 hour) is emailed. Requires SMTP to
be configured (see [Configuration](configuration.md)); the link's base URL comes
from `CITYHALL_BASE_URL` or the request host.

### `POST /api/auth/reset-password`

Public (no authentication).

```json
{ "token": "...", "new_password": "..." }
```

Redeems a reset or setup token and sets the new password (minimum 8
characters), clearing `must_change_password`. The token is single-use; an
unknown, expired, or already-used token returns `400`.

### `GET /api/auth/providers`

Public (no authentication). Reports which login methods are available so the
login page can render accordingly:

```json
{ "oidc": true }
```

`oidc` is `true` when OIDC SSO is configured and enabled.

### `GET /api/auth/oidc/login`

Public. Begins the OIDC flow: sets a short-lived flow cookie (CSRF state, nonce,
PKCE verifier) and `303`-redirects the browser to the identity provider.
Returns `400` if SSO is not configured. Open it as a top-level navigation, not
via `fetch`.

### `GET /api/auth/oidc/callback`

Public. The provider's redirect target. Exchanges the code, verifies the ID
token, provisions or links the user (see [Configuration](configuration.md)),
starts a session, and `303`-redirects to `/`. On any failure it redirects to
`/login?error=...` so the SPA can surface the message.

### `GET /api/users`

Requires `users.read`. Returns all users:

```json
[
  { "id": 1, "username": "admin", "email": null, "must_change_password": false, "created_at": "2026-07-07T09:20:50Z", "role_id": 1 }
]
```

Password hashes are never included in any response.

### `POST /api/users`

Requires `users.write`.

```json
{ "username": "bob", "email": "bob@example.com", "password": "...", "send_setup_email": false, "role_id": 2 }
```

`email` may be omitted or `null`. `password`, `send_setup_email`, and `role_id`
are optional; `role_id` defaults to the `member` role. Behavior:

- `send_setup_email: true` emails the user a setup link (requires an email and
  configured SMTP, else `400`); the user sets their own password.
- `password` given: used as-is.
- `password` omitted or empty: a password is generated and the user must change
  it on first login.

Returns the created user plus `generated_password`, which is the generated
password when one was generated and `null` otherwise (including when a setup
email was sent):

```json
{ "id": 2, "username": "bob", "email": null, "must_change_password": true, "created_at": "...", "generated_password": "..." }
```

A duplicate username returns `409`.

### `GET /api/users/{id}`

Returns a single user, or `404` if not found.

### `PATCH /api/users/{id}`

Requires `users.write`. Partial update; every field is optional:

```json
{ "username": "bob2", "email": "new@example.com", "password": "...", "role_id": 3 }
```

Renaming to an existing username returns `409`; an unknown `role_id` returns
`400`. Returns the updated user.

### `DELETE /api/users/{id}`

Requires `users.write`. Deletes a user. Deleting your own account returns `400`.
Returns:

```json
{ "deleted": true }
```

### `GET /api/permissions`

Requires `roles.read`. Returns the permission-key catalog for building a role
editor:

```json
[
  { "key": "users.read", "description": "View users" },
  { "key": "users.write", "description": "Create, edit, and delete users" }
]
```

### `GET /api/roles`

Requires `roles.read`. Returns all roles. `permissions` is the array of keys
(`["*"]` for the wildcard); `user_count` is how many users hold the role.

```json
[
  {
    "id": 1,
    "name": "admin",
    "description": "Full access to everything",
    "permissions": ["*"],
    "is_system": true,
    "created_at": "2026-07-07T09:20:50Z",
    "user_count": 1
  }
]
```

### `POST /api/roles`

Requires `roles.write`.

```json
{ "name": "editor", "description": "Manage users", "permissions": ["users.read", "users.write"] }
```

`description` may be omitted or `null`. Unknown permission keys return `400`; a
duplicate name returns `409`. Returns the created role.

### `PATCH /api/roles/{id}`

Requires `roles.write`. Partial update (`name`, `description`, `permissions`).
The built-in `admin` role cannot be modified (`403`); a built-in role cannot be
renamed (`403`). Unknown permission keys return `400`. Returns the updated role.

### `DELETE /api/roles/{id}`

Requires `roles.write`. Deletes a role. Built-in roles cannot be deleted
(`403`), and a role still assigned to users cannot be deleted (`409`). Returns
`{ "deleted": true }`.

### `GET /api/settings/smtp`

Requires `settings.read`. Returns the effective SMTP configuration. The password
itself is never returned, only whether one is stored (`password_set`).

```json
{
  "env_managed": false,
  "enabled": true,
  "host": "smtp.example.com",
  "port": 587,
  "encryption": "starttls",
  "username": "cityhall",
  "from_address": "cityhall@example.com",
  "from_name": "CityHall",
  "password_set": true,
  "secret_key_available": true
}
```

`env_managed` is `true` when SMTP is configured through environment variables
(see [Configuration](configuration.md)); the values then come from the
environment and `PUT` is rejected. `secret_key_available` reflects whether
`CITYHALL_SECRET_KEY` is set, which is required to store a password.

### `PUT /api/settings/smtp`

Requires `settings.write`. Updates the stored SMTP configuration:

```json
{
  "host": "smtp.example.com",
  "port": 587,
  "encryption": "starttls",
  "username": "cityhall",
  "password": "...",
  "from_address": "cityhall@example.com",
  "from_name": "CityHall",
  "enabled": true
}
```

`encryption` is one of `none`, `starttls`, `tls`. `username`, `password`, and
`from_name` may be omitted or `null`. Omitting `password` keeps the stored one;
sending a value replaces it. Storing a password requires `CITYHALL_SECRET_KEY`
to be set, otherwise the request returns `400`. When SMTP is env-managed, this
returns `409`. Returns the updated settings (same shape as `GET`).

### `POST /api/settings/smtp/test`

Requires `settings.write`.

```json
{ "to": "you@example.com" }
```

Sends a test email using the effective configuration. Always returns `200` with
the result, so a delivery failure surfaces the provider's message rather than an
HTTP error:

```json
{ "ok": false, "error": "send failed: connection refused" }
```

Returns `400` if SMTP is not configured or not enabled.

### `GET /api/settings/oidc`

Requires `settings.read`. Returns the OIDC configuration. The client secret is
never returned, only whether one is stored (`client_secret_set`).

```json
{
  "env_managed": false,
  "enabled": true,
  "issuer": "https://accounts.example.com",
  "client_id": "cityhall",
  "scopes": "openid email profile",
  "allowed_domains": "example.com",
  "client_secret_set": true,
  "secret_key_available": true,
  "callback_path": "/api/auth/oidc/callback"
}
```

`env_managed` is `true` when OIDC is configured through environment variables;
the values then come from the environment and `PUT` is rejected. Register
`{base_url}{callback_path}` with the identity provider.

### `PUT /api/settings/oidc`

Requires `settings.write`. Updates the stored OIDC configuration:

```json
{
  "enabled": true,
  "issuer": "https://accounts.example.com",
  "client_id": "cityhall",
  "client_secret": "...",
  "scopes": "openid email profile",
  "allowed_domains": "example.com"
}
```

`issuer` and `client_id` are required. Omitting `client_secret` keeps the stored
one; sending a value replaces it (and requires `CITYHALL_SECRET_KEY`, else
`400`). `allowed_domains` is comma-separated (empty allows any domain). When
OIDC is env-managed, this returns `409`. Returns the updated settings.
