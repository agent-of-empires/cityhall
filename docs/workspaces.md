# Workspaces

A workspace is one long-lived [aoe](https://github.com/agent-of-empires/agent-of-empires)
instance per user, spawned and managed by CityHall (docker containers by
default; kubernetes and bare-process backends are available, see
[Backends](#backends)). Each workspace has a persistent data volume, so aoe
sessions and configuration survive stops, restarts, and version changes.

A workspace runs in one of two modes, set by the owner's role (see
[GLOSSARY.md](../GLOSSARY.md)):

- **Full mode** serves aoe's complete dashboard: terminal, diff panes,
  worktrees and yolo. The workspace runs without `--cityhall`. A role gets it
  with the `workspaces.full` permission, which `admin` holds through `*`.
- **Composer mode** passes `--cityhall`, aoe's locked-down client mode: the
  message composer and the structured (chat) view only, with terminal and diff
  panes, project management and advanced settings hidden in the UI and refused
  server-side. Every role without `workspaces.full` gets this.

`--cityhall` needs an aoe version that supports it; starting a workspace on an
older version fails with an unknown-argument error from `aoe serve`. If no
release has it yet, see [Unreleased aoe](#unreleased-aoe).

The mode is read from the role when a workspace starts from scratch (first
launch, or after it was stopped) and on an explicit restart. A role change never
restarts a running workspace, and neither does a CityHall restart: a running
workspace keeps the mode it started with until it is stopped (an idle stop
included) or restarted with the restart action. Revoking `workspaces.full`
therefore leaves a running workspace in Full mode until then; restart it to
revoke at once.

**Upgrading.** A fresh install seeds the `member` role with `workspaces.full`.
Existing `member` roles are not changed and stay in Composer mode until an admin
adds the key. The upgrade recreates nothing, but roles that already hold the key
without being named (`admin` through `*`) switch their existing Composer
workspaces to Full mode on the next stop and start, or the next explicit
restart, on backends that allow Full mode (process, kubernetes). On docker,
Full mode is refused (see the table below) and these workspaces stay in
Composer mode.

The first time a user opens a Full mode workspace, aoe shows its optional
setup wizard at `/setup`. "Go to dashboard" dismisses it; the choice is kept on
the workspace volume, so it appears once per workspace.

### Granting Full mode gives a shell

Full mode puts aoe's own terminal behind `aoe serve --auth token`. On backends
that allow Full mode (process, kubernetes), granting `workspaces.full` therefore
gives that user a shell in their workspace, protected by CityHall's session and
the per-workspace bearer token. **A fresh install seeds the `member` role with the key, so
on those backends every new user gets Full mode by default.** On docker the
key has no effect today: Full mode is refused and every user gets Composer
mode. To withhold Full mode, remove `workspaces.full` from the `member` role on
the Roles page (or give users a role without it); the change applies on their
next stop and start or restart.

Composer mode has no shell: aoe refuses both shell routes under `--cityhall`,
even on a direct connection to the workspace. CityHall adds no exception, so a
user who needs a shell gets a role with `workspaces.full` and a restart.
CityHall has no shell of its own, only aoe's. Shell contents are not recorded.
CityHall gives a workspace no root or sudo path. System packages go in the team
base image; user-space installs (`~/.local/bin`, npm, cargo, pipx) need no root.
CityHall does not set the user a workspace runs as: the process backend uses the
CityHall OS user, and kubernetes uses the image's user, so a root-run process or
a root image gives the shell root.

If aoe is broken inside a kubernetes pod, there is no shell to open. Use
`kubectl exec` with your own cluster RBAC; CityHall has no `pods/exec`.

The shell is only as contained as the network and the host around the
workspace, and CityHall can verify none of it:

| Backend | Full mode |
| --- | --- |
| docker | **Refused.** Workspace containers share a bridge network (docker's default bridge, or `WORKSPACE_DOCKER_NETWORK`) that allows container to container traffic, and aoe listens on `0.0.0.0`. A peer container can reach the port but gets 401 without the workspace token (checked on docker's default bridge; `GET /` still serves aoe's static shell unauthenticated). Lifting the refusal is a follow-up. Publishing the port on `127.0.0.1` only keeps the host's network out. Users who hold the key get Composer mode, the log warns, and the workspaces pages show why. |
| kubernetes | Allowed. Relies on the shipped `cityhall-workspaces` NetworkPolicy, which only works on a CNI that enforces it, and is written for namespace `cityhall`; apply it to any other `WORKSPACE_K8S_NAMESPACE`. CityHall cannot detect whether it is enforced. Egress from a workspace pod is open: it reaches CityHall's API, `cityhall-db` and the kube API (rows 10 to 12 of [credential isolation](research/credential-isolation.md)). The shell can also read the pod's environment, including the bundle token and provider keys. |
| process | Allowed, **development only**. Every workspace runs as the CityHall OS user, so there is no isolation between users, or from CityHall itself. |

## How it works

- **Request-driven start.** Opening the workspace (the "Open workspace" link,
  or any request to the workspace proxy) starts the container if needed and
  resumes it if stopped. There is no manual start step for users.
- **Idle stop.** A workspace with no traffic for the configured idle window
  (default 30 minutes) is stopped automatically. Open WebSocket connections
  (live terminals) count as activity, so an open dashboard is never cut off.
  Stopping keeps the volume; the next request resumes with all data intact.
- **Destroy** (admin action) removes the container AND its volume. The volume
  is the user's whole home, so this permanently deletes their clones, installs,
  agent and subscription logins and unpushed work along with their aoe data.
- **Versions.** Admins set a default aoe version and can pin individual users
  (or a selected group) to a specific version. A version change recreates the
  container on its next start, keeping the volume. The image used is the
  settings' image template with `{version}` substituted, e.g.
  `cityhall/aoe:v0.5.0`. A version is normally a release tag, and can also be
  an unreleased commit ([Unreleased aoe](#unreleased-aoe)).

## Setup

Workspaces are always on and provision themselves. On the docker backend a
missing image is pulled from the registry the image template points at, and
when that fails (the default `cityhall/aoe:{version}` template is not a
published image) it is built locally from the reference Dockerfile; on the
process backend a missing binary is downloaded from the version's release
tarball. The first start of a new version therefore takes a few minutes; the
admin Workspaces page shows the progress, and requests get a retry-shortly
error until the artifact is ready. The kubernetes backend cannot be
auto-built: point the image template at a registry the cluster can pull.

Local builds need BuildKit, so whatever runs CityHall needs the `buildx` docker
CLI plugin. The published image ships it; a hand-rolled one that omits it fails
the build with a message naming the missing component.

Pre-building is still possible to skip the first-start wait, or to push to a
registry:

```sh
docker build --build-arg AOE_VERSION=v0.5.0 -t cityhall/aoe:v0.5.0 deploy/aoe-image/
```

The build checks the downloaded release against the `.sha256` published beside
it, so a damaged or substituted tarball fails the build rather than becoming the
binary your workspaces run. Both come from the same release, so this catches a
corrupted download and a partially replaced asset, not a release an attacker
controls outright. CityHall's own image pins its `docker`, `buildx`, and
`kubectl` digests in the `Dockerfile` itself, which does not have that limit.

Version fields offer the discovered stable aoe releases, fetched from the
GitHub API and cached for an hour (the last known list is served when GitHub
is unreachable; set `GITHUB_TOKEN` if the unauthenticated per-IP rate limit
is a problem). On a first startup the default version is pre-filled with the
latest release (skipped when offline); adjust it under **Settings →
Workspaces** if needed. Starting a workspace with no default version set
fails with a descriptive error.

### Unreleased aoe

A version does not have to be one of the discovered releases. Tick **custom
version** on any version field and type anything; it is substituted into the
image template exactly as a release tag is, so `main-20260804` with the default
template means `cityhall/aoe:main-20260804`. Nothing more happens: CityHall pulls
that image, or builds it from the reference Dockerfile's release path, and if
neither can produce it the workspace reports a provisioning failure naming the
image. Pointing at a different registry or repository is the image template's
job, under **Settings → Workspaces**, since that part is shared by everyone.

That is how you run an aoe with no release yet: build the image yourself and tag
it as the version you pin. The reference Dockerfile compiles a commit when told
to, so there is nothing to hand-assemble:

```sh
SHA=$(git -C ../agent-of-empires rev-parse main)
docker build --build-arg AOE_SOURCE=git --build-arg AOE_GIT_SHA="$SHA" \
  -t cityhall/aoe:main-20260804 deploy/aoe-image/
```

`AOE_GIT_SHA` has to be a full commit id. The build refuses a branch or a tag,
because either would let the same image tag mean a different build next week.

Then set the version to `main-20260804`, as the default or for one pinned user.
CityHall finds the image already present and runs it. Tag it however you like;
the tag is the version string and nothing parses it.

Notes on that build, all learned the hard way:

- **Give it memory.** It compiles unoptimized (`opt-level=0`, no LTO) with two
  parallel rustc jobs, which is what fits a 4 GB Docker VM with nothing else in
  it. Raise `CARGO_BUILD_JOBS` and `CARGO_PROFILE_DEV_RELEASE_OPT_LEVEL` on a
  bigger builder. A failure ending in `cannot allocate memory` or `signal: 9` is
  the builder running out of memory, not a compile error, and the build says so.
- **It needs BuildKit**, for the same reason the release path does.
- **Expect several minutes**, and a slower binary than a release build. This is
  for testing a change, not for a deployment to settle on.
- **Build for the architecture the workspace runs on.** The compile happens on
  the machine you run it on, so build on the docker host, or push a multi-arch
  image.
- **The process backend cannot use this.** It downloads a release tarball by tag
  name, so it only runs published versions. For kubernetes, push the image
  somewhere the cluster can pull it.

Members hold the `workspaces.use` permission by default and can open their own
workspace. `workspaces.read` / `workspaces.write` gate the admin Workspaces
page and its actions. `dashboard.read` gates the Dashboard page, and grants
more than workspace state: it also exposes CityHall's own CPU, memory, and
disk figures.

Two things about the Dashboard are not visible from the page itself. Its
numbers come from a snapshot refreshed every 10 seconds, not from a live read,
so they lag actions by up to that long; the page shows how old the sample is.
And per-workspace CPU and memory come from `docker stats`, so they are only
available on the docker backend. The kubernetes and process backends report
status without usage rather than reporting zeros.

`workspaces.impersonate` (never implied by `workspaces.read`) lets an admin
open another user's workspace for support: the Open action mints a short-lived
access link, and exchanging it scopes that browser's workspace origin to the
target user for up to 30 minutes (every workspace tab, not just the new one).
Each grant and exit is written to the server log as an audit line. Impersonation
is audited as entry only: what the admin does in the workspace, shell contents
included, is not recorded. Treat it as access to the user's credentials: in Full
mode the admin gets the user's terminal and can read login files such as
`~/.codex/auth.json` and the provider keys in the environment, and in either mode
can ask the user's agent to read them. The
top-bar "Open workspace" link always exits admin access first; note that
WebSocket connections opened during access survive until they disconnect,
even past expiry or permission revocation. Requires `CITYHALL_SECRET_KEY`.
The target's idle accounting keeps running while an admin browses.

## Workspace configuration

A locked-down workspace cannot configure itself. In CityHall client mode aoe
closes `PATCH /api/settings`, the project CRUD routes, and `POST /api/git/clone`,
and the project registry starts empty, so a fresh workspace has no project for
the user to launch a session against. CityHall fills that gap with a **config
bundle**: one TOML document holding the aoe settings and the project list every
workspace should have.

Produce one from a configured aoe install (`aoe cityhall export --out
cityhall.toml`, or its dashboard under **Settings → CityHall**) and paste or
upload it under **Settings → Workspace config**. It looks like this:

```toml
schema_version = 1

[settings.acp]
default_agent = "claude-code"

[[projects]]
name = "cityhall"
remote = "https://github.com/agent-of-empires/cityhall.git"
default_base_branch = "main"
```

Projects carry a **git remote**, not a path: the admin's local checkout path
means nothing inside a container, so aoe clones each remote into the workspace's
data volume and registers it. Settings are a sparse patch, so only the fields
the admin actually changed are carried.

An aoe install is a convenience here, not a prerequisite. The page offers two
views of the same document, and switching between them carries unsaved edits:

- **Form** edits the project list and a curated set of settings with labels and
  descriptions, so no TOML has to be written. It is deliberately not every aoe
  setting: CityHall has no aoe process to ask for the schema, and the aoe version
  a workspace runs is per-user, so any field list it claimed to be complete could
  describe a version nobody runs. Editing here rewrites the document from its
  values, which drops comments; the page warns when there are any.
- **TOML** is the whole document. It is how an uploaded export gets in, how a
  field the form does not render gets set, and where comments survive. **Start
  from scratch** seeds a commented skeleton to fill in by hand.

Either way, `schema_version = 1` on its own is a valid document; it just
provisions nothing. Nothing the form does not render is dropped on save: unknown
sections, `[meta]`, and extra keys on a project are all preserved.

To deliver it, set `WORKSPACE_BUNDLE_ORIGIN` to the origin a *workspace* uses to
reach CityHall. That is not the public origin: on the docker backend it is the
compose service name on the shared network (`http://cityhall:3000`), on
kubernetes the in-cluster Service. There is no safe default, because CityHall's
own container hostname is not resolvable by its peers, so leaving it unset simply
turns config provisioning off and workspaces start unconfigured. CityHall then
passes each workspace `AOE_CITYHALL_BUNDLE_URL` and a per-workspace
`AOE_CITYHALL_BUNDLE_TOKEN`, and aoe fetches and applies the document at startup.

Applying is idempotent, because it happens on every start: an existing checkout
is left untouched so a user's uncommitted work survives a restart, and a repo
that fails to clone is reported without taking the other projects down. Editing
the bundle takes effect the next time a workspace starts; restart one from the
admin Workspaces page to apply it immediately. Nothing is restarted
automatically, because that would interrupt every user's in-flight agent turn at
once.

CityHall shape-checks a submitted bundle (valid TOML, a known `schema_version`,
`projects` an array, every project with a name and a remote) but deliberately
does **not** validate setting keys against aoe's schema: it does not have that
schema, and duplicating it would drift with every aoe release. aoe rejects an
unknown key when it applies the document, which surfaces as a workspace that will
not start.

A bundle carrying a `[git]` section is rejected outright. That section is
CityHall's to compose, per user, at the moment a workspace fetches its document:
an admin-supplied one would put one person's git identity and token into
everybody's bundle.

### Git credentials

The stored bundle never holds a secret. Each user sets their own git credential
under **Account**, and CityHall composes it into the document per user when a
workspace fetches it, along with that user's name and email so commits made
inside a workspace are attributed correctly. Tokens are encrypted with
`CITYHALL_SECRET_KEY`, are never returned to a client once stored, and are
removed with the user's account.

Per user rather than one shared deployment credential: attribution is correct at
the push level, and deleting an account revokes exactly that person's access. A
user who has not set one can still work with public repos; a private clone fails
with git's own error in the workspace.

#### SSH keys

A token cannot authenticate a `git@host:...` remote, so **Account** also takes an
SSH private key, stored and served the same way. Two rules the form enforces
rather than leaving to fail inside the container:

- **The key must have no passphrase.** Nothing in a workspace can prompt for one,
  so a protected key would turn every clone into a hang. A key kept for this
  purpose only is the answer, not the key on your laptop.
- **Host keys are required with it.** The workspace connects with strict host key
  checking, so it refuses anything not listed in the known hosts field rather
  than trusting whatever answers. Shipping a key without them would trade a
  credential problem for a machine-in-the-middle one.

  For GitHub, **Fill from GitHub** fetches the keys GitHub publishes, over a
  connection authenticated for a name an attacker on the path cannot present. For
  another host, run `ssh-keyscan <host>` somewhere you trust the network and check
  the result against the fingerprints that host publishes. `ssh-keyscan` on its
  own trusts whatever answers on port 22, so pasting a scan unchecked pins
  whatever was listening at that moment, and strict checking then cannot tell the
  difference.

Host keys are public, so unlike the key itself they are shown back to the user
and can be edited without re-entering it.

Installing the key is aoe's half of the job, from the same `[git]` table it
already reads the token out of. An aoe too old to know about the two keys ignores
them, so nothing breaks on an older workspace; `git@` remotes simply keep failing
until it is upgraded.

| Variable | Default | Purpose |
| -------- | ------- | ------- |
| `WORKSPACE_BUNDLE_ORIGIN` | _(unset)_ | Origin a workspace uses to reach CityHall. Unset disables config provisioning. |

### Coding agents

**The reference image ships no coding agent.** It carries the aoe binary, git,
tmux, and a Node runtime, and nothing else. Which agent a workspace runs is the
user's choice, made per session in aoe's own session wizard.

A user installs the agent they want from the workspace terminal, which only
Full mode has. aoe prints the exact command when a chosen agent is missing, so
nothing here needs memorising:

```sh
npm install -g @agentclientprotocol/claude-agent-acp@latest   # Claude
npm install -g @agentclientprotocol/codex-acp@latest          # Codex
npm install -g @google/gemini-cli                             # Gemini
curl -fsSL https://opencode.ai/install | bash                 # OpenCode
```

**That install survives a restart.** The per-user volume is the whole `$HOME`,
so an installed agent and its login both persist across a container recreate.
See [The home volume](#the-home-volume).

#### Handing users a workspace that is already set up

Tick the agents under **Settings → Workspaces** and a workspace installs them
itself, so a user opens one that is ready to use instead of installing something
first. Leave them all unticked and nothing is installed, which is the behaviour
above.

Four things are worth knowing before turning it on.

**The install happens at boot, in the background.** CityHall gives a workspace
15 seconds to answer HTTP before it treats the start as failed, and an
`npm install -g` takes minutes, so the workspace comes up first and the agents
land shortly after. A user who opens a brand new workspace within the first
minute or so may still find an agent missing; it appears without them doing
anything. Progress and failures are logged inside the workspace, at
`~/.config/agent-of-empires/cityhall-agents/provision.log`.

**It adds a network dependency to a cold start.** A registry outage means the
workspace starts without the agent it was supposed to have rather than failing
to start. The cost is paid once per user, because the install lands on the
persistent volume, so later starts install nothing. An operator who wants a
fixed set with no boot-time network at all builds a derived image instead; see
below.

**A change reaches an existing workspace when that workspace is next created**,
which is a restart, an idle stop, or a first launch, exactly like a credential
change. Saving the setting deliberately does not disturb anyone's running
session. Versions are not pinned: an install tracks whatever the registry serves,
the same as a user running the command by hand.

**An agent a user removes stays removed.** CityHall records that it installed a
default once, and does not reinstall it on every start. Unticking an agent does
not uninstall anything either; it only stops new workspaces from getting it.

To set which agent a new session picks by default, put it in the workspace config
document (**Settings → Workspace config**):

```toml
[settings.acp]
default_agent = "claude"
```

**The installed set is a default, not a restriction.** aoe applies no allowlist
to the agents a session may pick, and a Full mode shell can run whatever is on
`PATH` regardless, so an admin choosing the set decides what a workspace *arrives
with*, not what it is *limited to*. A user can install and run anything else.
Making a restriction actually hold needs an aoe-side allowlist, tracked in
[agent-of-empires#3241](https://github.com/agent-of-empires/agent-of-empires/issues/3241),
and separately a decision about terminal access, since a shell defeats an
allowlist that only covers the structured view.

For a fixed set with no boot-time install at all, build a derived image:

```dockerfile
FROM cityhall/aoe:v0.5.0
RUN npm install -g @agentclientprotocol/claude-agent-acp@0.64.2
```

Point the image template at it and every workspace starts with exactly that,
pinned, with nothing fetched at boot. The install prefixes come first on `PATH`,
so a user can still upgrade a baked-in agent for themselves. This is the right
choice for a deployment that wants reproducible workspaces; the setting above is
the right choice for convenience.

The `process` backend has no image and no entrypoint, so **the agent setting does
nothing there** and agents are installed on the host by the operator; each user's
`HOME` is already a persistent directory, so logins persist there without any of
the above.

### Agent credentials

Coding agents inside a workspace need their own provider credentials. A user
sets theirs under **Account**; an admin can also set them for someone else from
**Workspaces**, which is how a workspace can be handed over ready to use. They
are optional: a workspace with none configured starts exactly as before, and the
user can add theirs the first time they need one.

Only these variables can be stored, and nothing else:

| Variable | Agent |
| -------- | ----- |
| `ANTHROPIC_API_KEY` | Claude, API billing |
| `ANTHROPIC_AUTH_TOKEN` | Claude through a gateway |
| `CLAUDE_CODE_OAUTH_TOKEN` | Claude, subscription login |
| `OPENAI_API_KEY` | Codex |
| `GEMINI_API_KEY` | Gemini |
| `OPENROUTER_API_KEY` | OpenCode, OpenRouter |

aoe v1.15.0 and later forwards each of these into the structured-view agent it
belongs to.

The list is closed rather than free-form because these values become the
workspace's environment, so an arbitrary name would be a way to reconfigure the
workspace from a credential form.

**Subscriptions depend on the mode.** CityHall brokers no subscription login. A
login happens inside the workspace, or not at all:

| Agent | Full mode: run in the workspace terminal (not on docker, which refuses Full mode) | Composer mode (no terminal) |
| ----- | ------------------------------------------------------------------------------ | --------------------------- |
| Claude | `claude auth login` | `claude setup-token` run anywhere you are logged in, stored as `CLAUDE_CODE_OAUTH_TOKEN` |
| Codex | `codex login --device-auth` | API key |
| Gemini | `gemini` | API key |
| OpenCode | `opencode auth login` | API key |

A Composer mode user who wants a Codex, Gemini or OpenCode subscription needs a
role with `workspaces.full`. An admin can set `CLAUDE_CODE_OAUTH_TOKEN` on a
user's behalf like any other credential.

The terminal logins write to `~/.claude`, `~/.codex`, `~/.gemini` and OpenCode's
directories, which are in the home directory on the volume. They survive an idle
stop, a restart and an aoe version change, and a destroy deletes them. CityHall
never holds these credentials and cannot show whether one exists.

**In Full mode, prefer `claude auth login` over a stored
`CLAUDE_CODE_OAUTH_TOKEN`.** Anthropic's terms allow a user to sign in to the
unmodified binary with their own subscription. They do not allow a third party to
"collect, store or intermediate" claude.ai credentials. Whether CityHall storing a
`setup-token` counts is our reading, not Anthropic's wording, so check the terms
for your deployment. The stored token sits where an API key sits, and CityHall
cannot rotate or revoke it: the user revokes it at Anthropic.

Values are encrypted with `CITYHALL_SECRET_KEY` and bound to the user and variable
they were stored for, so a value moved to another user's row does not decrypt for
them. They are never returned to a client once stored, and are removed with the
user's account. Changing the key without following the
[rotation procedure](configuration.md#rotating-the-key) leaves stored credentials
unreadable; the account page then shows them as needing to be re-entered, and a
workspace starts without them rather than failing.

**A change applies when the workspace is next created**, because credentials are
part of a container's environment rather than something injected into a running
one. Saving one deliberately does not disturb a running workspace, so a form
save cannot end a session mid-task. **Restart workspace** on the account page
applies pending changes; an idle stop or a first launch picks them up too.

Where the values are visible, stated plainly so a deployment can judge it:
CityHall passes them to the docker CLI through a mode-`0600` file that is
deleted as soon as the command returns, which keeps them out of argv, the host
process list, and CityHall's logs, but `docker inspect` on a running container
still shows them. On kubernetes they live in a per-user Secret referenced with
`envFrom`, so they stay out of the Deployment; note a Secret is not encrypted at
rest unless the cluster is configured for that. **That split only buys anything if
your namespace RBAC keeps it**: reading a Deployment must not imply reading
Secrets, or the values are back in reach of everyone who could see them before.
Grant `secrets` read separately and to fewer principals than `deployments` read,
and remember a per-user Secret is enough to fetch that user's whole config bundle,
including their decrypted git token. With the `process` backend they
are readable through `/proc/<pid>/environ` by the CityHall OS user, which is the
same user every workspace runs as. In every case, anyone who can administer the
runtime can read a workspace's credentials.

## Telemetry

aoe's usage telemetry is opt-in, and aoe asks each user for that opt-in inside
their own workspace. In a CityHall deployment that is the wrong person to ask, so
**Settings → Workspaces → aoe telemetry** decides it for everyone: off for
everyone, on for everyone, or let each user choose (the default, and how it
behaved before this setting existed).

`WORKSPACE_TELEMETRY_POLICY` (`user_choice`, `force_on`, `force_off`) pins the
policy for the whole deployment. While it is set it wins, and a choice saved on
the page is stored for when the variable is removed rather than applied.

What each state does to a workspace:

- **Off for everyone** sets `DO_NOT_TRACK=1`, which aoe treats as absolute:
  nothing is sent, no install id is generated, and no consent prompt appears.
  Works with any workspace image.
- **On for everyone** runs `aoe telemetry enable` in the workspace before the
  server starts, which is what also records the consent as answered, so no prompt
  appears. On the container backends it needs an image with a shell and an aoe new
  enough to have that subcommand; without either, the workspace fails to start
  rather than coming up with the policy unapplied. The reason is in that
  workspace's own backend log: `docker logs cityhall-workspace-u<id>` on the
  docker backend, `kubectl logs deploy/cityhall-workspace-u<id>` on kubernetes,
  and `$WORKSPACE_PROCESS_DIR/u<id>/serve.log` on the process backend (which runs
  the command directly, needing no shell, and reports the failure through the
  API instead).
- **Let each user choose** injects nothing. aoe's own prompt reaches the user and
  CityHall never marks the consent as answered on their behalf.

A change reaches a workspace the next time it starts, which for a stopped one is
automatic. Tick **restart running workspaces on save** to apply it to running
ones immediately; that recreates their containers, ending whatever their users
are running, which is why it is not the default.

Two things worth knowing before forcing telemetry on. Suppressing the consent
prompt makes disclosing the collection to your users your deployment's
responsibility, not aoe's. And the opt-in is recorded in each user's data volume,
so relaxing the policy back to "let each user choose" stops CityHall enforcing
anything but leaves those users opted in until they turn it off themselves;
CityHall does not rewrite aoe's stored consent to undo it.

## The workspace proxy

Workspaces are served through a dedicated listener (default
`127.0.0.1:3001`), separate from the main CityHall origin, because the aoe
dashboard owns root-absolute paths. Every proxied request is authenticated
with the regular CityHall session cookie; the workspace itself runs
`aoe serve --auth token --behind-proxy --allowed-host <proxy-host> [--cityhall]
--dangerously-allow-plaintext-remote` (`--cityhall` only in Composer mode, the
last flag only on aoe 1.22.0 and later, see below). CityHall mints a
per-workspace bearer token (stored encrypted, so `CITYHALL_SECRET_KEY` is
required) and the proxy injects it as `Authorization: Bearer`, so the bearer
keeps a workspace safe from its peers, not from its owner, who can read their
own `serve.token`. The proxy drops every `Set-Cookie` from the workspace, not
only `aoe_token`. What else can reach the workspace's port still depends on the
backend, see
[Granting Full mode gives a shell](#granting-full-mode-gives-a-shell). In
development nothing needs configuring:
the cookie set by `127.0.0.1:3000` is also sent to `127.0.0.1:3001` (cookies
ignore ports).

aoe 1.22.0 added a plaintext gate that answers `426 tls_required` to every
request it did not receive over HTTPS, and CityHall's proxy talks plain HTTP to
the workspace. CityHall therefore adds `--dangerously-allow-plaintext-remote`
when the workspace version is 1.22.0 or later; aoe before 1.22.0 rejects the
flag, so it is omitted there.

A version that is not a release tag with a `major.minor` number (`latest`,
`main`, `nightly-20261127`, a git SHA) gets the flag. That side was chosen
because it fails loudly: if the binary is older than 1.22.0, aoe exits at
start with an unknown-argument error. The other side fails silently: a newer
aoe answers 426 on every request while the workspace looks healthy.

The decision keys on the configured version string, not on the binary. For
docker and kubernetes the version must therefore match the aoe inside the
image: an `image_template` without `{version}` (for example a `:stable` tag),
or a custom tag over an older aoe, can make the decision wrong. Keep the
version and the image in step.

The plaintext hop is accepted for now because it stays inside the docker or
pod network (or on loopback, for the process backend) between the proxy and
the workspace; a TLS sidecar is the planned replacement (issue #127).

The allowed host is the host part of `WORKSPACE_PROXY_PUBLIC_ORIGIN` (defaulting
to `localhost`), because the proxy forwards the public `Host` it was called on
and aoe's DNS-rebinding gate refuses `--behind-proxy` without an allowlist
entry. Set that variable to the origin browsers actually use, or workspaces
answer 403 to every proxied request.

For production, expose the proxy listener on one extra hostname, a sibling of
the CityHall host such as `ws.cityhall.<domain>`, and set
`WORKSPACE_PROXY_PUBLIC_ORIGIN` so the "Open workspace" link points at it. That
one origin serves every workspace: the proxy routes by session cookie, not by
`Host`. Set `CITYHALL_COOKIE_DOMAIN` to a dedicated parent such as
`cityhall.<domain>` so the session cookie reaches the proxy host; empty, its
default, keeps the cookie host-only. TLS ends at the ingress, which talks plain
HTTP to the proxy port. The proxy ingress needs `proxy-read-timeout` and
`proxy-send-timeout` of 3600 seconds for WebSocket.

| Variable | Default | Purpose |
| -------- | ------- | ------- |
| `WORKSPACE_PROXY_BIND_ADDR` | `127.0.0.1:3001` | Address of the workspace proxy listener. |
| `WORKSPACE_PROXY_PUBLIC_ORIGIN` | _(derived)_ | Public origin browsers use to reach the proxy. |

### Workspace bearer token

- Delivery relies on an undocumented aoe contract: CityHall writes
  `$HOME/.config/agent-of-empires/serve.token` (0600) before aoe starts and aoe
  reads it (aoe issue 4953 tracks an explicit input). It needs aoe 1.18 or
  later (tags like `latest` pass the check); below that the workspace refuses
  to start, with no fallback to `--auth none`.
- Upgrade: every running workspace is recreated once on its first start after
  upgrading (docker and kubernetes recreate, process respawns). The volume is
  kept, but the recreate ends that workspace's live sessions. aoe below 1.18 is
  refused.
- The token is visible in `docker inspect`, the Kubernetes Secret and aoe's own
  startup log (the token URL). Treat workspace logs as secret-bearing.
- On an upstream 401 the proxy stops the workspace and answers 503, because
  aoe's rate limiter keys on the proxy IP and five failures lock everyone out
  for 15 minutes. The workspace starts again on the next request (a reload),
  not automatically.

### The home volume

On the docker and kubernetes backends the per-user volume is mounted at
`/home/aoe`, the whole `$HOME` (kubernetes uses `subPath: home`, which keeps the
volume's `lost+found` out of it). The process backend's `HOME` is already a
persistent directory. Everything under it survives a restart: clones and
worktrees, rc files, npm, cargo and pipx installs, agent logins, and aoe's own
data in `~/.config/agent-of-empires`. Anything an image bakes under `/home/aoe`
is hidden by the mount (docker copies it into a brand new volume once, and the
entrypoint would then mistake it for an old-layout volume), so do not bake files
there; image configuration lives elsewhere:

- `PATH` (`~/.local/bin`, `~/.npm-global/bin`, `~/.cargo/bin`, ahead of the
  image's dirs) and `NPM_CONFIG_PREFIX` are image `ENV`. Defaults for login and
  interactive shells are in `/etc/profile.d/cityhall.sh` and `/etc/bash.bashrc`.
- The entrypoint copies `/etc/skel` into `$HOME` with `cp -n`, so a file the
  user edited is never overwritten, and writes nothing else there.

**System packages go in a derived image.** Workspaces run as uid 1000 with no
root and no `apt`. Build a team image from the reference one and point the
image template at it:

```dockerfile
FROM cityhall/aoe:v1.26.0
USER root
RUN apt-get update && apt-get install -y --no-install-recommends \
      build-essential pkg-config libssl-dev python3 python3-venv pipx \
      openssh-client jq ripgrep less vim \
    && rm -rf /var/lib/apt/lists/*
USER aoe
```

Toolchains (rustup, Node versions, Python tools) install into `$HOME` and persist.

**Sizing.** The kubernetes PVC is `20Gi` per workspace by default
(`WORKSPACE_K8S_VOLUME_SIZE`, helm `workspaces.volumeSize`), one value for every
user. A full volume shows up as `ENOSPC` in the shell and can stop aoe writing
its own config; nothing detects or reports it. To grow it, raise the value and
let the workspace restart; the StorageClass needs `allowVolumeExpansion: true`,
and a PVC never shrinks. The StorageClass also needs `reclaimPolicy: Delete`:
`Retain` leaves the users' credentials on an orphaned volume after destroy.

**Runbook.**

- A stale dotfile the image seeds (`.bashrc`, `.profile`): delete it and restart;
  the seed restores the current one. Files you created, such as `~/.gitconfig`,
  are not recreated.
- A tool in `~/.local/bin` shadows a newer one in the image: `which -a <tool>`.
- After a Node major bump in the image: `npm rebuild -g`.
- A full reset: destroy the workspace.

**Kubernetes pod spec.** The pod runs as uid 1000 with `fsGroup: 1000` and no
ServiceAccount token. The workspace container also sets
`allowPrivilegeEscalation: false`, drops all capabilities and uses the
`RuntimeDefault` seccomp profile, so a setuid binary in a derived image cannot
elevate. The kubelet creates the `subPath` directory root-owned and
world-writable, and aoe refuses to run under a `$HOME` like that, so a root init
container (only `CHOWN` and `FOWNER`; a namespace enforcing the Pod Security
`restricted` profile rejects it) creates `home` as uid 1000, mode 0755,
before the workspace container starts. `fsGroupChangePolicy: OnRootMismatch`
stops the kubelet re-adding group write to every directory on each pod start,
which aoe also refuses. An existing PVC from an earlier build keeps its old
content at the volume root, outside `home`, where it is not read; there is no
migration.

**Upgrading an existing docker volume.** An older volume holds aoe's data at its
root, recognised by `serve.token`, `serve.lock` or `cityhall-agents` there. On the
first start after the upgrade the entrypoint moves everything at the volume root into `~/.config/agent-of-empires` (it logs one line), so sessions and
config carry over and nothing is deleted. The agent installs and logins from the
old layout come along but sit in `~/.config/agent-of-empires/cityhall-agents/`,
not where the agents look, so install and log in again once, then delete that
directory. A workspace image built before this layout is refused at start with
an error (it lacks the `cityhall.home-volume` label); rebuild or pull a current one.

## Backends

`WORKSPACE_BACKEND` selects how workspaces run (see
[Configuration](configuration.md) for every variable):

- **`docker`** (default). One container + named volume per user. CityHall on
  the docker host dials loopback-published ports; with
  `WORKSPACE_DOCKER_NETWORK` set, workspaces instead join that docker network
  with no published ports and are dialed by container DNS, which is how
  CityHall itself runs in docker/compose (socket mounted, see
  `deploy/docker-compose.workspaces.yml`). Mounting the docker socket gives
  CityHall effective root on the host; use a restricted socket proxy if that
  matters.
- **`kubernetes`**. One Deployment + Service + PVC per user, plus a Secret when
  there is a bundle token or an agent credential to inject, managed with
  `kubectl` in the CityHall pod's namespace (override with
  `WORKSPACE_K8S_NAMESPACE`). Stop scales to zero keeping the PVC; destroy
  deletes all of them. The image template must point at a registry the cluster
  can pull. Requires the RBAC and NetworkPolicy shipped in `deploy/k8s/` and
  the helm chart; the NetworkPolicy is defense in depth on top of the bearer token. Restrict who else may read Secrets in that
  namespace, per the note above.
- **`process`** (unix). Data isolation only: all workspaces share one OS user,
  so the bearer token is not a peer boundary here. One detached `aoe serve` per user with an isolated
  HOME under `WORKSPACE_PROCESS_DIR`, for VPS hosts without docker. Version
  binaries live at `$WORKSPACE_PROCESS_DIR/versions/<version>/aoe`,
  downloaded automatically from the release tarball (or installed there
  manually), so released versions only. Processes survive CityHall restarts.
  This isolates data, not security: every workspace runs as the CityHall OS
  user.

## Current limitations

- An operator can choose which agents a workspace arrives with, but cannot
  restrict it to them: a session may pick any agent, and a Full mode shell can
  run any binary on `PATH`. Enforcement needs an aoe-side allowlist
  ([agent-of-empires#3241](https://github.com/agent-of-empires/agent-of-empires/issues/3241)).
  See [Coding agents](#coding-agents).
- An SSH key only reaches a workspace running an aoe new enough to install one.
  An older aoe ignores it and `git@` remotes keep failing. See
  [SSH keys](#ssh-keys).
- A workspace built from an unreleased commit does not follow the ref it came
  from, and only the docker backend can build one. See
  [Unreleased aoe](#unreleased-aoe).
