//! Workspace policy layer: settings resolution, intent rows, lifecycle entry
//! points shared by the API handlers and the proxy, and the idle-stop sweeper.

use std::time::Duration;

use chrono::Utc;
use sea_orm::sea_query::Expr;
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};

use crate::entities::{workspace, workspace_settings};
use crate::error::AppError;
use crate::orchestrator::{
    bundle_url, render_image, telemetry_policy_override, BundleAccess, OrchestratorError,
    TelemetryPolicy, WorkspaceSpec, WorkspaceStatus,
};
use crate::state::AppState;

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
/// with a bundle token (see [`ensure_bundle_token`]).
pub async fn get_or_create(
    db: &DatabaseConnection,
    user_id: i32,
) -> Result<workspace::Model, AppError> {
    if let Some(row) = workspace::Entity::find_by_id(user_id).one(db).await? {
        return ensure_bundle_token(db, row).await;
    }
    let now = Utc::now();
    Ok(workspace::ActiveModel {
        user_id: Set(user_id),
        pinned_version: Set(None),
        last_active_at: Set(Some(now)),
        created_at: Set(now),
        updated_at: Set(now),
        bundle_token: Set(Some(crate::auth::random_token(48))),
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
        telemetry: effective_telemetry_policy(settings),
        // Already canonical on the way in, so it is passed through rather than
        // re-derived: the backends compare this exact string against what a
        // running workspace was created with.
        agents: settings.agents.clone(),
    })
}

impl From<OrchestratorError> for AppError {
    fn from(e: OrchestratorError) -> Self {
        match e {
            OrchestratorError::Provisioning(m) => AppError::WorkspaceProvisioning(m),
            other => AppError::WorkspaceUnavailable(other.to_string()),
        }
    }
}

/// Start (or resume) `user_id`'s workspace and return its address. This is the
/// request-driven start path: the proxy calls it on every request, the admin
/// start endpoint calls it explicitly. Serialized per user against the sweeper.
///
/// The endpoint cache key deliberately does NOT include the credential
/// fingerprint: a saved credential must not make the next proxied request
/// recreate the container and kill a session the user is actively running.
/// Credentials apply on the next create, which happens via the explicit
/// restart route, an idle stop, or first launch.
pub async fn ensure_started(state: &AppState, user_id: i32) -> Result<String, AppError> {
    let cfg = settings(&state.db).await?;
    let row = get_or_create(&state.db, user_id).await?;
    let mut spec = build_spec(&cfg, &row)?;

    // Hot path: a cached address means no backend round-trip per request.
    if let Some(addr) = state.endpoints.get(user_id, &spec.version, &spec.image) {
        state.activity.touch(user_id);
        return Ok(addr);
    }

    let lock = state.locks.lock_for(user_id);
    let _guard = lock.lock().await;
    // Re-check under the lock: a concurrent request may have reconciled.
    if let Some(addr) = state.endpoints.get(user_id, &spec.version, &spec.image) {
        state.activity.touch(user_id);
        return Ok(addr);
    }
    // Only materialize credentials once a container is actually about to be
    // created or reconciled, not on the cache-hit path above.
    spec.agent_env = crate::agent_credentials::materialize(&state.db, user_id).await?;
    let addr = state.orchestrator.ensure_started(&spec).await?;
    state
        .endpoints
        .put(user_id, &spec.version, &spec.image, addr.clone());
    state.activity.touch(user_id);
    Ok(addr)
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
    let mut spec = build_spec(&cfg, &row)?;

    let lock = state.locks.lock_for(user_id);
    let _guard = lock.lock().await;
    if !matches!(
        state.orchestrator.status(user_id).await,
        Ok(WorkspaceStatus::Running { .. })
    ) {
        return Ok(());
    }
    spec.agent_env = crate::agent_credentials::materialize(&state.db, user_id).await?;
    state.endpoints.invalidate(user_id);
    let addr = state.orchestrator.ensure_started(&spec).await?;
    state
        .endpoints
        .put(user_id, &spec.version, &spec.image, addr);
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
    use crate::migration::Migrator;
    use sea_orm::{ConnectOptions, Database};
    use sea_orm_migration::MigratorTrait;

    /// Capped at one connection on purpose. Every connection to
    /// `sqlite::memory:` gets its own empty database, so a pool that opened a
    /// second one would hand a concurrent test an unmigrated schema. Today's
    /// driver already serializes on a single connection (8 tasks holding
    /// transactions for 120ms each take 960ms, not 120ms), but that is behavior
    /// observed rather than promised, and `a_concurrent_backfill_agrees_on_one_token`
    /// depends on it.
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
    async fn seeded_version_fills_a_fresh_install() {
        let db = setup().await;
        apply_seeded_version(&db, "v9.9.9".to_string())
            .await
            .unwrap();
        let s = settings(&db).await.unwrap();
        assert_eq!(s.default_version.as_deref(), Some("v9.9.9"));
    }

    #[tokio::test]
    async fn seeding_never_touches_a_saved_row() {
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
        let db = setup().await;
        let uid = make_user(&db).await;
        let row = get_or_create(&db, uid).await.unwrap();
        assert!(row.bundle_token.is_some());
    }

    /// Rows created before the bundle existed have `None`; they must be
    /// backfilled rather than left unable to fetch anything.
    #[tokio::test]
    async fn a_pre_bundle_row_is_backfilled() {
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

    /// Two starts racing to backfill must agree on one token. A caller that
    /// returned the token it generated rather than the one that persisted would
    /// hand a workspace a token the bundle endpoint rejects.
    #[tokio::test]
    async fn a_concurrent_backfill_agrees_on_one_token() {
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

    #[tokio::test]
    async fn spec_uses_pin_over_default() {
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
}
