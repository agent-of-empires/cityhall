# Adopting the per-workspace bearer: sequencing against aoe

Resolves cityhall#127 (research ticket under map cityhall#119) for cityhall#100.

As of 2026-10-26. Sources: agent-of-empires `origin/main` at `227d023a` (2026-10-26
13:59 UTC), open PR agent-of-empires#4743 at its 2026-10-26 state, cityhall `main`
at `2740d83`, and the GitHub threads cited inline. Line numbers are against those
revisions. "Daemon", "Client", "Principal" follow aoe's `CONTEXT.md`.

## TL;DR

- Nothing on aoe main implements a `Proxy` principal or a "token mode" distinct
  from the default `--auth token`. ADR 0002 (merged 2026-10-14, #4408) is the
  only artifact; no implementation ticket exists for the typed Principal with a
  Proxy variant (#3897 covers `LocalPeer` only).
- The 426 `tls_required` gate is in PR #4743 (open, approved by both maintainers,
  not merged). Its opt-out flag is `--dangerously-allow-plaintext-remote`.
  Trusted proxy peers are loopback only; there is no configured-peer setting.
- CityHall's docker and kubernetes hops are non-loopback plaintext, so they get
  426 once #4743 ships, and so does the loopback process backend, because the
  proxy currently asserts `x-forwarded-proto: http` and #4743 refuses an explicit
  non-https assertion from a loopback proxy.
- `--auth none --behind-proxy` on a non-loopback bind keeps starting after #4743;
  the refusal is per request, so the grace window is CityHall's version pin.
- Switching to `--auth token` needs no aoe change for the token check itself,
  but aoe has no hook to accept a CityHall-minted token: the Daemon generates
  `serve.token` and regenerates it after 24h idle. CityHall can pre-write the
  file from its own entrypoint as an interim; a proper input is an aoe ask.
- In token mode a Bearer-authenticated response echoes the token in
  `Set-Cookie: aoe_token=` and `x-aoe-token`, which CityHall's proxy forwards
  today and the dashboard persists to localStorage. The proxy must strip both.
- An in-container `aoe` CLI reaches the Daemon over `<app_dir>/daemon/api.sock`
  already on main (merged 2026-10-22), independent of #3897. The separate
  runtime-read socket is suppressed in `--cityhall` mode, and open PR #4886
  removes its disk fallback, so a dev shell in a workspace would lose `aoe list`.

## Findings per ticket bullet

### 1. Which aoe change ships the Proxy principal and token mode, and its state

**Proxy principal: decided, not implemented, no ticket.**

- ADR 0002 `docs/adr/0002-single-user-daemon-multi-user-in-cityhall.md:11-19`
  (merged by #4408 on 2026-10-14, closes #4394) defines the four Principal
  variants (Local owner, Device, Token holder, Proxy) and says "CityHall reaches
  its Daemons with a per-workspace bearer. CityHall mints the token and its proxy
  injects it; the Daemon runs in token mode and sees a Proxy principal." Status
  line is still "proposed" (`:3`).
- `CONTEXT.md:243-244` lists Principal as "Target (#4394)" and describes today as
  "requests authenticate with a bearer token or passphrase login".
- On main there is no Principal type. The auth middleware stamps
  `LocalAuthorization::UnixOwner(uid)` for Unix-socket peers
  (`src/server/auth.rs:498-506`) and `AuthenticatedTokenHash` for token callers
  (`:685-689`); `--auth none` stamps a zero hash (`:551-554`). Nothing
  distinguishes a proxy from any other bearer holder.
- #2734's Decisions table maps the Principal seam to ADR 0002 and lists
  implementation slices I1 = #3896 (error contract, TLS gate) and I2 = #3897
  (Unix socket, `LocalPeer`). Neither mentions a Proxy variant; #3897's body
  (created 2026-09-10, 0 comments) scopes "a typed `LocalPeer` Principal".
  #4506 (CityHall as a caller on its own behalf, an operator Principal) was
  closed as premature on 2026-10-15 by njbrake.
- Consequence: for CityHall's bearer, the Daemon will see a Token holder until a
  Principal type lands. ADR 0002 `:14` defines Token holder as "a caller
  presenting the `serve.token` bearer", so functionally the proxy is admitted
  the same way; what the Proxy variant adds later is the authorization seam
  (and the operator principal of #4506), not admission.

**Token mode: it is the default `--auth token`, shipped long ago.**

- `src/cli/serve.rs:18-22,53-58`: `AuthMode::{Token, Passphrase, None}`; token is
  the default. `src/server/startup.rs:344-347` issues a token only in Token mode.
- There is no second "token mode" setting. What ADR 0002 calls token mode is
  `--auth token` with the bearer supplied by CityHall, and that supply path does
  not exist (see finding 6).

**The TLS gate and error contract: PR #4743, open and approved.**

- `src/daemon/error.rs:5,15,42,73,104` on main already has
  `ERROR_CODE_HEADER = "aoe-error-code"` and `ApiErrorCode::TlsRequired`
  (`"tls_required"`, status 426). Introduced by #4009 (merged 2026-10-22). Nothing
  on main emits it: the only references are the enum itself.
- PR #4743 "feat(daemon): align native transport security and safe API errors"
  (Seluj78, opened 2026-10-20, closes #3896): adds `src/server/ingress.rs` with
  `plaintext_gate`, layered outermost in the router so it runs before the Host
  check and authentication; the Unix listener is exempt. Review state on
  2026-10-26: `reviewDecision: APPROVED` (jerome-benoit 2026-10-24 14:33,
  njbrake 2026-10-26 11:06), `mergeable`, not merged. njbrake's D1b ratification
  on #2734 (2026-09-11) says the gate "Needs a minor release and release notes".
- #3896 was ratified on 2026-10-19 by Seluj78 (comment on #3896) after the
  original ratification by njbrake on 2026-09-11 (#2734, comment 5635868081).

### 2. Exact flag and setting names

| Concept | Name | Where | State |
| --- | --- | --- | --- |
| Token mode | `--auth token` (default), `AuthMode::Token` | `src/cli/serve.rs:18-22,53-58` | shipped |
| Legacy alias | `--no-auth` = `--auth none` | `src/cli/serve.rs:60-63` | shipped |
| Proxy mode | `--behind-proxy` (persisted as `ServeLaunch.behind_proxy`, runtime `AppState.behind_tunnel`) | `src/cli/serve.rs:65-72`, `src/server/startup.rs:809` | shipped |
| Host allowlist | `--allowed-host`, `--allowed-origin` (required with `--behind-proxy`) | `src/cli/serve.rs:74-95,300-312` | shipped |
| TLS opt-out | `--dangerously-allow-plaintext-remote`; persisted `ServeLaunch.allow_plaintext_remote`; `AppState.allow_plaintext_remote` | PR #4743 diff, `src/cli/serve.rs` and `src/server/state.rs` hunks | open PR |
| Gate response | `426` with `aoe-error-code: tls_required` | `src/daemon/error.rs:42,73` (enum on main), emitted by PR #4743 `ingress.rs` | enum shipped, emission in open PR |
| Configured proxy peers | none. "Trusted proxy peers are the loopback addresses implied by `--behind-proxy` and `--remote`; there is no peer-list setting" (PR #4743 body); `via_trusted_proxy = proxy_mode && peer_ip.is_loopback()` | PR #4743 `ingress.rs` | not yet named, no ticket; closest is open #2735 (identity-aware proxy tiers) |
| Forwarded headers honored | `X-Forwarded-For`, `X-Forwarded-Proto` from a loopback peer in proxy mode; `cf-connecting-ip` only for a Cloudflare `--remote` tunnel | PR #4743 docs hunk `docs/guides/web-dashboard.md:143-144` | open PR (main: XFF and cf-connecting-ip from loopback, `src/server/auth.rs:26-56`) |
| Proxy principal | none on main; ADR 0002 only | `docs/adr/0002-*.md:15` | not yet named, no ticket |
| Token supply by an operator | none. Token is generated or read from `<app_dir>/serve.token` | `src/server/token.rs:203-238` | not yet named, no ticket |
| CityHall mode | `--cityhall` or `AOE_CITYHALL_MODE` | `src/cli/serve.rs:96-102,520` | shipped |
| Bundle | `AOE_CITYHALL_BUNDLE_URL`, `AOE_CITYHALL_BUNDLE_TOKEN` | `src/cli/serve.rs:1363-1365` | shipped |
| Daemon API socket | `<app_dir>/daemon/api.sock` | `src/daemon/transport.rs:35-40` | shipped (#4009, 2026-10-22) |
| Client endpoint override | `AOE_DAEMON_URL`, `AOE_DAEMON_TOKEN`, `AOE_DAEMON_PASSPHRASE` | `docs/structured-view.md:189-202` | shipped |

### 3. Do the docker and kubernetes `0.0.0.0` hops trip the TLS gate, and what is sanctioned

**Yes, all three backends trip it as CityHall is written today.**

What CityHall does now:

- docker: `aoe serve --host 0.0.0.0 --port 8080 --auth none --behind-proxy
  --allowed-host … --allowed-origin … --cityhall`
  (`api/src/orchestrator/docker.rs:534-561`); the port is published on the host's
  `127.0.0.1:0` (`:518`) or the container joins `WORKSPACE_DOCKER_NETWORK`
  (`:506-515`). From the Daemon's view the peer is the docker bridge gateway or
  CityHall's container IP, never loopback.
- kubernetes: same args (`api/src/orchestrator/kubernetes.rs:330-347`), dialed by
  Service DNS (`:135`); the peer is the CityHall pod IP. Isolation relies on
  `deploy/k8s/workspaces-networkpolicy.yaml` (`docs/workspaces.md:530-533`).
- process: `--host 127.0.0.1` (`api/src/orchestrator/process.rs:172-193`), so the
  peer is loopback.
- The proxy dials `http://{addr}` (`api/src/proxy.rs:114`) and sets
  `x-forwarded-proto: http` on every forwarded request (`:351`).

How #4743 judges each (`ingress.rs`, `Ingress::resolve`):

- `plaintext_remote = !externally_secure && (!peer_ip.is_loopback() || forwarded_plaintext)`.
- `externally_secure` is only ever true when the peer is loopback, proxy mode is
  on, and the last `x-forwarded-proto` element is `https`. The PR's own test
  asserts a non-loopback peer sending `x-forwarded-proto: https` in proxy mode is
  still `plaintext_remote` (`("203.0.113.9", [("x-forwarded-proto","https")], true, true)`).
- A loopback peer that asserts `x-forwarded-proto: http` is refused
  (`("127.0.0.1", [("x-forwarded-proto","http")], true, true)`); one that omits
  the header is not ("A loopback proxy that omits `X-Forwarded-Proto` makes no
  HTTPS assertion and is not refused", docs hunk `web-dashboard.md:144`).
- The gate ignores the auth mode entirely, so `--auth none` does not exempt it.

Result per backend once #4743 ships, with CityHall unchanged: docker 426,
kubernetes 426, process 426 (because of the explicit `http` assertion). Process
passes as soon as the proxy stops asserting `http`.

**Sanctioned shapes** (ADR 0002 `:19`: "the proxy to Daemon hop is HTTPS,
loopback, or inside the workspace container"; D1b on #2734: the opt-out is
"named so nobody sets it by accident, logged at every startup, never inferred
from the address"):

1. **Per-bind opt-out**: keep `--host 0.0.0.0` and add
   `--dangerously-allow-plaintext-remote`. One flag, persisted in `ServeLaunch`,
   logged as `DANGEROUS` at every start. This is the D1b escape and matches
   CityHall's existing trust model (network isolation is already what protects
   the `--auth none` port, `docs/workspaces.md:530-533`). Downside: the per-start
   warning in every workspace log, and the hop stays plaintext on the docker
   bridge or pod network.
2. **Loopback bind plus an in-container sidecar**: `--host 127.0.0.1` in the aoe
   container, a sidecar sharing the network namespace (k8s: second container in
   the pod; docker: `--network container:<ws>`) that terminates TLS from CityHall
   and forwards to `127.0.0.1:8080` with `X-Forwarded-Proto: https`. The Daemon
   sees a loopback peer in proxy mode asserting https, which #4743 honors. This
   is the literal "inside the workspace container" shape and encrypts the
   CityHall-to-workspace hop. Cost: a sidecar image, a per-workspace cert
   (CityHall-issued) and a change to the proxy's dial scheme.
3. **Trusted-proxy forwarded-https from a non-loopback peer**: not available.
   #4743 deliberately has no peer list; D1 on #2734 said "only configured proxy
   peers may supply them", and #4743 narrowed "configured" to loopback. Would
   need an aoe decision (ask 2 below).
4. **HTTPS directly to the Daemon**: not available, aoe has no native TLS
   (#3896 acceptance: "no native TLS addition").

Note the opt-out cannot be passed to an aoe older than the #4743 release: clap
rejects unknown flags, so the workspace would fail to start. CityHall already
knows each workspace's aoe version (`api/src/workspaces.rs:267-275`), so the
flag must be version-gated.

### 4. Does anything in-container still reach the Daemon once TCP needs auth, and is #3897's UDS needed

**Nothing CityHall runs reaches the Daemon over TCP today, and a dev's CLI already
has a UDS path on main.**

- The image entrypoint (`deploy/aoe-image/Dockerfile:206-280`) only creates
  symlinks and backgrounds `provision-agents.sh`, then `exec "$@"`. No `aoe`
  command runs before serve.
- Force-on telemetry runs `aoe telemetry enable` before `exec`
  (`api/src/orchestrator/mod.rs:83`, `:139-150`; process backend
  `process.rs:494-510`). It is a config write; `src/cli/telemetry.rs` has no
  `DaemonClient` use, and it runs before any Daemon exists.
- The Daemon always binds `<app_dir>/daemon/api.sock` beside the TCP listener,
  in CityHall mode too (`src/server/startup.rs:551-562`; path
  `src/daemon/transport.rs:35-40`). Merged in #4009 on 2026-10-22. ADR 0003 `:13`
  fixes the path as `HOME`-derived, so the process backend's per-user `HOME`
  gives each Daemon its own socket. The auth middleware admits the kernel-verified
  owner without any credential (`src/server/auth.rs:498-506`); TCP loopback gets
  no bypass (the TCP branch goes straight to the token check, `:484,610-671`),
  and `--behind-proxy` already withdraws the passphrase-wall loopback carve-out
  (#3849, 2026-09-09).
- So in token mode a terminal inside the workspace works for Daemon-backed verbs
  (ACP attach, lifecycle) as the same UID over `api.sock`, with no token and no
  dependency on #3897. #3897's obligation is converging the separate runtime-read
  protocol onto the shared router and typing the principal, not creating the
  socket.
- Two caveats for the dev tier (map #119):
  - The runtime-read socket used by read-only CLI commands (`aoe list` and
    friends) is **not published in CityHall mode**: `publish_runtime_uds` returns
    `None` when `state.cityhall_mode` (`src/server/startup.rs:1245-1255`, since
    #4136 on 2026-10-19). Those commands then read local storage
    (`docs/api.md:212`). Open PR #4886 "fix(cli): require daemon-backed runtime
    reads" removes that disk fallback, after which `aoe list` inside a
    `--cityhall` workspace hard-fails. Setting `AOE_DAEMON_URL=http://127.0.0.1:8080`
    with `AOE_DAEMON_TOKEN` would work (loopback plaintext with credentials is
    allowed client-side, `src/daemon/mod.rs:1065-1067`), but it hands the
    per-workspace bearer to the user's shell.
  - The `--cityhall` mutation gate has no principal exemption
    (`src/server/access.rs:455-489`), so a Unix-owner CLI is still refused on
    non-allowlisted mutations with `cityhall_mode`. That is the lockdown the map
    wants to remove, not a transport question.

### 5. Does `--auth none` keep working on a non-loopback bind in the meantime

**Yes at startup, both before and after #4743; the break is per request and
version-pinned.**

- Today: `validate_auth_combination` accepts `--auth none` on a non-loopback bind
  when `--behind-proxy` is set (`src/cli/serve.rs:263-275`); `check_auth_gate`
  treats None as always satisfied (`src/server/startup.rs:216-241`); the
  middleware passes every request through (`src/server/auth.rs:530-555`).
- ADR 0002 `:19` keeps the option and defers "whether it survives on non-loopback
  binds" to #3839. #3839's closing decision (njbrake, 2026-09-11) and #4743 do
  not remove it; #4743 only reworded the validation message. The docs hunk still
  lists `--auth=none --host 0.0.0.0` as "refuses to start without
  `--behind-proxy`", i.e. allowed with it.
- After #4743: the process starts, the request-time gate answers 426 to
  non-loopback peers and to loopback peers asserting a non-https forwarded proto.
- Grace window: CityHall pins the workspace aoe version
  (`api/src/workspaces.rs:267-275`, `pinned_version` or `default_version`), and
  the gate needs a minor release. The break arrives only when an operator moves
  a workspace to a release containing #4743.

### 6. Additional findings that shape #100

**No hook for a CityHall-minted token.** `load_or_generate_token` reads
`<app_dir>/serve.token` if its mtime is under 24h and the content is 64 hex or
legacy 32 chars (`is_valid_token_format`, `src/server/token.rs:195-201`), refreshes
the mtime, and otherwise generates a new token (`:203-238`). No env var or flag
supplies one. Rotation runs only under `--remote` (every 4h with a 5-minute
grace, `src/server/startup.rs:356-363,1118-1136`), so in `--behind-proxy` mode
the token is stable for the process lifetime and across restarts within 24h of
the last start. A workspace idle-stopped for over 24h comes back with a new
token. CityHall options, in order of cleanliness:

- aoe adds an input (ask 1).
- CityHall pre-writes `serve.token` before serve: the process backend writes
  `$HOME/.config/agent-of-empires/serve.token` directly; the docker and kubernetes
  backends pass the token in the existing env file or Secret and the entrypoint
  (CityHall's own `deploy/aoe-image/Dockerfile`) writes it with mode 0600 before
  `exec`. Fresh mtime and valid format mean the Daemon adopts it. Relies on an
  undocumented input contract (the file is documented as storage only,
  `docs/guides/web-dashboard.md:160`).
- CityHall reads the token back after each start (`docker exec`/`kubectl exec`
  `cat serve.token`, or the file under `HOME`). Works with no entrypoint change
  but costs a round trip per start and leaves a window where the proxy holds a
  stale value.

**The token echo.** When a request authenticates via `Authorization: Bearer`,
the middleware attaches `Set-Cookie: aoe_token=<token>; HttpOnly; SameSite=Strict;
Secure` (Secure because `behind_tunnel`) and `x-aoe-token: <token>` to the
response (`src/server/auth.rs:70-100,691-692,724-726`). CityHall's proxy forwards
every upstream response header except `connection`, `keep-alive` and
`transfer-encoding` (`api/src/proxy.rs:375-382`), and the dashboard adopts
`x-aoe-token` into localStorage (`web/src/lib/fetchInterceptor.ts:86-87`,
`web/src/lib/token.ts:37-49`). Without stripping, every user learns their
workspace bearer on the first proxied API call.

**Rate limiting keys on the proxy.** Under `--behind-proxy`, forwarded client
identity is trusted only from a loopback peer (`src/server/auth.rs:37`; same in
#4743). For docker and kubernetes the key is the proxy's socket IP, so all
users of one workspace share a bucket; five failures lock it for 15 minutes
(`src/server/rate_limit.rs:11-13`). The only failure source in token mode is a
stale token, so CityHall must treat a 401 from a workspace as "re-read or
re-mint", never retry blindly.

**WebSocket path.** `forward_headers` is shared by `http_forward` and
`websocket_tunnel` (`api/src/proxy.rs:329-358,369,403`), so one Bearer insertion
covers both. The Daemon checks the Bearer before any `aoe-token.<x>` subprotocol
(`src/server/auth.rs:615-637`), so a browser without a stored token is fine.

**What `--behind-proxy` actually requires.** `--allowed-host` is mandatory
(`src/cli/serve.rs:300-312`); CityHall already passes the public host and origin
and forwards the public `Host` (`api/src/proxy.rs:344-350`), so switching the
auth mode changes nothing on the Host/Origin gate (`src/server/access.rs:226-246`).

## (a) Recommended sequencing for cityhall#100

**Phase 0, now, no behavior change for users, no aoe dependency**

1. Proxy: stop asserting `x-forwarded-proto: http`. Forward `https` only when
   CityHall's own inbound request was TLS (its `X-Forwarded-Proto` from the
   operator's reverse proxy, or the `CITYHALL_BASE_URL` scheme); otherwise omit
   the header. Required for the process backend to survive #4743; harmless now.
2. Proxy: drop `set-cookie` values for `aoe_token` and the `x-aoe-token` header
   from upstream responses. Harmless with `--auth none`, mandatory before token
   mode.

**Phase 1, now, flips `--auth none` to `--auth token`, no aoe dependency**

3. Mint a per-workspace Daemon token and store it beside `bundle_token`
   (`api/src/workspaces.rs:211-229`). It cannot reuse `random_token(48)`
   (`api/src/auth.rs:37-43`, 48 mixed-case alphanumerics): the Daemon only adopts
   a `serve.token` of exactly 64 or 32 chars drawn from hex digits or lowercase
   letters (`src/server/token.rs:195-201`). Generate 32 random bytes and
   hex-encode them, the same shape as aoe's `generate_token` (`:187-192`).
4. Deliver it at start: process backend writes `serve.token` under the
   workspace `HOME`; docker and kubernetes pass it through the existing env-file
   and Secret and the image entrypoint writes `serve.token` (0600) before `exec`.
   Image change means a new image build per aoe version, which CityHall already
   does.
5. Launch with `--auth token` (all three backends). Everything else in the
   argument list stays.
6. Proxy: inject `Authorization: Bearer <token>` in `forward_headers`.
   On a 401 from the workspace, re-read the token (or restart the workspace to
   re-deliver it) instead of retrying.
7. Verify against current aoe main: dashboard loads through the proxy, structured
   view WS connects, no `aoe_token` cookie or `x-aoe-token` reaches the browser.

After Phase 1, #100's "no `--auth none`" goal is met and the Daemon sees a
Token holder, which is how ADR 0002 admits CityHall until a Proxy variant exists.

**Phase 2, waits on agent-of-empires#4743 merge and its minor release**

8. docker and kubernetes: add `--dangerously-allow-plaintext-remote`, gated on
   the workspace aoe version being at or above the release carrying #4743 (older
   clap rejects the flag). Document the per-start `DANGEROUS` log line and that
   the hop is protected by network isolation, as today.
9. Optional, later: replace the opt-out with the loopback-bind plus
   TLS-terminating sidecar shape (sanctioned shape 2) if the team wants the
   in-cluster hop encrypted. Decide after seeing real kube usage, it is a
   deployment-shape change (sidecar image, per-workspace cert, proxy dials https).

**Phase 3, waits on aoe decisions (asks below)**

10. Replace the `serve.token` pre-write with aoe's token input once it exists.
11. For the dev tier, follow up the runtime-read UDS suppression in CityHall mode
    before #4886 lands, or the in-workspace `aoe list` breaks.
12. Nothing in #100 waits on the Proxy principal or on #3897. #4476 (web serving
    as a Daemon setting) does not change the launch contract studied here.

## (b) Exact names

- Token mode: `--auth token` (default). No separate setting.
- TLS opt-out: `--dangerously-allow-plaintext-remote` (PR #4743, not yet
  released); persisted as `allow_plaintext_remote` in `serve.launch`. Refusal is
  `426` with `aoe-error-code: tls_required`.
- Configured proxy peers for `--behind-proxy`: not yet named, no ticket. #4743
  states "there is no peer-list setting"; trusted peers are loopback only.
- Proxy principal: not yet named, no ticket (ADR 0002 text only; #3897 is
  `LocalPeer`).
- Operator-supplied Daemon token: not yet named, no ticket.

## (c) aoe-side asks, as draft comments (not posted)

**On #3896 (or #4743 as its implementing PR), ask 1: an input for the bearer in `--auth token`**

> ADR 0002 says CityHall mints the per-workspace bearer and the Daemon "runs in
> token mode", but `--auth token` has no input: `load_or_generate_token`
> (`src/server/token.rs:203-238`) reads `serve.token` only if its mtime is under
> 24h and otherwise generates a new one, and nothing else supplies a value.
> CityHall can pre-write `serve.token` from its image entrypoint as an interim,
> but that leans on an undocumented contract and regenerates after 24h idle.
> Would you accept an explicit input, for example `AOE_SERVE_TOKEN` or
> `--token-file <path>`, honored only with `--auth token`, that pins the value
> for the process lifetime and skips the idle regeneration? Related: a request
> authenticated with that bearer currently gets the token echoed back in
> `Set-Cookie: aoe_token` and `x-aoe-token` (`src/server/auth.rs:691-692,724-726`).
> CityHall will strip both at its proxy; if a Proxy principal lands later, the
> echo should probably not apply to it.

**On #2734, ask 2: the container hop under D1b**

> cityhall#100 follow-up. #4743 trusts forwarded headers from loopback peers only
> and has no configured-peer setting, so CityHall's docker and kubernetes
> workspaces (Daemon on `0.0.0.0` inside the container, proxy peer on the bridge
> or pod network, isolated by NetworkPolicy) will answer 426 once it ships. Two
> shapes fit ADR 0002's "HTTPS, loopback, or inside the workspace container":
> `--dangerously-allow-plaintext-remote` on the workspace bind, or a loopback
> bind with an in-pod TLS sidecar asserting `X-Forwarded-Proto: https`. CityHall
> plans the opt-out first, version-gated, and the sidecar later if the hop
> should be encrypted. Is that the intended reading of D1b for a container peer,
> or would a configured proxy peer (CIDR) under D1's "only configured proxy
> peers may supply them" be acceptable in a later slice?

**On #2734, ask 3: the Proxy variant has no ticket**

> ADR 0002 lists Proxy as one of four Principal variants, but the filed slices
> cover `LocalPeer` only (#3897). CityHall will switch to `--auth token` and be
> admitted as a Token holder, which works for #100. Should a ticket exist for
> the typed Principal with the Proxy variant and the single authorization
> function, so cityhall#13/#15 (the operator principal, #4506) have something to
> attach to, or is Token holder the intended mapping for CityHall indefinitely?

**On #2734, ask 4: runtime-read UDS in CityHall mode versus #4886**

> `publish_runtime_uds` returns `None` in CityHall mode
> (`src/server/startup.rs:1245-1255`), so read-only CLI commands inside a
> CityHall workspace use the disk fallback today. #4886 removes that fallback,
> after which `aoe list` in a workspace shell hard-fails. cityhall#119 targets a
> dev tier with a terminal in the workspace. Could the suppression be lifted
> (the socket is `HOME`-scoped and owner-verified, so CityHall mode does not
> change its trust), or does #3897's convergence onto `api.sock` cover it?

_Note: drafted by Claude via back-and-forth with @Seluj78._
