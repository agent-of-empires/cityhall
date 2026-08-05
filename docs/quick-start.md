# Quick Start

## Run the server

With no `DATABASE_URL` set, CityHall creates a local SQLite database and runs
migrations automatically:

```sh
cargo run
```

On first launch against an empty database, CityHall seeds an `admin` user with a
random password and logs it once:

```
WARN cityhall: seeded initial admin user | username: admin | password: <random> | change it on first login
```

Copy that password. It is not stored anywhere in plaintext and cannot be
recovered later (only reset). The server then listens on
`http://127.0.0.1:3000`.

> Building the frontend first (`cd web && npm install && npm run build`) lets the
> server serve the web UI. Without it the API still works; see
> [Development](development.md) for the live-reload dev workflow.

## Sign in

1. Open `http://127.0.0.1:3000`.
2. Sign in as `admin` with the seeded password.
3. You are required to set a new password before continuing.
4. You land on the **setup wizard**, which walks the seven things a fresh
   deployment usually needs.

## Finish the setup

The wizard covers, in order: the admin password, the aoe version workspaces run,
which coding agents a workspace arrives with, the projects it clones, email
(SMTP), sign-in and SSO, and inviting your team. Only the password and the aoe
version are required; a workspace cannot start without a version.

Every step saves through the same settings the UI exposes elsewhere, so nothing
is wizard-only. Use **Skip this step** for anything you do not want, and the
step stops being counted as outstanding.

Leaving early is fine. Whatever is unfinished waits on the **Dashboard** as a
"Finish setting up CityHall" checklist with a **Resume setup** button, and the
checklist disappears once every step is either configured or skipped. Each item
also links straight to the settings page that owns it, if you would rather not
use the wizard at all.

Members see a smaller version of this: their workspace page carries a
"before your first session" checklist (password, git credential, agent
credential) and a one-time guide, both driven by real state rather than a
tutorial flag.

## Manage users without the UI

The same binary manages users from the command line:

```sh
cargo run -- user list
cargo run -- user create --username bob --email bob@example.com
```

See the [CLI reference](cli.md) for all commands.

## Point it at a real database

For anything beyond local use, set `DATABASE_URL` to Postgres or MySQL:

```sh
DATABASE_URL=postgres://user:pass@localhost/cityhall cargo run
```

See [Configuration](configuration.md) for all options.
