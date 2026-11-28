//! Workspace policy layer: settings resolution, intent rows, lifecycle entry
//! points shared by the API handlers and the proxy, and the idle-stop sweeper.

use std::time::Duration;

use chrono::Utc;
use sea_orm::sea_query::Expr;
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};

use crate::crypto::Secret;
use crate::entities::{user, workspace, workspace_settings};
use crate::error::AppError;
use crate::orchestrator::{
    bundle_url, render_image, telemetry_policy_override, BundleAccess, OrchestratorError,
    TelemetryPolicy, WorkspaceMode, WorkspaceSpec, WorkspaceStatus,
};
use crate::rbac::Perms;
use crate::state::{AppState, Endpoint, EndpointCache, WorkspaceLocks};

pub const SETTINGS_ID: i32 = 1;
const SWEEP_INTERVAL: Duration = Duration::from_secs(60);

/// Effective workspace settings: the stored row, or defaults when none exists.
pub async fn settings(db: &DatabaseConnection) -> Result<workspace_settings::Model, AppError> {
    Ok(workspace_settings::Entity::find_by_id(SETTINGS_ID)
        .one(db)
        .await?
        .unwrap_or(workspace_settings::Model {
            id: SETTINGS_ID,
            image_template: "cityhall/aoe:{version}".to_string(),
            default_version: None,
            idle_stop_minutes: 30,
            telemetry_policy: TelemetryPolicy::default().as_str().to_string(),
            updated_at: Utc::now(),
            agents: String::new(),
        }))
}

/// The telemetry policy workspaces actually run under: the deployment-level
/// override when one is set, otherwise the stored setting.
///
/// The override's own validity was settled at startup (see
/// [`crate::orchestrator::from_env`]), so an `Err` here cannot happen; reading
/// it as "no override" if it somehow did would be the same lenient direction as
/// [`TelemetryPolicy::from_stored`].
pub fn effective_telemetry_policy(settings: &workspace_settings::Model) -> TelemetryPolicy {
    telemetry_policy_override()
        .ok()
        .flatten()
        .unwrap_or_else(|| TelemetryPolicy::from_stored(&settings.telemetry_policy))
}

/// On a first startup (no settings row saved yet), pre-fill the default
/// version with the latest aoe release so workspaces work out of the box.
/// Best effort: offline or rate-limited lookups just log and skip; a saved
/// row (even with no version) is never touched.
pub async fn seed_default_version(db: &DatabaseConnection) -> Result<(), AppError> {
    if workspace_settings::Entity::find_by_id(SETTINGS_ID)
        .one(db)
        .await?
        .is_some()
    {
        return Ok(());
    }
    let version = match fetch_releases().await.map(|v| v.into_iter().next()) {
        Ok(Some(tag)) => tag,
        Ok(None) => {
            tracing::warn!("no aoe releases found to seed the default workspace version");
            return Ok(());
        }
        Err(e) => {
            tracing::warn!(
                "could not resolve the latest aoe release for the default workspace version: {e}"
            );
            return Ok(());
        }
    };
    tracing::info!(version = %version, "seeding workspace default version from the latest aoe release");
    apply_seeded_version(db, version).await
}

async fn apply_seeded_version(db: &DatabaseConnection, version: String) -> Result<(), AppError> {
    let defaults = settings(db).await?;
    workspace_settings::ActiveModel {
        id: Set(SETTINGS_ID),
        image_template: Set(defaults.image_template),
        default_version: Set(Some(version)),
        idle_stop_minutes: Set(defaults.idle_stop_minutes),
        telemetry_policy: Set(defaults.telemetry_policy),
        updated_at: Set(Utc::now()),
        agents: Set(defaults.agents),
    }
    .insert(db)
    .await?;
    Ok(())
}

/// Stable (non-draft, non-prerelease) agent-of-empires release tags, newest
/// first by version.
async fn fetch_releases() -> Result<Vec<String>, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;
    let mut req = client
        .get("https://api.github.com/repos/agent-of-empires/agent-of-empires/releases?per_page=100")
        // GitHub's API rejects requests without a User-Agent.
        .header("User-Agent", "cityhall");
    // Optional: authenticated requests get a much higher rate limit (the
    // unauthenticated 60/h is shared per source IP).
    if let Ok(token) = std::env::var("GITHUB_TOKEN") {
        if !token.trim().is_empty() {
            req = req.header("Authorization", format!("Bearer {}", token.trim()));
        }
    }
    let body = req
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .text()
        .await
        .map_err(|e| e.to_string())?;
    parse_releases(&body).map_err(|e| e.to_string())
}

/// Parse the GitHub releases payload into stable tags, newest version first.
/// GitHub orders by creation date, which misplaces backported patches
/// (v1.0.1 released after v2.0.0 would come first).
fn parse_releases(body: &str) -> Result<Vec<String>, serde_json::Error> {
    #[derive(serde::Deserialize)]
    struct Release {
        tag_name: String,
        draft: bool,
        prerelease: bool,
    }
    let releases: Vec<Release> = serde_json::from_str(body)?;
    let mut tags: Vec<String> = releases
        .into_iter()
        .filter(|r| !r.draft && !r.prerelease)
        .map(|r| r.tag_name)
        .collect();
    tags.sort_by_key(|t| std::cmp::Reverse(version_key(t)));
    Ok(tags)
}

/// Numeric components of a version tag ("v1.10.2" -> [1, 10, 2]) for
/// ordering; tags without digits compare as empty (never "outdated").
pub fn version_key(tag: &str) -> Vec<u64> {
    tag.split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .map(|s| s.parse().unwrap_or(u64::MAX))
        .collect()
}

/// Cached release discovery: single-flight refresh, 1h TTL, and
/// stale-on-error so a GitHub outage degrades to the last known list
/// instead of an empty dropdown.
#[derive(Default)]
pub struct VersionCache {
    /// Serializes refreshes so concurrent cache misses produce one request.
    refresh: tokio::sync::Mutex<()>,
    state: std::sync::Mutex<VersionCacheState>,
}

#[derive(Default)]
struct VersionCacheState {
    fetched_at: Option<tokio::time::Instant>,
    versions: Vec<String>,
}

const VERSION_CACHE_TTL: Duration = Duration::from_secs(3600);

