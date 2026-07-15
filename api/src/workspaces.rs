//! Workspace policy layer: settings resolution, intent rows, lifecycle entry
//! points shared by the API handlers and the proxy, and the idle-stop sweeper.

use std::time::Duration;

use chrono::Utc;
use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, Set};

use crate::entities::{workspace, workspace_settings};
use crate::error::AppError;
use crate::orchestrator::{render_image, OrchestratorError, WorkspaceSpec, WorkspaceStatus};
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
            updated_at: Utc::now(),
        }))
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
    let version = match fetch_latest_release().await {
        Ok(tag) => tag,
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
        updated_at: Set(Utc::now()),
    }
    .insert(db)
    .await?;
    Ok(())
}

/// The `tag_name` of the latest agent-of-empires GitHub release.
async fn fetch_latest_release() -> Result<String, String> {
    #[derive(serde::Deserialize)]
    struct Release {
        tag_name: String,
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())?;
    let body = client
        .get("https://api.github.com/repos/agent-of-empires/agent-of-empires/releases/latest")
        // GitHub's API rejects requests without a User-Agent.
        .header("User-Agent", "cityhall")
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .text()
        .await
        .map_err(|e| e.to_string())?;
    let release: Release = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    Ok(release.tag_name)
}

/// The user's workspace intent row, created on first use.
pub async fn get_or_create(
    db: &DatabaseConnection,
    user_id: i32,
) -> Result<workspace::Model, AppError> {
    if let Some(row) = workspace::Entity::find_by_id(user_id).one(db).await? {
        return Ok(row);
    }
    let now = Utc::now();
    Ok(workspace::ActiveModel {
        user_id: Set(user_id),
        pinned_version: Set(None),
        last_active_at: Set(Some(now)),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(db)
    .await?)
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

pub fn build_spec(
    settings: &workspace_settings::Model,
    row: &workspace::Model,
) -> Result<WorkspaceSpec, AppError> {
    let version = effective_version(settings, row).ok_or(AppError::BadRequest(
        "no workspace version configured; set a default version in the workspace settings",
    ))?;
    Ok(WorkspaceSpec {
        user_id: row.user_id,
        image: render_image(&settings.image_template, &version),
        version,
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
pub async fn ensure_started(state: &AppState, user_id: i32) -> Result<String, AppError> {
    let cfg = settings(&state.db).await?;
    let row = get_or_create(&state.db, user_id).await?;
    let spec = build_spec(&cfg, &row)?;

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
    let addr = state.orchestrator.ensure_started(&spec).await?;
    state
        .endpoints
        .put(user_id, &spec.version, &spec.image, addr.clone());
    state.activity.touch(user_id);
    Ok(addr)
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
    use sea_orm::Database;
    use sea_orm_migration::MigratorTrait;

    async fn setup() -> DatabaseConnection {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        db
    }

    fn cfg(default_version: Option<&str>) -> workspace_settings::Model {
        workspace_settings::Model {
            id: SETTINGS_ID,
            image_template: "cityhall/aoe:{version}".to_string(),
            default_version: default_version.map(String::from),
            idle_stop_minutes: 30,
            updated_at: Utc::now(),
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
            updated_at: Set(Utc::now()),
        }
        .insert(&db)
        .await
        .unwrap();

        seed_default_version(&db).await.unwrap();
        assert_eq!(settings(&db).await.unwrap().default_version, None);
    }

    #[tokio::test]
    async fn settings_defaults_when_no_row() {
        let db = setup().await;
        let s = settings(&db).await.unwrap();
        assert_eq!(s.image_template, "cityhall/aoe:{version}");
        assert_eq!(s.idle_stop_minutes, 30);
    }

    #[tokio::test]
    async fn get_or_create_is_idempotent() {
        let db = setup().await;
        let uid = crate::service::create(&db, "u", None, "password123", false, None)
            .await
            .unwrap()
            .id;
        let a = get_or_create(&db, uid).await.unwrap();
        let b = get_or_create(&db, uid).await.unwrap();
        assert_eq!(a.user_id, b.user_id);
        assert_eq!(a.created_at, b.created_at);
    }

    #[tokio::test]
    async fn spec_uses_pin_over_default() {
        let db = setup().await;
        let uid = crate::service::create(&db, "u", None, "password123", false, None)
            .await
            .unwrap()
            .id;
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
        let uid = crate::service::create(&db, "u", None, "password123", false, None)
            .await
            .unwrap()
            .id;
        let row = get_or_create(&db, uid).await.unwrap();
        assert!(build_spec(&cfg(None), &row).is_err());
    }
}
