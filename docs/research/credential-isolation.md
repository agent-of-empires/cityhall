# Credential isolation audit: multi-tenant CityHall on Kubernetes

Resolves [cityhall#122](https://github.com/agent-of-empires/cityhall/issues/122)
(map: #119). Question: what can leak between users, or to an operator, in a
multi-tenant CityHall on Kubernetes today, and which of those are acceptable for
an internal team.

Sources read at these revisions:

- cityhall `origin/main` at `2740d83` (this branch's base).
- agent-of-empires `main` at `548c58b25`, plus the two merged PRs that closed
  agent-of-empires#3238: `de6dc539` (#3360, 2026-08-14) and `710ac06ee` (#3382,
  2026-08-19). Both first shipped in aoe `v1.15.0` (`git tag --contains`).

Line numbers are for those revisions. `aoe:` prefixes a path in the
agent-of-empires checkout; everything else is a cityhall path.

## Threat model used

Three principals, in order of what they already hold:

- **User**: a CityHall member with `workspaces.use`. Reaches their own
  workspace only, through the session-gated proxy (`api/src/proxy.rs:99-103`).
- **CityHall admin**: holds `workspaces.write` and/or `workspaces.impersonate`
  (`api/src/rbac.rs:21-41`; the seeded `admin` role holds `*`,
  `api/src/rbac.rs:46`). No cluster access assumed.
- **Namespace operator**: anyone with kubectl rights in the `cityhall`
  namespace. The shipped manifests grant nothing to humans; whatever they can
  do comes from the cluster's own role bindings (see "What the shipped RBAC
  actually grants").

"Internal team" means every user is also, in practice, trusted not to attack
colleagues, but the point of this audit is which boundaries exist, so the table
rates each path on who can exercise it, not on how likely they are to.

## Leak paths

| # | Leak path | Who can exercise it | Evidence (file:line) | Recommendation | Notes |
|---|-----------|---------------------|----------------------|----------------|-------|
| 1 | **Per-user agent credentials in the per-user Secret.** `ANTHROPIC_*`, `OPENAI_API_KEY`, `GEMINI_API_KEY`, `OPENROUTER_API_KEY` land in `cityhall-workspace-u<id>-env` as `stringData` and reach the pod via `envFrom`. | Anyone with `secrets get` or `list` in the namespace (`kubectl get secret -o yaml`); CityHall's own ServiceAccount. | Secret name `api/src/orchestrator/kubernetes.rs:47-49`; values collected `kubernetes.rs:281-297`; Secret + `envFrom` emitted `kubernetes.rs:386-395`; CityHall SA granted `secrets get,list,create,patch,update,delete` in `deploy/k8s/workspaces-rbac.yaml:37-39` and `deploy/helm/cityhall/templates/workspaces-rbac.yaml:39-41`. | **document** (already partly done) + **fix** (ship a human-facing Role) | The Deployment itself never carries a value: test `kubernetes.rs:662-705` asserts the literal is absent from the Deployment JSON. So `deployments get` alone does not leak. Not encrypted at rest unless the cluster enables it (`kubernetes.rs:382-385`). |
| 2 | **Same credentials through the pod.** `kubectl exec <pod> -- env`, or `kubectl debug`, prints every `envFrom` value. | Anyone with `pods/exec` or `pods/ephemeralcontainers` in the namespace. | Pod gets the whole Secret as environment: `kubernetes.rs:387`. Pod spec sets no `securityContext`, no `automountServiceAccountToken: false`: `kubernetes.rs:437-443`. CityHall's own Role deliberately has `pods get,list` only (`workspaces-rbac.yaml:40-42`), so CityHall cannot exec. | **document** | `pods/exec` is in the stock `edit` and `admin` ClusterRoles, so any dev bound to those in the namespace has this. The shipped manifests cannot prevent it; the deployment docs need to say "do not bind `edit`/`admin` to team members in the cityhall namespace". |
| 3 | **Same credentials, docker backend.** `docker inspect` on the container shows every `--env-file` value. | Anyone with the docker socket (which `docs/workspaces.md:522-524` already calls effective root). | `api/src/orchestrator/docker.rs:521-528` (comment states `docker inspect` still shows them); env file is mode 0600 and deleted after the CLI returns `docker.rs:431-449`. | **accept** | Out of the kubernetes scope but in the ticket. Socket access already implies host root; nothing to narrow. |
| 4 | **Same credentials, process backend.** `/proc/<pid>/environ` of each `aoe serve`. | The CityHall OS user, which is the user every workspace runs as. | `api/src/orchestrator/process.rs:207-216`; `docs/workspaces.md:435-438`, `538-540`. | **accept** (documented) | Backend is explicitly "isolates data, not security". Not a kube concern. |
| 5 | **The bundle token.** Holding `AOE_CITYHALL_BUNDLE_TOKEN` fetches that user's composed bundle: admin settings + `[git]` with the **decrypted HTTPS token** and **SSH private key**. After #47 it lives in: (a) the per-user Secret, (b) the pod environment, (c) the `workspaces.bundle_token` column, in plaintext. | (a)(b): same principals as rows 1 and 2. (c): anyone who can read the database (holders of `DATABASE_URL`, i.e. the `cityhall` Secret, or Postgres access). Any process inside the workspace pod (row 11). | Token minted `api/src/workspaces.rs:225` and `:254` (`random_token(48)`, stored as-is); looked up by equality, so plaintext at rest `api/src/handlers/workspace_config.rs:371-375`; endpoint composes the decrypted git token `workspace_config.rs:443-457` and the SSH key below it; into the Secret `kubernetes.rs:283-289`; the shipped RBAC comment says exactly this `workspaces-rbac.yaml:9-11`. | **fix** (rotation) + **document** | There is no rotate or revoke path: `ensure_bundle_token` only backfills `NULL` (`workspaces.rs:242-265`), and `destroy` deletes the Secret (`kubernetes.rs:230-248`) but leaves the DB token valid, so a copied token keeps working after the workspace is destroyed until the user row is deleted. The endpoint is on CityHall's main listener at `WORKSPACE_BUNDLE_ORIGIN/api/workspace-bundle` (`api/src/orchestrator/mod.rs:443-458`), reachable from any pod that can reach CityHall. Candidate fix: regenerate the token on destroy and on git-credential change, and add an admin "rotate" action. |
| 6 | **The `cityhall` Secret itself.** `CITYHALL_SECRET_KEY` + `DATABASE_URL` together decrypt every stored secret of every user (git tokens, SSH keys, agent credentials, SMTP, OIDC). | Anyone with `secrets get` in the namespace. | `deploy/k8s/secret.yaml:11-15` (one Secret holds DB password, DSN, and the key); `api/src/crypto.rs:6-8` (AES-GCM under that key); `api/src/secrets.rs:27-35` lists the stores. Helm `existingSecret` path exists: `deploy/helm/cityhall/values.yaml:21-23`. | **document** | Bigger blast radius than row 1 and the same principal. The honest statement for the docs: in this namespace, `secrets get` is equivalent to every user's credentials, so it must be a cluster-admin verb, not a team verb. An external secret manager (`existingSecret`) does not change who can `kubectl get` the resulting Secret. |
| 7 | **Admin impersonation (`workspaces.impersonate`).** The admin's browser is routed to the target's workspace for up to 30 minutes; everything the target's aoe dashboard can do, the admin can do. | Holders of `workspaces.impersonate` (seeded `admin`). | Routing `api/src/proxy.rs:81-103`; mint `api/src/handlers/workspaces.rs:189-204`; audit lines `proxy.rs:216-220`, `:241-248`; 30 min TTL `proxy.rs:141`; WebSockets survive revocation `docs/workspaces.md:136-138`. | **accept** + **document** | Does not expose credentials **through CityHall**: stored values are never returned by any CityHall endpoint (`docs/api.md:663-664`, `handlers/agent_credentials.rs` returns `value_set`/`usable` only). It exposes **sessions and transcripts**, and, because the workspace runs the user's agent with tool execution, an admin can ask that agent to run `env` or read `~/.config/agent-of-empires/git-credentials` (row 11). So impersonation is effectively "the admin can obtain the user's keys by asking the agent", audited only as "admin entered workspace". The docs should say so plainly. |
| 8 | **Admin overwrite of a user's credentials.** `PUT /api/users/{id}/agent-credentials/{var}` lets an admin set a value the user's agents will then use. | Holders of `workspaces.write`. | `api/src/handlers/agent_credentials.rs:117-144` (gated on `workspaces.write`, logs `target_user_id`). | **accept** | Documented feature ("hand a workspace over ready to use", `docs/workspaces.md:364-368`). Write-only; cannot read back. The catalog has no base-URL variable, so an admin cannot redirect a user's traffic this way (`api/src/agent_credentials.rs:41-72`). |
| 9 | **Agent state on the PVC.** `~/.claude`, `~/.claude.json`, `~/.codex`, `~/.gemini`, opencode dirs (subscription logins), plus aoe's own `git-credentials` and `ssh/id_cityhall`, all live on `cityhall-workspace-u<id>-data`. | Anyone who can mount the PVC: `pods create` in the namespace (RWO only prevents a second node, not a second pod on the same node), `pods/exec` on the workspace pod, node root, storage-backend admins, and CityHall's own SA (`persistentvolumeclaims` + `deployments create`). | PVC name `kubernetes.rs:41-43`, mounted at the aoe app dir `kubernetes.rs:28`, `:349`; entrypoint symlinks the agent dirs onto it `deploy/aoe-image/Dockerfile:238-249`; aoe writes the git token owner-only into the app dir `aoe:src/session/cityhall_bundle.rs:27-33`, `:251-258`, and the SSH key `:274-283`. | **document** + **fix** (pod hardening) | Owner-only file modes mean nothing to a principal that mounts the volume as root. Candidate fix in `render_manifests`: `securityContext.runAsNonRoot`, `runAsUser: 1000`, `automountServiceAccountToken: false` on the workspace pod (the image already runs as uid 1000, `Dockerfile:179-188`). The shipped manifests bind no role to the `default` SA (`workspaces-rbac.yaml:53-57` binds only `cityhall`), so the mounted SA token is inert today but is still a token sitting in every workspace. |
| 10 | **Network: workspace to workspace.** | A workspace pod (the user's agent, or anything the user runs). | NetworkPolicy selects `cityhall.managed=true`, `policyTypes: [Ingress]`, allows only `app: cityhall` on 8080: `deploy/k8s/workspaces-networkpolicy.yaml:10-21`; helm equivalent with the release selector `deploy/helm/cityhall/templates/workspaces-networkpolicy.yaml:12-23`, `_helpers.tpl:11-14`. Workspaces run `--auth none` (`kubernetes.rs:337`). | **accept** | Blocked, provided the CNI enforces NetworkPolicy (both files say so). Without enforcement, every workspace is an unauthenticated aoe reachable from every pod; `docs/workspaces.md:530-533` already warns. |
| 11 | **Network: workspace egress.** No egress rule, so a workspace pod reaches CityHall's API (needed for the bundle), `cityhall-db:5432`, the kube API, and the internet. | A workspace pod. | Policy has no `Egress` type `workspaces-networkpolicy.yaml:13-14`; Postgres has no NetworkPolicy at all (`deploy/k8s/postgres.yaml`, `deploy/k8s/kustomization.yaml:4-12` lists only the workspace policy). | **fix** (small) + **document** | Postgres is password-gated and the password is only in the `cityhall` Secret, so this is defence in depth, not a leak on its own. But a dev-tier workspace is a shell the user controls, and a prompt-injected agent is a shell nobody controls. A `cityhall-db` ingress policy allowing only `app: cityhall` is a five-line manifest. |
| 12 | **Inside the workspace: any process reads aoe's environment.** Everything in the Secret, including `AOE_CITYHALL_BUNDLE_TOKEN` and every provider key, is in `aoe serve`'s environment; the agent runs as the same uid and can read `/proc/<aoe pid>/environ` or just `env` in a tool call. | The workspace owner, and anything running as them: a prompt-injected agent, a malicious npm package pulled by an agent install. | aoe clears the env and forwards only `HOST_BASE_ENV_KEYS` plus the adapter allowlist to the ACP child (`aoe:src/acp/acp_client/spawn.rs:197-233`, `aoe:src/session/environment.rs:11-20`), so the token is not handed over, but it is one `cat /proc/1/environ` away. The entrypoint scrubs the env for the boot-time agent installers for exactly this reason: `Dockerfile:264-280`. | **accept** (same tenant) + **document** | Same-tenant exposure, so not a cross-user leak. Worth one sentence in the docs because the bundle token is a durable credential (row 5) that a one-off exfiltration keeps working. Rotation (row 5 fix) is what limits the damage. |
| 13 | **Proxy header hygiene.** CityHall session cookie or `Authorization` forwarded to the workspace. | n/a | Stripped: `api/src/proxy.rs:319-342`, test `:544-564`. | **accept** | Checked because a workspace is user-controlled code; nothing crosses. |

## What the shipped RBAC actually grants

Both `deploy/k8s/workspaces-rbac.yaml` and the helm chart create **one**
ServiceAccount (`cityhall`), **one** namespaced Role, and **one** RoleBinding:

| Resource | Verbs | Where |
|----------|-------|-------|
| `apps/deployments` | get, list, create, patch, update, delete | `deploy/k8s/workspaces-rbac.yaml:24-26`, helm `:26-28` |
| `apps/deployments/scale` | get, patch, update | `:27-29`, helm `:29-31` |
| `services`, `persistentvolumeclaims` | get, list, create, patch, update, delete | `:30-32`, helm `:32-34` |
| `secrets` | get, list, create, patch, update, delete | `:37-39`, helm `:39-41` |
| `pods` | get, list | `:40-42`, helm `:42-44` |

Nothing is bound to any human, group, or to the `default` ServiceAccount the
workspace pods run under. So the sentence in the ticket, "readable by anyone
with `secrets` read in the namespace", is right, and the shipped files neither
cause nor prevent it: who has `secrets get` is decided by whatever the cluster
binds in that namespace. The stock `view` ClusterRole excludes secrets; `edit`
and `admin` include secrets read **and** `pods/exec`. The practical rule for a
team cluster is therefore: team members get at most `view` in the `cityhall`
namespace, and the namespace is administered by the same people who hold
`CITYHALL_SECRET_KEY`.

The file comments already say "keep `secrets` read narrower than
`deployments` read" (`workspaces-rbac.yaml:5-11`). What is missing is a shipped
example of the narrow human Role, and the explicit statement that `pods/exec`
and `pods create` are equivalent to `secrets get` here (rows 2 and 9).

## Is the "terminal-only" marking stale after agent-of-empires#3238?

**Yes, for any workspace running aoe v1.15.0 or later. All of it.**

What changed upstream, in order:

1. `de6dc539` (PR #3360, closed #3238, merged 2026-08-14). Added a per-adapter
   `env_allowlist` resolved from a catalog in `install_hints.rs`, consumed by
   every spawn path. `codex-acp` now receives `CODEX_API_KEY`, `OPENAI_API_KEY`,
   `OPENAI_BASE_URL`, `CODEX_HOME`; `gemini` receives `GEMINI_API_KEY` and the
   Google set; `opencode` receives `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`,
   `GEMINI_API_KEY`, `OPENROUTER_API_KEY`, and more
   (`aoe:src/acp/install_hints.rs:45-83` at main).
2. `710ac06ee` (PR #3382, merged 2026-08-19, marked breaking). Removed the four
   Claude variables from the fixed `ALWAYS_FORWARD_ENV`, so Claude keys are now
   also per-adapter (`claude-agent-acp`, `opencode`, `aoe-agent`, `prime-agent`
   only). Custom `agent_acp_cmd` adapters and `pi`/`omp`/`kimi`/`vibe` get no
   ambient provider keys at all.
3. `d3c5fb66c` (PR #4307, 2026-10-15). `ALWAYS_FORWARD_ENV` no longer exists
   under that name; the infrastructure-only base list is
   `HOST_BASE_ENV_KEYS` in `aoe:src/session/environment.rs:11-20` (`PATH`,
   `HOME`, `XDG_CONFIG_HOME`, `LANG`, `LC_ALL`, `TERM`, `USER`,
   `SSH_AUTH_SOCK`). The ticket's pointer to `src/acp/acp_client.rs` is also
   stale: that file was split into `src/acp/acp_client/*.rs` by `f197ae07c`
   (#3593, 2026-08-30).

Both #3360 and #3382 are first contained in tag `v1.15.0`.

What is stale on the CityHall side as a result:

| Stale item | Location | What it says | What is true now |
|------------|----------|--------------|------------------|
| `structured_view: false` on `OPENAI_API_KEY`, `GEMINI_API_KEY`, `OPENROUTER_API_KEY` | `api/src/agent_credentials.rs:57-71` | Not forwarded to structured-view agents | Forwarded to their own adapters (codex-acp, gemini, opencode). |
| `STRUCTURED_VIEW_LIMITATION` text | `api/src/agent_credentials.rs:76-77` | "Available in terminal sessions only. Structured-view agents do not receive this variable yet." | Wrong twice: the variable now reaches its adapter, and a `--cityhall` workspace has **no terminal session at all** (`aoe:src/server/live_ws.rs:633-641`, `runtime_ws.rs:57` refuse the PTY and runtime sockets in CityHall mode), so before v1.15.0 these three keys reached nothing in a workspace. |
| API field and docs | `api/src/handlers/agent_credentials.rs:191-192`, `docs/api.md:667-696` | `structured_view` / `limitation` per entry | The real answer is per adapter, not per variable: `ANTHROPIC_API_KEY` reaches `claude-agent-acp` and `opencode` but not `codex-acp`. |
| UI badge | `web/src/components/AgentCredentials.tsx:133-138` | Renders `limitation` with a warning icon | Shows a warning on three entries that now work. |
| Table column "Reaches structured-view agents" | `docs/workspaces.md:372-379` | yes / yes / yes / no / no / no | All six reach their matching adapter. The column should become "Forwarded to (aoe >= v1.15.0)" naming adapters. |
| "The last three reach terminal sessions but not structured-view agents" paragraph | `docs/workspaces.md:403-408` | Links #3238 as open work | #3238 is closed; the paragraph should describe the per-adapter rule and the v1.15.0 floor. |
| Current limitations bullet | `docs/workspaces.md:544-545` | "Agent credentials only reach structured-view agents for Claude" | Remove, or replace with "custom adapters receive no ambient provider keys". |
| Symlink rationale | `deploy/aoe-image/Dockerfile:202-205`, `docs/workspaces.md:288-291` | env-based `CODEX_HOME` relocation would fail in structured view | `CODEX_HOME` is now on codex-acp's allowlist. Symlinks are still the right call (they cover every agent and the install prefixes), only the stated reason is partly stale. |

One thing #3238 asked for and did not get: a way to ask a given aoe build which
variables it forwards. CityHall still has to carry a static table, so whatever
replaces `structured_view` needs an "as of aoe vX" qualifier, and a workspace
pinned below v1.15.0 reverts to the old behaviour.

## Decisions suggested for the map

- Accept: rows 3, 4, 7, 8, 10, 12, 13. All are same-tenant, already documented,
  or inherent to the backend.
- Document: rows 1, 2, 6, 9, 11, plus the impersonation caveat in row 7 and the
  human-RBAC rule above. One docs PR.
- Fix: bundle token rotation (row 5), a `cityhall-db` NetworkPolicy and
  workspace pod `securityContext` (rows 9, 11), a shipped example human Role
  (row 1), and the #3238 cleanup (code, API, UI, docs).

## Candidate follow-up tickets

1. **Rotate the workspace bundle token**: regenerate on destroy and on
   git-credential or SSH-key change; add an admin rotate action. Evidence row 5.
2. **Drop the stale structured-view marking** after agent-of-empires#3238:
   remove `structured_view`/`limitation` or replace them with a per-adapter
   "forwarded to" list gated on aoe >= v1.15.0; update `docs/workspaces.md`,
   `docs/api.md`, the UI badge, and the Dockerfile comment.
3. **Harden the workspace pod spec**: `securityContext` (non-root, uid 1000),
   `automountServiceAccountToken: false`. Evidence row 9.
4. **Ship a `cityhall-db` NetworkPolicy** (ingress from CityHall only) in
   `deploy/k8s/` and the helm chart. Evidence row 11.
5. **Ship an example human Role** for the namespace (no `secrets`, no
   `pods/exec`, no `pods create`) and state the rule in `docs/deployment.md`.
   Evidence rows 1, 2, 9.
6. **Docs: name the impersonation reach**: an admin in a user's workspace can
   obtain that user's keys by asking the agent; the audit line records entry
   only. Evidence row 7.

_Note: drafted by Claude via back-and-forth with @Seluj78._