impl VersionCache {
    /// The cached (or freshly fetched) release tags and whether they are
    /// stale (a refresh failed and an older list is being served).
    pub async fn versions(&self) -> (Vec<String>, bool) {
        if let Some(fresh) = self.fresh() {
            return (fresh, false);
        }
        let _refresh = self.refresh.lock().await;
        // Another caller may have refreshed while this one waited.
        if let Some(fresh) = self.fresh() {
            return (fresh, false);
        }
        match fetch_releases().await {
            Ok(versions) => {
                let mut state = self.state.lock().unwrap();
                state.fetched_at = Some(tokio::time::Instant::now());
                state.versions = versions.clone();
                (versions, false)
            }
            Err(e) => {
                tracing::warn!("release discovery failed, serving the last known list: {e}");
                let state = self.state.lock().unwrap();
                (state.versions.clone(), true)
            }
        }
    }

    fn fresh(&self) -> Option<Vec<String>> {
        let state = self.state.lock().unwrap();
        state
            .fetched_at
            .filter(|at| at.elapsed() < VERSION_CACHE_TTL)
            .map(|_| state.versions.clone())
    }
}

/// The user's workspace intent row, created on first use. Always comes back
/// with a bundle token and a daemon token (see [`ensure_bundle_token`],
/// [`ensure_daemon_token`]).
pub async fn get_or_create(
    db: &DatabaseConnection,
    user_id: i32,
) -> Result<workspace::Model, AppError> {
    if let Some(row) = workspace::Entity::find_by_id(user_id).one(db).await? {
        let row = ensure_bundle_token(db, row).await?;
        return ensure_daemon_token(db, row).await;
    }
    let now = Utc::now();
    Ok(workspace::ActiveModel {
        user_id: Set(user_id),
        pinned_version: Set(None),
        last_active_at: Set(Some(now)),
        created_at: Set(now),
        updated_at: Set(now),
        bundle_token: Set(Some(crate::auth::random_token(48))),
        daemon_token: Set(Some(encrypt_daemon_token(user_id, &mint_daemon_token())?)),
    }
    .insert(db)
    .await?)
}

/// Give a row a bundle token if it has none.
///
/// Rows created before the config bundle existed have `None`, and a workspace
/// without a token cannot fetch its configuration, so it would silently start
/// unconfigured. Backfilled lazily here rather than in the migration so the
/// token is generated by the same RNG path as every other CityHall secret.
/// Written as a conditional update rather than a plain one: two concurrent starts
/// would both see `None` and write different tokens, and whichever workspace
/// started with the loser's token would then be rejected by the bundle endpoint.
/// The `WHERE bundle_token IS NULL` makes exactly one writer win, and the row is
/// re-read afterwards so both callers return the token that actually persisted.
pub async fn ensure_bundle_token(
    db: &DatabaseConnection,
    row: workspace::Model,
) -> Result<workspace::Model, AppError> {
    if row.bundle_token.is_some() {
        return Ok(row);
    }

    let user_id = row.user_id;
    workspace::Entity::update_many()
        .col_expr(
            workspace::Column::BundleToken,
            Expr::value(crate::auth::random_token(48)),
        )
        .filter(workspace::Column::UserId.eq(user_id))
        .filter(workspace::Column::BundleToken.is_null())
        .exec(db)
        .await?;

    workspace::Entity::find_by_id(user_id)
        .one(db)
        .await?
        .ok_or(AppError::NotFound("workspace"))
}

/// Replace a user's bundle token, if they have a workspace row.
///
/// Called when the material the token protects changes (git credential, SSH
/// key) or an admin asks. It does not interrupt a running workspace: the old
/// container keeps running, and the new token reaches it at the next explicit
/// restart or recreation. The old token is refused by the bundle endpoint at
/// once, so a copy of it stops working immediately.
/// A user with no row needs nothing: [`get_or_create`] mints one on first use.
pub async fn rotate_bundle_token(db: &DatabaseConnection, user_id: i32) -> Result<(), AppError> {
    workspace::Entity::update_many()
        .col_expr(
            workspace::Column::BundleToken,
            Expr::value(crate::auth::random_token(48)),
        )
        .filter(workspace::Column::UserId.eq(user_id))
        .exec(db)
        .await?;
    Ok(())
}

