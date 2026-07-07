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

## Errors

Errors use HTTP status codes with a JSON body:

```json
{ "error": "human-readable message" }
```

Common codes: `400` (bad request), `401` (unauthenticated), `403` (password
change required), `404` (not found), `409` (conflict, e.g. duplicate username).

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

Returns the current user:

```json
{ "id": 1, "username": "admin", "email": null, "must_change_password": false }
```

### `POST /api/auth/change-password`

```json
{ "current_password": "...", "new_password": "..." }
```

Verifies the current password and requires the new one to be at least 8
characters. Returns the updated user (with `must_change_password: false`).

### `GET /api/users`

Returns all users:

```json
[
  { "id": 1, "username": "admin", "email": null, "must_change_password": false, "created_at": "2026-07-07T09:20:50Z" }
]
```

Password hashes are never included in any response.

### `POST /api/users`

```json
{ "username": "bob", "email": "bob@example.com", "password": "..." }
```

`email` may be omitted or `null`. Returns the created user. A duplicate username
returns `409`.

### `GET /api/users/{id}`

Returns a single user, or `404` if not found.

### `PATCH /api/users/{id}`

Partial update; every field is optional:

```json
{ "username": "bob2", "email": "new@example.com", "password": "..." }
```

Renaming to an existing username returns `409`. Returns the updated user.

### `DELETE /api/users/{id}`

Deletes a user. Deleting your own account returns `400`. Returns:

```json
{ "deleted": true }
```

### `GET /api/settings/smtp`

Returns the effective SMTP configuration. The password itself is never
returned, only whether one is stored (`password_set`).

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

Updates the stored SMTP configuration:

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
