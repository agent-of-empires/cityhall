# CLI Reference

The `cityhall` binary runs the server and manages users. During development,
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