/// A fresh daemon token: 32 random bytes as 64 lowercase hex characters. aoe
/// only adopts a `--auth token` value of exactly 64 (or 32) hex or lowercase
/// letters, so `crate::auth::random_token` (mixed-case alphanumeric) does not
/// qualify.
pub fn mint_daemon_token() -> String {
    use std::fmt::Write;
    let bytes: [u8; 32] = rand::random();
    bytes.iter().fold(String::with_capacity(64), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

fn encrypt_daemon_token(user_id: i32, token: &str) -> Result<String, AppError> {
    crate::crypto::encrypt(token, &crate::crypto::Aad::DaemonToken { user_id })
}

/// Give a row a daemon token if it has none. Same shape and reasoning as
/// [`ensure_bundle_token`]: a conditional update so concurrent backfills
/// converge on one persisted value, which is re-read and returned. Diverging
/// would start a workspace under a token CityHall then fails to present.
pub async fn ensure_daemon_token(
    db: &DatabaseConnection,
    row: workspace::Model,
) -> Result<workspace::Model, AppError> {
    if row.daemon_token.is_some() {
        return Ok(row);
    }

    let user_id = row.user_id;
    workspace::Entity::update_many()
        .col_expr(
            workspace::Column::DaemonToken,
            Expr::value(encrypt_daemon_token(user_id, &mint_daemon_token())?),
        )
        .filter(workspace::Column::UserId.eq(user_id))
        .filter(workspace::Column::DaemonToken.is_null())
        .exec(db)
        .await?;

    workspace::Entity::find_by_id(user_id)
        .one(db)
        .await?
        .ok_or(AppError::NotFound("workspace"))
}

/// The plaintext daemon token of a row that has one (see
/// [`ensure_daemon_token`]).
pub fn decrypt_daemon_token(row: &workspace::Model) -> Result<String, AppError> {
    let envelope = row
        .daemon_token
        .as_deref()
        .ok_or(AppError::Internal("workspace has no daemon token"))?;
    crate::crypto::decrypt(
        envelope,
        &crate::crypto::Aad::DaemonToken {
            user_id: row.user_id,
        },
    )
}

/// The aoe version this workspace should run: its pin, or the global default.
pub fn effective_version(
    settings: &workspace_settings::Model,
    row: &workspace::Model,
) -> Option<String> {
    row.pinned_version
        .clone()
        .or_else(|| settings.default_version.clone())
}

/// Stays synchronous and never loads credentials. `ensure_started` returns
/// from `state.endpoints` on almost every proxied request (the proxy calls it
/// per request), so loading and decrypting credentials during spec
/// construction would run a query plus several AES-GCM decryptions on every
/// request for a value used only when a container is actually created.
/// Callers that actually reconcile populate `agent_env` themselves just
/// before invoking the orchestrator.
pub fn build_spec(
    settings: &workspace_settings::Model,
    row: &workspace::Model,
) -> Result<WorkspaceSpec, AppError> {
    let version = effective_version(settings, row).ok_or(AppError::BadRequest(
        "no workspace version configured; set a default version in the workspace settings",
    ))?;
    // No configured internal origin, or a row still awaiting its token, means no
    // bundle: the workspace starts unconfigured, exactly as before the feature.
    let bundle = bundle_url()
        .zip(row.bundle_token.clone())
        .map(|(url, token)| BundleAccess { url, token });
    Ok(WorkspaceSpec {
        user_id: row.user_id,
        image: render_image(&settings.image_template, &version),
        version,
        bundle,
        agent_env: crate::agent_credentials::AgentEnv::default(),
        // Like `agent_env`, decrypted by the reconcile sites, not per request.
        daemon_token: None,
        telemetry: effective_telemetry_policy(settings),
        // Already canonical on the way in, so it is passed through rather than
        // re-derived: the backends compare this exact string against what a
        // running workspace was created with.
        agents: settings.agents.clone(),
        // Resolved from the owner's role after a cache miss; see `resolve_mode`.
        mode: WorkspaceMode::Composer,
        apply_mode_to_running: false,
    })
}

/// The mode a permission set asks for. Full needs `workspaces.full` (admins
/// hold it through `*`); a user with no role gets Composer.
pub fn mode_for_perms(perms: &Perms) -> WorkspaceMode {
    if perms.can("workspaces.full") {
        WorkspaceMode::Full
    } else {
        WorkspaceMode::Composer
    }
}

/// The mode `user_id`'s role asks for, before backend policy.
pub async fn requested_mode(
    db: &DatabaseConnection,
    user_id: i32,
) -> Result<WorkspaceMode, AppError> {
    let Some(user) = user::Entity::find_by_id(user_id).one(db).await? else {
        return Ok(WorkspaceMode::Composer);
    };
    let role = match user.role_id {
        Some(id) => crate::service::find_role_by_id(db, id).await?,
        None => None,
    };
    Ok(mode_for_perms(&Perms::from_role(role.as_ref())))
}

/// The mode a workspace starts in next, and the reason when Full was asked for
/// and the backend refused it (Full only when the backend reports no blocker).
/// A downgrade is never silent; callers surface the reason. A running
/// workspace keeps its current mode until it restarts.
pub fn next_mode(
    requested: WorkspaceMode,
    blocker: Option<&'static str>,
) -> (WorkspaceMode, Option<&'static str>) {
    match (requested, blocker) {
        (WorkspaceMode::Full, Some(reason)) => (WorkspaceMode::Composer, Some(reason)),
        (mode, _) => (mode, None),
    }
}

/// Whether a restart would change the mode: the workspace is running (`Some`)
/// in a mode other than the one it would start in next.
pub fn restart_pending(running: Option<WorkspaceMode>, wanted: WorkspaceMode) -> bool {
    running.is_some_and(|mode| mode != wanted)
}

/// The mode to launch `user_id`'s workspace in, logging a downgrade.
async fn resolve_mode(state: &AppState, user_id: i32) -> Result<WorkspaceMode, AppError> {
    let requested = requested_mode(&state.db, user_id).await?;
    let (mode, note) = next_mode(requested, state.orchestrator.full_mode_blocker());
    if let Some(reason) = note {
        tracing::warn!(
            user_id,
            reason,
            "Full mode unavailable, starting in Composer mode"
        );
    }
    Ok(mode)
}

impl From<OrchestratorError> for AppError {
    fn from(e: OrchestratorError) -> Self {
        match e {
            OrchestratorError::Provisioning(m) => AppError::WorkspaceProvisioning(m),
            other => AppError::WorkspaceUnavailable(other.to_string()),
        }
    }
}

/// Start (or resume) `user_id`'s workspace and return its address and daemon
/// token. This is the
/// request-driven start path: the proxy calls it on every request, the admin
/// start endpoint calls it explicitly. Serialized per user against the sweeper.
///
/// The endpoint cache key deliberately does NOT include the credential
/// fingerprint: a saved credential must not make the next proxied request
/// recreate the container and kill a session the user is actively running.
/// Credentials apply on the next create, which happens via the explicit
/// restart route, an idle stop, or first launch.
pub async fn ensure_started(state: &AppState, user_id: i32) -> Result<Endpoint, AppError> {
    let cfg = settings(&state.db).await?;
    let row = get_or_create(&state.db, user_id).await?;
    let mut spec = build_spec(&cfg, &row)?;

    // Hot path: a cached address means no backend round-trip per request.
    if let Some(endpoint) = state.endpoints.get(user_id, &spec.version, &spec.image) {
        state.activity.touch(user_id);
        return Ok(endpoint);
    }

    let lock = state.locks.lock_for(user_id);
    let _guard = lock.lock().await;
    // Re-check under the lock: a concurrent request may have reconciled.
    if let Some(endpoint) = state.endpoints.get(user_id, &spec.version, &spec.image) {
        state.activity.touch(user_id);
        return Ok(endpoint);
    }
    // The row was read before the lock; a rotation may have landed while this
    // start was queued, and a spec built from the old token would hand the new
    // container a revoked one.
    let row = get_or_create(&state.db, user_id).await?;
    spec.bundle = build_spec(&cfg, &row)?.bundle;
    // Only materialize credentials once a container is actually about to be
    // created or reconciled, not on the cache-hit path above.
    spec.agent_env = crate::agent_credentials::materialize(&state.db, user_id).await?;
    spec.mode = resolve_mode(state, user_id).await?;
    let token = Secret::new(decrypt_daemon_token(&row)?);
    spec.daemon_token = Some(token.clone());
    let addr = state.orchestrator.ensure_started(&spec).await?;
    let endpoint = state.endpoints.put(
        user_id,
        &spec.version,
        &spec.image,
        Endpoint {
            addr,
            token,
            generation: 0,
        },
    );
    state.activity.touch(user_id);
    Ok(endpoint)
}

/// Recreate a RUNNING workspace onto its current effective spec (admin
/// version rollout). Deliberately does not touch activity: a rollout must
/// not grant idle workspaces a fresh idle-stop lease. Stopped workspaces are
/// left stopped; they pick the new version up on their next start.
pub async fn restart_if_running(state: &AppState, user_id: i32) -> Result<(), AppError> {
    let cfg = settings(&state.db).await?;
    let Some(row) = workspace::Entity::find_by_id(user_id)
        .one(&state.db)
        .await?
    else {
        return Ok(());
    };
    // A pre-bundle row would otherwise be recreated without a token, and so
    // without its configuration.
    let row = ensure_bundle_token(&state.db, row).await?;
    let row = ensure_daemon_token(&state.db, row).await?;
    let mut spec = build_spec(&cfg, &row)?;

    let lock = state.locks.lock_for(user_id);
    let _guard = lock.lock().await;
    if !matches!(
        state.orchestrator.status(user_id).await,
        Ok(WorkspaceStatus::Running { .. })
    ) {
        return Ok(());
    }
    // Re-read under the lock, as `ensure_started` does, so a rotation that landed
    // while this restart was queued is not overwritten by a stale token.
    let row = get_or_create(&state.db, user_id).await?;
    spec.bundle = build_spec(&cfg, &row)?.bundle;
    spec.agent_env = crate::agent_credentials::materialize(&state.db, user_id).await?;
    spec.mode = resolve_mode(state, user_id).await?;
    // An explicit restart is where a changed mode reaches a running workspace.
    spec.apply_mode_to_running = true;
    state.endpoints.invalidate(user_id);
    let token = Secret::new(decrypt_daemon_token(&row)?);
    spec.daemon_token = Some(token.clone());
    let addr = state.orchestrator.ensure_started(&spec).await?;
    state.endpoints.put(
        user_id,
        &spec.version,
        &spec.image,
        Endpoint {
            addr,
            token,
            generation: 0,
        },
    );
    Ok(())
}

/// Stop the workspace (volume kept), checkpointing the activity time.
pub async fn stop(state: &AppState, user_id: i32) -> Result<(), AppError> {
    let lock = state.locks.lock_for(user_id);
    let _guard = lock.lock().await;
    state.endpoints.invalidate(user_id);
    state.orchestrator.stop(user_id).await?;
    checkpoint_activity(state, user_id).await
}

/// Stop the workspace only if its cached endpoint is still `generation`.
/// Returns whether it stopped. The check and the stop run under one per-user
/// lock, so a request that started a healthy replacement in between is never
/// stopped by a late 401 about the obsolete runtime.
pub async fn stop_if_generation(
    state: &AppState,
    user_id: i32,
    generation: u64,
) -> Result<bool, AppError> {
    stop_if_generation_with(
        &state.endpoints,
        &state.locks,
        user_id,
        generation,
        || async {
            state.orchestrator.stop(user_id).await?;
            checkpoint_activity(state, user_id).await
        },
    )
    .await
}

/// The decision behind `stop_if_generation`, with the stop injected.
async fn stop_if_generation_with<F, Fut>(
    endpoints: &EndpointCache,
    locks: &WorkspaceLocks,
    user_id: i32,
    generation: u64,
    stop: F,
) -> Result<bool, AppError>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<(), AppError>>,
{
    let lock = locks.lock_for(user_id);
    let _guard = lock.lock().await;
    if !endpoints.invalidate_if(user_id, generation) {
        return Ok(false);
    }
    stop().await?;
    Ok(true)
}

/// Destroy the workspace and its volume, dropping the intent row.
pub async fn destroy(state: &AppState, user_id: i32) -> Result<(), AppError> {
    let lock = state.locks.lock_for(user_id);
    let _guard = lock.lock().await;
    state.endpoints.invalidate(user_id);
    state.orchestrator.destroy(user_id).await?;
    workspace::Entity::delete_by_id(user_id)
        .exec(&state.db)
        .await?;
    Ok(())
}

/// Write the in-memory activity time (when known) to the row so it survives
/// restarts and shows up in the admin UI.
async fn checkpoint_activity(state: &AppState, user_id: i32) -> Result<(), AppError> {
    let Some(row) = workspace::Entity::find_by_id(user_id)
        .one(&state.db)
        .await?
    else {
        return Ok(());
    };
    let last_active = state
        .activity
        .get(user_id)
        .map(|e| Utc::now() - chrono::Duration::from_std(e.last_seen.elapsed()).unwrap_or_default())
        .unwrap_or_else(Utc::now);
    let mut active: workspace::ActiveModel = row.into();
    active.last_active_at = Set(Some(last_active));
    active.updated_at = Set(Utc::now());
    active.update(&state.db).await?;
    Ok(())
}

/// Background loop stopping workspaces idle past the configured threshold.
/// Workspaces found running without any in-memory activity (e.g. right after a
/// CityHall restart) get a grace entry instead of an immediate stop.
pub async fn idle_sweeper(state: AppState) {
    let mut interval = tokio::time::interval(SWEEP_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        interval.tick().await;
        if let Err(e) = sweep_once(&state).await {
            tracing::warn!("idle sweep failed: {e}");
        }
    }
}

async fn sweep_once(state: &AppState) -> Result<(), AppError> {
    let cfg = settings(&state.db).await?;
    let idle_after = Duration::from_secs(cfg.idle_stop_minutes.max(1) as u64 * 60);

    for row in workspace::Entity::find().all(&state.db).await? {
        let user_id = row.user_id;
        let entry = state.activity.get(user_id);
        match entry {
            None => {
                // No in-memory record: if it is running (fresh restart), start
                // the idle clock now rather than killing an active session.
                if matches!(
                    state.orchestrator.status(user_id).await,
                    Ok(WorkspaceStatus::Running { .. })
                ) {
                    state.activity.touch(user_id);
                }
            }
            Some(entry) if entry.active_websockets > 0 => {}
            Some(entry) if entry.last_seen.elapsed() >= idle_after => {
                let lock = state.locks.lock_for(user_id);
                let _guard = lock.lock().await;
                // Re-check under the lock: a proxy request may have just
                // touched or restarted the workspace.
                let still_idle = state
                    .activity
                    .get(user_id)
                    .map(|e| e.active_websockets == 0 && e.last_seen.elapsed() >= idle_after)
                    .unwrap_or(false);
                if !still_idle {
                    continue;
                }
                if let Ok(WorkspaceStatus::Running { .. }) =
                    state.orchestrator.status(user_id).await
                {
                    tracing::info!(user_id, "stopping idle workspace");
                    state.endpoints.invalidate(user_id);
                    if let Err(e) = state.orchestrator.stop(user_id).await {
                        tracing::warn!(user_id, "idle stop failed: {e}");
                        continue;
                    }
                    checkpoint_activity(state, user_id).await?;
                }
            }
            Some(_) => {
                checkpoint_activity(state, user_id).await?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::role;
    use crate::migration::Migrator;
    use base64::engine::general_purpose::STANDARD as B64;
    use base64::Engine;
    use sea_orm::{ConnectOptions, Database};
    use sea_orm_migration::MigratorTrait;
    use std::sync::Arc;

    /// Capped at one connection on purpose. Every connection to
    /// `sqlite::memory:` gets its own empty database, so a pool that opened a
    /// second one would hand a concurrent test an unmigrated schema. Today's
    /// driver already serializes on a single connection (8 tasks holding
    /// transactions for 120ms each take 960ms, not 120ms), but that is behavior
    /// observed rather than promised, and `a_concurrent_backfill_agrees_on_one_token`
    /// depends on it.
    /// The daemon token is encrypted, so any test that creates a workspace row
    /// needs a key. Held for the whole test: the env var is process-wide.
    fn test_key() -> crate::crypto::KeyEnvGuard {
        let guard = crate::crypto::guard_key_env();
        std::env::set_var("CITYHALL_SECRET_KEY", B64.encode([9u8; 32]));
        std::env::remove_var("CITYHALL_SECRET_KEY_PREVIOUS");
        guard
    }

    async fn setup() -> DatabaseConnection {
        let mut opts = ConnectOptions::new("sqlite::memory:");
        opts.max_connections(1);
        let db = Database::connect(opts).await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        db
    }

    /// Create a user to hang a workspace row off. The password is generated
    /// rather than written inline: these tests never authenticate, and a literal
    /// here is a hard-coded-credential finding for no benefit.
    async fn make_user(db: &DatabaseConnection) -> i32 {
        crate::service::create(db, "u", None, &crate::auth::random_token(24), false, None)
            .await
            .unwrap()
            .id
    }

    fn cfg(default_version: Option<&str>) -> workspace_settings::Model {
        workspace_settings::Model {
            id: SETTINGS_ID,
            image_template: "cityhall/aoe:{version}".to_string(),
            default_version: default_version.map(String::from),
            idle_stop_minutes: 30,
            telemetry_policy: TelemetryPolicy::default().as_str().to_string(),
            updated_at: Utc::now(),
            agents: String::new(),
        }
    }

    #[tokio::test]
    async fn a_stale_generation_never_stops_the_replacement() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let cache = EndpointCache::default();
        let locks = WorkspaceLocks::default();
        let stops = AtomicUsize::new(0);
        let ep = || Endpoint {
            addr: "svc:1".to_string(),
            token: Secret::new("tok".to_string()),
            generation: 0,
        };
        let run = |generation| {
            stop_if_generation_with(&cache, &locks, 1, generation, || async {
                stops.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        };

        let g1 = cache.put(1, "v", "i", ep()).generation;
        // Request B replaced the entry (same address) before A's stop ran.
        let g2 = cache.put(1, "v", "i", ep()).generation;
        assert_ne!(g1, g2);

        assert!(!run(g1).await.unwrap());
        assert_eq!(stops.load(Ordering::SeqCst), 0);
        assert!(cache.get(1, "v", "i").is_some());

        assert!(run(g2).await.unwrap());
        assert_eq!(stops.load(Ordering::SeqCst), 1);
        assert!(cache.get(1, "v", "i").is_none());

        assert!(!run(g2).await.unwrap());
        assert_eq!(stops.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn seeded_version_fills_a_fresh_install() {
        let _key = test_key();
        let db = setup().await;
        apply_seeded_version(&db, "v9.9.9".to_string())
            .await
            .unwrap();
        let s = settings(&db).await.unwrap();
        assert_eq!(s.default_version.as_deref(), Some("v9.9.9"));
    }

    #[tokio::test]
    async fn seeding_never_touches_a_saved_row() {
        let _key = test_key();
        let db = setup().await;
        // Operator saved settings without a version; the seeder must leave
        // that choice alone (and must not hit the network to do so).
        workspace_settings::ActiveModel {
            id: Set(SETTINGS_ID),
            image_template: Set("cityhall/aoe:{version}".to_string()),
            default_version: Set(None),
            idle_stop_minutes: Set(30),
            telemetry_policy: Set(TelemetryPolicy::default().as_str().to_string()),
            updated_at: Set(Utc::now()),
            agents: Set(String::new()),
        }
        .insert(&db)
        .await
        .unwrap();

        seed_default_version(&db).await.unwrap();
        assert_eq!(settings(&db).await.unwrap().default_version, None);
    }

    #[test]
    fn version_keys_order_numerically() {
        assert!(version_key("v1.10.0") > version_key("v1.9.9"));
        assert!(version_key("v2.0.0") > version_key("v1.99.99"));
        assert_eq!(version_key("1.2.3"), version_key("v1.2.3"));
        // Non-numeric tags compare as empty: never flagged outdated.
        assert!(version_key("custom-build").is_empty());
    }

    #[test]
    fn release_parsing_filters_and_sorts() {
        let body = r#"[
            {"tag_name": "v1.9.0", "draft": false, "prerelease": false},
            {"tag_name": "v2.0.0-rc.1", "draft": false, "prerelease": true},
            {"tag_name": "v1.10.0", "draft": false, "prerelease": false},
            {"tag_name": "v3.0.0", "draft": true, "prerelease": false}
        ]"#;
        // Drafts and prereleases are dropped; order is by version, not the
        // API's creation-date order.
        assert_eq!(parse_releases(body).unwrap(), vec!["v1.10.0", "v1.9.0"]);
    }

    #[tokio::test]
    async fn settings_defaults_when_no_row() {
        let _key = test_key();
        let db = setup().await;
        let s = settings(&db).await.unwrap();
        assert_eq!(s.image_template, "cityhall/aoe:{version}");
        assert_eq!(s.idle_stop_minutes, 30);
        // No agents by default: an install that has never configured this keeps
        // shipping a workspace the user installs their own agent into.
        assert_eq!(s.agents, "");
    }

    /// The workspace has to be told which agents to arrive with, and it is told
    /// through the spec, so a settings value that stopped reaching it here would
    /// silently turn the whole feature off.
    #[tokio::test]
    async fn spec_carries_the_configured_agents() {
        let _key = test_key();
        let db = setup().await;
        let uid = make_user(&db).await;
        let row = get_or_create(&db, uid).await.unwrap();
        assert_eq!(build_spec(&cfg(Some("v1.0.0")), &row).unwrap().agents, "");

        let mut with_agents = cfg(Some("v1.0.0"));
        with_agents.agents = "claude,codex".to_string();
        assert_eq!(
            build_spec(&with_agents, &row).unwrap().agents,
            "claude,codex"
        );
    }

    #[tokio::test]
    async fn get_or_create_is_idempotent() {
        let _key = test_key();
        let db = setup().await;
        let uid = make_user(&db).await;
        let a = get_or_create(&db, uid).await.unwrap();
        let b = get_or_create(&db, uid).await.unwrap();
        assert_eq!(a.user_id, b.user_id);
        assert_eq!(a.created_at, b.created_at);
    }

    /// Without a token a workspace cannot fetch its configuration, so it would
    /// silently boot unconfigured.
    #[tokio::test]
    async fn a_fresh_row_gets_a_bundle_token() {
        let _key = test_key();
        let db = setup().await;
        let uid = make_user(&db).await;
        let row = get_or_create(&db, uid).await.unwrap();
        assert!(row.bundle_token.is_some());
    }

    /// Rows created before the bundle existed have `None`; they must be
    /// backfilled rather than left unable to fetch anything.
    #[tokio::test]
    async fn a_pre_bundle_row_is_backfilled() {
        let _key = test_key();
        let db = setup().await;
        let uid = make_user(&db).await;
        let row = get_or_create(&db, uid).await.unwrap();
        let mut active: workspace::ActiveModel = row.into();
        active.bundle_token = Set(None);
        let stripped = active.update(&db).await.unwrap();
        assert!(stripped.bundle_token.is_none());

        let restored = ensure_bundle_token(&db, stripped).await.unwrap();
        let token = restored.bundle_token.expect("backfilled");
        assert!(!token.is_empty());
        // Stable across calls: rotating on every start would invalidate the
        // token a running workspace already holds.
        let again = get_or_create(&db, uid).await.unwrap();
        assert_eq!(again.bundle_token.as_deref(), Some(token.as_str()));
    }

    /// A rotated token replaces the old one, which the bundle endpoint (an
    /// equality lookup on the column) then no longer finds. Without a rotation
    /// the token is stable, so a running workspace's copy stays valid.
    #[tokio::test]
    async fn rotating_replaces_the_token_and_only_then() {
        let _key = test_key();
        let db = setup().await;
        let uid = make_user(&db).await;
        let old = get_or_create(&db, uid).await.unwrap().bundle_token.unwrap();
        assert_eq!(
            get_or_create(&db, uid)
                .await
                .unwrap()
                .bundle_token
                .as_deref(),
            Some(old.as_str())
        );

        rotate_bundle_token(&db, uid).await.unwrap();

        let new = get_or_create(&db, uid).await.unwrap().bundle_token.unwrap();
        assert_ne!(new, old);
        let by_token = |t: String| {
            workspace::Entity::find()
                .filter(workspace::Column::BundleToken.eq(t))
                .one(&db)
        };
        assert!(by_token(old).await.unwrap().is_none());
        assert!(by_token(new).await.unwrap().is_some());
    }

    /// Rotating for a user with no workspace row is a no-op, not an error.
    #[tokio::test]
    async fn rotating_without_a_row_is_a_no_op() {
        let _key = test_key();
        let db = setup().await;
        let uid = make_user(&db).await;
        rotate_bundle_token(&db, uid).await.unwrap();
        assert!(workspace::Entity::find_by_id(uid)
            .one(&db)
            .await
            .unwrap()
            .is_none());
    }

    /// Two starts racing to backfill must agree on one token. A caller that
    /// returned the token it generated rather than the one that persisted would
    /// hand a workspace a token the bundle endpoint rejects.
    #[tokio::test]
    async fn a_concurrent_backfill_agrees_on_one_token() {
        let _key = test_key();
        let db = setup().await;
        let uid = make_user(&db).await;
        let row = get_or_create(&db, uid).await.unwrap();
        let mut active: workspace::ActiveModel = row.into();
        active.bundle_token = Set(None);
        let stripped = active.update(&db).await.unwrap();

        // Both callers hold the same tokenless snapshot, which is exactly what
        // two starts observing the row at the same moment would see.
        let (a, b) = tokio::join!(
            ensure_bundle_token(&db, stripped.clone()),
            ensure_bundle_token(&db, stripped)
        );
        let (a, b) = (a.unwrap(), b.unwrap());

        let persisted = workspace::Entity::find_by_id(uid)
            .one(&db)
            .await
            .unwrap()
            .unwrap()
            .bundle_token
            .expect("backfilled");
        assert_eq!(a.bundle_token.as_deref(), Some(persisted.as_str()));
        assert_eq!(b.bundle_token.as_deref(), Some(persisted.as_str()));
    }

    fn is_64_lowercase_hex(s: &str) -> bool {
        s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    }

    #[test]
    fn a_minted_daemon_token_is_64_lowercase_hex() {
        let a = mint_daemon_token();
        assert!(is_64_lowercase_hex(&a), "{a}");
        assert_ne!(a, mint_daemon_token());
    }

    /// The row holds an envelope, never the token, and it opens only for its
    /// owner.
    #[tokio::test]
    async fn a_fresh_row_gets_an_encrypted_daemon_token() {
        let _key = test_key();
        let db = setup().await;
        let uid = make_user(&db).await;
        let row = get_or_create(&db, uid).await.unwrap();
        let stored = row.daemon_token.clone().expect("minted");
        let plain = decrypt_daemon_token(&row).unwrap();
        assert!(is_64_lowercase_hex(&plain));
        assert!(!stored.contains(&plain));

        let other = crate::crypto::Aad::DaemonToken { user_id: uid + 1 };
        assert!(crate::crypto::decrypt(&stored, &other).is_err());
    }

    #[tokio::test]
    async fn a_pre_feature_row_gets_a_stable_daemon_token() {
        let _key = test_key();
        let db = setup().await;
        let uid = make_user(&db).await;
        let row = get_or_create(&db, uid).await.unwrap();
        let mut active: workspace::ActiveModel = row.into();
        active.daemon_token = Set(None);
        let stripped = active.update(&db).await.unwrap();
        assert!(decrypt_daemon_token(&stripped).is_err());

        let restored = ensure_daemon_token(&db, stripped).await.unwrap();
        let token = decrypt_daemon_token(&restored).unwrap();
        assert!(is_64_lowercase_hex(&token));
        // Rotating on every start would break the token a running workspace
        // already serves under.
        let again = get_or_create(&db, uid).await.unwrap();
        assert_eq!(decrypt_daemon_token(&again).unwrap(), token);
    }

    #[tokio::test]
    async fn a_concurrent_daemon_token_backfill_agrees_on_one_value() {
        let _key = test_key();
        let db = setup().await;
        let uid = make_user(&db).await;
        let row = get_or_create(&db, uid).await.unwrap();
        let mut active: workspace::ActiveModel = row.into();
        active.daemon_token = Set(None);
        let stripped = active.update(&db).await.unwrap();

        let (a, b) = tokio::join!(
            ensure_daemon_token(&db, stripped.clone()),
            ensure_daemon_token(&db, stripped)
        );
        let (a, b) = (a.unwrap(), b.unwrap());

        let persisted = workspace::Entity::find_by_id(uid)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        let want = decrypt_daemon_token(&persisted).unwrap();
        assert_eq!(decrypt_daemon_token(&a).unwrap(), want);
        assert_eq!(decrypt_daemon_token(&b).unwrap(), want);
    }

    #[test]
    fn debug_output_never_contains_the_daemon_token() {
        let token = "f".repeat(64);
        let secret = Secret::new(token.clone());
        let endpoint = Endpoint {
            addr: "127.0.0.1:1".to_string(),
            token: secret.clone(),
            generation: 0,
        };
        assert!(!format!("{endpoint:?}").contains(&token));
        let spec = WorkspaceSpec {
            user_id: 1,
            image: "i".to_string(),
            version: "v".to_string(),
            bundle: None,
            agent_env: crate::agent_credentials::AgentEnv::default(),
            daemon_token: Some(secret),
            telemetry: TelemetryPolicy::default(),
            agents: String::new(),
            mode: WorkspaceMode::default(),
            apply_mode_to_running: false,
        };
        assert!(!format!("{spec:?}").contains(&token));
    }

    #[tokio::test]
    async fn spec_uses_pin_over_default() {
        let _key = test_key();
        let db = setup().await;
        let uid = make_user(&db).await;
        let row = get_or_create(&db, uid).await.unwrap();
        let spec = build_spec(&cfg(Some("v1.0.0")), &row).unwrap();
        assert_eq!(spec.image, "cityhall/aoe:v1.0.0");

        let mut active: workspace::ActiveModel = row.into();
        active.pinned_version = Set(Some("v2.0.0".to_string()));
        let row = active.update(&db).await.unwrap();
        let spec = build_spec(&cfg(Some("v1.0.0")), &row).unwrap();
        assert_eq!(spec.version, "v2.0.0");
        assert_eq!(spec.image, "cityhall/aoe:v2.0.0");
    }

    #[tokio::test]
    async fn spec_requires_some_version() {
        let _key = test_key();
        let db = setup().await;
        let uid = make_user(&db).await;
        let row = get_or_create(&db, uid).await.unwrap();
        assert!(build_spec(&cfg(None), &row).is_err());
    }

    /// `build_spec` must stay synchronous and must never load credentials:
    /// `ensure_started` calls it on the hot cache-hit path taken by almost
    /// every proxied request, so a query here would run per request for a
    /// value only needed when a container is actually created.
    #[tokio::test]
    async fn build_spec_does_not_hit_the_db_for_credentials() {
        let _key = test_key();
        let db = setup().await;
        let uid = make_user(&db).await;
        let row = get_or_create(&db, uid).await.unwrap();
        let spec = build_spec(&cfg(Some("v1.0.0")), &row).unwrap();
        assert_eq!(
            spec.agent_env,
            crate::agent_credentials::AgentEnv::default()
        );
        assert_eq!(spec.agent_env.fingerprint, "");
        assert!(spec.agent_env.pairs.is_empty());
    }
    async fn user_with_perms(db: &DatabaseConnection, name: &str, perms: &[&str]) -> i32 {
        let keys: Vec<String> = perms.iter().map(|s| s.to_string()).collect();
        let role = role::ActiveModel {
            name: Set(name.to_string()),
            description: Set(None),
            permissions: Set(crate::rbac::encode(&keys)),
            is_system: Set(false),
            created_at: Set(Utc::now()),
            ..Default::default()
        }
        .insert(db)
        .await
        .unwrap();
        crate::service::create(
            db,
            name,
            None,
            &crate::auth::random_token(24),
            false,
            Some(role.id),
        )
        .await
        .unwrap()
        .id
    }

    /// Full needs `workspaces.full` on the owner's role; `*` grants it, and a
    /// user with no role (or no row) is Composer.
    #[tokio::test]
    async fn the_mode_follows_the_owners_role() {
        let db = setup().await;
        let dev = user_with_perms(&db, "dev", &["workspaces.use", "workspaces.full"]).await;
        let pm = user_with_perms(&db, "pm", &["workspaces.use"]).await;
        let admin = user_with_perms(&db, "root", &["*"]).await;
        let roleless = make_user(&db).await;
        assert_eq!(requested_mode(&db, dev).await.unwrap(), WorkspaceMode::Full);
        assert_eq!(
            requested_mode(&db, pm).await.unwrap(),
            WorkspaceMode::Composer
        );
        assert_eq!(
            requested_mode(&db, admin).await.unwrap(),
            WorkspaceMode::Full
        );
        assert_eq!(
            requested_mode(&db, roleless).await.unwrap(),
            WorkspaceMode::Composer
        );
        assert_eq!(
            requested_mode(&db, 9999).await.unwrap(),
            WorkspaceMode::Composer
        );
    }

    /// A blocker turns a Full request into Composer and never the reverse.
    #[test]
    fn the_note_names_a_blocker_only_when_full_was_denied() {
        use WorkspaceMode::{Composer, Full};
        assert_eq!(next_mode(Full, Some("why")), (Composer, Some("why")));
        assert_eq!(next_mode(Full, None), (Full, None));
        assert_eq!(next_mode(Composer, Some("why")), (Composer, None));
    }

    /// `build_spec` runs on every proxied request and must not read the role.
    #[tokio::test]
    async fn build_spec_defaults_to_composer() {
        let db = setup().await;
        let uid = make_user(&db).await;
        let row = get_or_create(&db, uid).await.unwrap();
        let spec = build_spec(&cfg(Some("v1.0.0")), &row).unwrap();
        assert_eq!(spec.mode, WorkspaceMode::Composer);
    }

    /// Records the spec each `ensure_started` receives.
    #[derive(Default)]
    struct Recorder {
        specs: std::sync::Mutex<Vec<WorkspaceSpec>>,
        blocker: Option<&'static str>,
        running: bool,
    }

    #[async_trait::async_trait]
    impl crate::orchestrator::Orchestrator for Recorder {
        async fn ensure_started(&self, spec: &WorkspaceSpec) -> Result<String, OrchestratorError> {
            self.specs.lock().unwrap().push(spec.clone());
            Ok("127.0.0.1:1".to_string())
        }
        fn full_mode_blocker(&self) -> Option<&'static str> {
            self.blocker
        }
        async fn stop(&self, _: i32) -> Result<(), OrchestratorError> {
            Ok(())
        }
        async fn destroy(&self, _: i32) -> Result<(), OrchestratorError> {
            Ok(())
        }
        async fn status(&self, _: i32) -> Result<WorkspaceStatus, OrchestratorError> {
            Ok(if self.running {
                WorkspaceStatus::Running {
                    addr: "127.0.0.1:1".to_string(),
                }
            } else {
                WorkspaceStatus::Stopped
            })
        }
        async fn running_mode(&self, _: i32) -> Result<Option<WorkspaceMode>, OrchestratorError> {
            Ok(self
                .running
                .then(|| self.specs.lock().unwrap().last().map(|s| s.mode))
                .flatten())
        }
    }

    async fn state_with(db: DatabaseConnection, orch: Arc<Recorder>) -> AppState {
        workspace_settings::ActiveModel {
            id: Set(SETTINGS_ID),
            image_template: Set("cityhall/aoe:{version}".to_string()),
            default_version: Set(Some("v1.0.0".to_string())),
            idle_stop_minutes: Set(30),
            telemetry_policy: Set(TelemetryPolicy::default().as_str().to_string()),
            updated_at: Set(Utc::now()),
            agents: Set(String::new()),
        }
        .insert(&db)
        .await
        .unwrap();
        AppState {
            db,
            orchestrator: orch,
            activity: Arc::default(),
            locks: Arc::default(),
            endpoints: Arc::default(),
            provisioning: Arc::default(),
            versions: Arc::default(),
            metrics: Arc::default(),
            session_activity: Arc::default(),
            proxy_client: reqwest::Client::new(),
        }
    }

    /// The owner's role reaches the spec the backend sees: Full for a role with
    /// the key, Composer without it or when the backend refuses Full. Only an
    /// explicit restart asks the backend to apply a changed mode to a running
    /// workspace.
    #[tokio::test]
    async fn ensure_started_gives_the_backend_the_owners_mode() {
        let db = setup().await;
        let dev = user_with_perms(&db, "dev", &["workspaces.use", "workspaces.full"]).await;
        let pm = user_with_perms(&db, "pm", &["workspaces.use"]).await;
        let orch = Arc::new(Recorder {
            running: true,
            ..Default::default()
        });
        let state = state_with(db, orch.clone()).await;

        ensure_started(&state, dev).await.unwrap();
        ensure_started(&state, pm).await.unwrap();
        restart_if_running(&state, dev).await.unwrap();
        let specs = orch.specs.lock().unwrap().clone();
        let seen: Vec<_> = specs
            .iter()
            .map(|s| (s.user_id, s.mode, s.apply_mode_to_running))
            .collect();
        assert_eq!(
            seen,
            vec![
                (dev, WorkspaceMode::Full, false),
                (pm, WorkspaceMode::Composer, false),
                (dev, WorkspaceMode::Full, true),
            ]
        );
    }

    #[tokio::test]
    async fn a_backend_blocker_makes_the_spec_composer() {
        let db = setup().await;
        let dev = user_with_perms(&db, "dev", &["workspaces.use", "workspaces.full"]).await;
        let orch = Arc::new(Recorder {
            blocker: Some("why"),
            ..Default::default()
        });
        let state = state_with(db, orch.clone()).await;
        ensure_started(&state, dev).await.unwrap();
        assert_eq!(orch.specs.lock().unwrap()[0].mode, WorkspaceMode::Composer);
    }

    #[test]
    fn restart_pending_needs_a_running_workspace_in_another_mode() {
        use WorkspaceMode::{Composer, Full};
        assert!(!restart_pending(None, Full));
        assert!(!restart_pending(Some(Full), Full));
        assert!(!restart_pending(Some(Composer), Composer));
        assert!(restart_pending(Some(Composer), Full));
        assert!(restart_pending(Some(Full), Composer));
    }

    /// Drives the same comparison the list and `/me` handlers make: the mode
    /// the backend reports running against the owner's current requested mode.
    async fn pending(state: &AppState, user_id: i32) -> bool {
        let wanted = requested_mode(&state.db, user_id).await.unwrap();
        let running = state.orchestrator.running_mode(user_id).await.unwrap();
        restart_pending(running, wanted)
    }

    #[tokio::test]
    async fn a_role_change_is_pending_until_the_workspace_restarts() {
        let _key = test_key();
        let db = setup().await;
        let uid = user_with_perms(&db, "dev", &["workspaces.use"]).await;
        let orch = Arc::new(Recorder {
            running: true,
            ..Default::default()
        });
        let state = state_with(db, orch).await;
        ensure_started(&state, uid).await.unwrap();
        assert!(!pending(&state, uid).await);

        let user = user::Entity::find_by_id(uid)
            .one(&state.db)
            .await
            .unwrap()
            .unwrap();
        let role = role::Entity::find_by_id(user.role_id.unwrap())
            .one(&state.db)
            .await
            .unwrap()
            .unwrap();
        let mut role: role::ActiveModel = role.into();
        role.permissions = Set(crate::rbac::encode(&[
            "workspaces.use".to_string(),
            "workspaces.full".to_string(),
        ]));
        role.update(&state.db).await.unwrap();
        assert!(pending(&state, uid).await);

        restart_if_running(&state, uid).await.unwrap();
        assert!(!pending(&state, uid).await);
    }

    #[tokio::test]
    async fn nothing_is_pending_for_a_stopped_workspace() {
        let db = setup().await;
        let uid = user_with_perms(&db, "dev", &["workspaces.use", "workspaces.full"]).await;
        let state = state_with(db, Arc::new(Recorder::default())).await;
        ensure_started(&state, uid).await.unwrap();
        assert!(!pending(&state, uid).await);
    }
}
