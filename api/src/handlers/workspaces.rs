use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::Json;
use chrono::Utc;
use sea_orm::{ActiveModelTrait, EntityTrait, Set};
use serde::{Deserialize, Serialize};

use crate::auth::AuthUser;
use crate::entities::{workspace, workspace_settings};
use crate::error::AppError;
use crate::orchestrator::WorkspaceStatus;
use crate::proxy;
use crate::state::AppState;
use crate::workspaces::{self, SETTINGS_ID};

#[derive(Serialize)]
pub struct WorkspaceItem {
    pub user_id: i32,
    pub username: String,
    /// `not_created` | `stopped` | `running` | `unknown` (runtime unreachable).
    pub status: &'static str,
    pub pinned_version: Option<String>,
    pub effective_version: Option<String>,
    pub last_active_at: Option<chrono::DateTime<Utc>>,
}

fn status_str(status: Result<WorkspaceStatus, impl std::fmt::Display>) -> &'static str {
    match status {
        Ok(WorkspaceStatus::NotCreated) => "not_created",
        Ok(WorkspaceStatus::Stopped) => "stopped",
        Ok(WorkspaceStatus::Running { .. }) => "running",
        Err(e) => {
            tracing::warn!("workspace status check failed: {e}");
            "unknown"
        }
    }
}

/// GET /api/workspaces: every user with their workspace state.
pub async fn list(
    State(state): State<AppState>,
    caller: AuthUser,
) -> Result<Json<Vec<WorkspaceItem>>, AppError> {
    caller.require("workspaces.read")?;
    let cfg = workspaces::settings(&state.db).await?;
    let users = crate::service::list(&state.db).await?;
    let rows = workspace::Entity::find().all(&state.db).await?;

    let mut items = Vec::with_capacity(users.len());
    for user in users {
        let row = rows.iter().find(|r| r.user_id == user.id);
        let effective = row.and_then(|r| workspaces::effective_version(&cfg, r));
        items.push(WorkspaceItem {
            user_id: user.id,
            status: match row {
                Some(_) => status_str(state.orchestrator.status(user.id).await),
                // No intent row: never used, skip the runtime round-trip.
                None => "not_created",
            },
            pinned_version: row.and_then(|r| r.pinned_version.clone()),
            effective_version: effective.or_else(|| cfg.default_version.clone()),
            last_active_at: row.and_then(|r| r.last_active_at),
            username: user.username,
        });
    }
    Ok(Json(items))
}

/// GET /api/workspaces/me: the caller's own workspace.
pub async fn me(
    State(state): State<AppState>,
    headers: HeaderMap,
    caller: AuthUser,
) -> Result<Json<serde_json::Value>, AppError> {
    caller.require("workspaces.use")?;
    let cfg = workspaces::settings(&state.db).await?;
    let row = workspace::Entity::find_by_id(caller.user.id)
        .one(&state.db)
        .await?;
    let status = status_str(state.orchestrator.status(caller.user.id).await);
    let effective = row
        .as_ref()
        .and_then(|r| workspaces::effective_version(&cfg, r))
        .or_else(|| cfg.default_version.clone());
    Ok(Json(serde_json::json!({
        "status": status,
        "pinned_version": row.as_ref().and_then(|r| r.pinned_version.clone()),
        "effective_version": effective,
        "proxy_origin": proxy::public_origin(&headers),
    })))
}

/// POST /api/workspaces/{user_id}/start
pub async fn start(
    State(state): State<AppState>,
    caller: AuthUser,
    Path(user_id): Path<i32>,
) -> Result<Json<serde_json::Value>, AppError> {
    caller.require("workspaces.write")?;
    ensure_user_exists(&state, user_id).await?;
    workspaces::ensure_started(&state, user_id).await?;
    Ok(Json(serde_json::json!({ "status": "running" })))
}

/// POST /api/workspaces/{user_id}/stop: stops the workspace, keeps its volume.
pub async fn stop(
    State(state): State<AppState>,
    caller: AuthUser,
    Path(user_id): Path<i32>,
) -> Result<Json<serde_json::Value>, AppError> {
    caller.require("workspaces.write")?;
    workspaces::stop(&state, user_id).await?;
    Ok(Json(serde_json::json!({ "status": "stopped" })))
}

/// DELETE /api/workspaces/{user_id}: destroys the workspace AND its volume.
pub async fn destroy(
    State(state): State<AppState>,
    caller: AuthUser,
    Path(user_id): Path<i32>,
) -> Result<Json<serde_json::Value>, AppError> {
    caller.require("workspaces.write")?;
    workspaces::destroy(&state, user_id).await?;
    Ok(Json(serde_json::json!({ "destroyed": true })))
}

