# CLI Reference

The `cityhall` binary runs the server, manages users, and rotates the encryption
of stored secrets. During development,
invoke it through Cargo (`cargo run -- <args>`); a release build exposes the same
interface as `cityhall <args>`.

Every command connects to the database (running migrations first), so
`DATABASE_URL` applies to the CLI exactly as it does to the server. See
[Configuration](configuration.md).

## Global options

| Option              | Env            | Description                                              |
| ------------------- | -------------- | -------------------------------------------------------- |
| `--log-level <lvl>` | `CITYHALL_LOG` | Log level for the app and dependencies (cascades).       |

## `cityhall serve`

Run the web server (API + frontend). This is the default when no subcommand is
given, so `cityhall` and `cityhall serve` are equivalent. Seeds the initial
`admin` user on an empty database.

```sh
cargo run                 # same as: cargo run -- serve
```

## `cityhall user`

Manage user accounts.

### `user list`

List all users (id, username, email, and whether a password change is pending).

```sh
cargo run -- user list
```

### `user create`

Create a user.

```sh
cargo run -- user create --username bob --email bob@example.com
cargo run -- user create --username svc --password 's3cret-value'
```

| Option              | Required | Description                                              |
| ------------------- | -------- | -------------------------------------------------------- |
| `--username <name>` | yes      | Unique username.                                         |
| `--email <email>`   | no       | Email address.                                           |
| `--password <pw>`   | no       | Password. Omit to generate a random one (printed once).  |
| `--role <name>`     | no       | Role to assign (defaults to `member`).                   |

When `--password` is omitted, a random password is generated and printed, and
the user must change it on first login.

### `user passwd`

Reset a user's password.

```sh
cargo run -- user passwd --username bob
cargo run -- user passwd --username bob --password 'new-value'
```

As with `create`, omitting `--password` generates and prints a random one that
the user must change on next login.

### `user delete`

Delete a user by username.

```sh
cargo run -- user delete --username bob
```

## `cityhall secrets`

Inspect and re-encrypt the secrets CityHall stores: the SMTP password, the OIDC
client secret, and every user's git and agent credentials. See
[Rotating the key](configuration.md#rotating-the-key) for the full procedure.

### `secrets status`

Report what `CITYHALL_SECRET_KEY` can read, per store.

```sh
cargo run -- secrets status
```

```
STORE                 ROWS  CURRENT  NEEDS-PREVIOUS  LEGACY  UNREADABLE
SMTP password            1        1               0       0           0
OIDC client secret       0        0               0       0           0
git credentials          4        3               1       0           0
agent credentials        7        7               0       0           0
```

| Column           | Meaning                                                                                       |
| ---------------- | --------------------------------------------------------------------------------------------- |
| `CURRENT`        | Bound to its row and readable with `CITYHALL_SECRET_KEY` alone. Nothing to do.                 |
| `NEEDS-PREVIOUS` | Readable only while `CITYHALL_SECRET_KEY_PREVIOUS` is set. Removing it now would lose these.    |
| `LEGACY`         | Written before values were bound to their row, so it could be moved between rows.              |
| `UNREADABLE`     | No configured key opens it. Either a former key is missing, or it must be entered again.       |

The command prints which of these to act on, so `NEEDS-PREVIOUS` above means
`secrets rotate` has not run yet and the old key cannot be dropped.

### `secrets rotate`

Re-encrypt every stored secret under the current `CITYHALL_SECRET_KEY`, reading
former keys from `CITYHALL_SECRET_KEY_PREVIOUS`.

```sh
cargo run -- secrets rotate
```

Safe to re-run: a secret already bound and already under the current key is left
untouched, so a second run writes nothing. A secret no configured key can read is
named and skipped rather than failing the run, and a secret something else wrote
mid-rotation is left alone and reported.

With no key change, this upgrades secrets written by an older CityHall, which is
what binds them to their row.