#[derive(Deserialize)]
pub struct SetVersionRequest {
    /// `null` (or empty) unpins, following the default version.
    pub pinned_version: Option<String>,
}

/// PATCH /api/workspaces/{user_id}: pin or unpin the served aoe version. A
/// running workspace is recreated (volume kept) on its next start or proxied
/// request.
pub async fn set_version(
    State(state): State<AppState>,
    caller: AuthUser,
    Path(user_id): Path<i32>,
    Json(body): Json<SetVersionRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    caller.require("workspaces.write")?;
    ensure_user_exists(&state, user_id).await?;
    pin_version(&state, user_id, body.pinned_version.clone()).await?;
    Ok(Json(
        serde_json::json!({ "pinned_version": normalize(body.pinned_version) }),
    ))
}

#[derive(Deserialize)]
pub struct BulkSetVersionRequest {
    pub user_ids: Vec<i32>,
    pub pinned_version: Option<String>,
}

/// PATCH /api/workspaces: pin/unpin a group of users in one call (grouped
/// upgrade/downgrade).
pub async fn bulk_set_version(
    State(state): State<AppState>,
    caller: AuthUser,
    Json(body): Json<BulkSetVersionRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    caller.require("workspaces.write")?;
    if body.user_ids.is_empty() {
        return Err(AppError::BadRequest("user_ids is required"));
    }
    for user_id in &body.user_ids {
        ensure_user_exists(&state, *user_id).await?;
    }
    for user_id in body.user_ids {
        pin_version(&state, user_id, body.pinned_version.clone()).await?;
    }
    Ok(Json(
        serde_json::json!({ "pinned_version": normalize(body.pinned_version) }),
    ))
}

fn normalize(version: Option<String>) -> Option<String> {
    version
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

async fn ensure_user_exists(state: &AppState, user_id: i32) -> Result<(), AppError> {
    crate::entities::user::Entity::find_by_id(user_id)
        .one(&state.db)
        .await?
        .ok_or(AppError::NotFound("user not found"))?;
    Ok(())
}

async fn pin_version(
    state: &AppState,
    user_id: i32,
    version: Option<String>,
) -> Result<(), AppError> {
    let row = workspaces::get_or_create(&state.db, user_id).await?;
    let mut active: workspace::ActiveModel = row.into();
    active.pinned_version = Set(normalize(version));
    active.updated_at = Set(Utc::now());
    active.update(&state.db).await?;
    Ok(())
}

#[derive(Serialize)]
pub struct WorkspaceSettingsResponse {
    pub image_template: String,
    pub default_version: Option<String>,
    pub idle_stop_minutes: i32,
}

#[derive(Deserialize)]
pub struct UpdateWorkspaceSettingsRequest {
    pub image_template: String,
    pub default_version: Option<String>,
    pub idle_stop_minutes: i32,
}

/// GET /api/settings/workspaces
pub async fn get_settings(
    State(state): State<AppState>,
    caller: AuthUser,
) -> Result<Json<WorkspaceSettingsResponse>, AppError> {
    caller.require("settings.read")?;
    let cfg = workspaces::settings(&state.db).await?;
    Ok(Json(WorkspaceSettingsResponse {
        image_template: cfg.image_template,
        default_version: cfg.default_version,
        idle_stop_minutes: cfg.idle_stop_minutes,
    }))
}

/// PUT /api/settings/workspaces
pub async fn update_settings(
    State(state): State<AppState>,
    caller: AuthUser,
    Json(body): Json<UpdateWorkspaceSettingsRequest>,
) -> Result<Json<WorkspaceSettingsResponse>, AppError> {
    caller.require("settings.write")?;
    if body.image_template.trim().is_empty() {
        return Err(AppError::BadRequest("image template is required"));
    }
    if body.idle_stop_minutes < 1 {
        return Err(AppError::BadRequest("idle stop must be at least 1 minute"));
    }
    let default_version = normalize(body.default_version);

    let existing = workspace_settings::Entity::find_by_id(SETTINGS_ID)
        .one(&state.db)
        .await?;
    let model = workspace_settings::ActiveModel {
        id: Set(SETTINGS_ID),
        image_template: Set(body.image_template.trim().to_string()),
        default_version: Set(default_version),
        idle_stop_minutes: Set(body.idle_stop_minutes),
        updated_at: Set(Utc::now()),
    };
    if existing.is_some() {
        model.update(&state.db).await?;
    } else {
        model.insert(&state.db).await?;
    }
    get_settings(State(state), caller).await
}
