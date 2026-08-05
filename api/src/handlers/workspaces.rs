use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use chrono::Utc;
use sea_orm::{ActiveModelTrait, EntityTrait, Set};
use serde::{Deserialize, Serialize};

use crate::auth::AuthUser;
use crate::entities::{workspace, workspace_settings};
use crate::error::AppError;
use crate::orchestrator::{
    binary_key, render_image, telemetry_policy_override, TelemetryPolicy, WorkspaceStatus,
};
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
    /// Background artifact provisioning (image pull/build, binary download)
    /// for this user's effective version, when one is underway or failed.
    pub provisioning: Option<ProvisioningInfo>,
}

#[derive(Serialize)]
pub struct ProvisioningInfo {
    pub message: String,
    pub failed: bool,
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
        let effective = row
            .and_then(|r| workspaces::effective_version(&cfg, r))
            .or_else(|| cfg.default_version.clone());
        // Whichever backend runs, the effective version maps to exactly one
        // artifact key (image ref or binary); probe both.
        let provisioning = effective
            .as_ref()
            .and_then(|v| {
                let image = render_image(&cfg.image_template, v);
                state
                    .provisioning
                    .state(&image)
                    .or_else(|| state.provisioning.state(&binary_key(v)))
            })
            .map(|(message, failed)| ProvisioningInfo { message, failed });
        items.push(WorkspaceItem {
            user_id: user.id,
            status: match row {
                Some(_) => status_str(state.orchestrator.status(user.id).await),
                // No intent row: never used, skip the runtime round-trip.
                None => "not_created",
            },
            pinned_version: row.and_then(|r| r.pinned_version.clone()),
            effective_version: effective,
            last_active_at: row.and_then(|r| r.last_active_at),
            username: user.username,
            provisioning,
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

/// GET /api/workspaces/versions: discovered stable aoe releases (cached),
/// newest first, feeding the version dropdowns. Readable by anyone who can
/// see the workspaces page or the settings page.
pub async fn versions(
    State(state): State<AppState>,
    caller: AuthUser,
) -> Result<Json<serde_json::Value>, AppError> {
    if caller.require("workspaces.read").is_err() {
        caller.require("settings.read")?;
    }
    let (versions, stale) = state.versions.versions().await;
    Ok(Json(serde_json::json!({
        "latest": versions.first(),
        "versions": versions,
        "stale": stale,
    })))
}

/// POST /api/workspaces/{user_id}/access-url: a short-lived link that opens
/// `user_id`'s workspace through the proxy for a privileged caller. The
/// actual access grant (and its audit line) happens when the proxy exchanges
/// the token; this endpoint only mints it and does not start anything.
pub async fn access_url(
    State(state): State<AppState>,
    headers: HeaderMap,
    caller: AuthUser,
    Path(user_id): Path<i32>,
) -> Result<Json<serde_json::Value>, AppError> {
    caller.require("workspaces.impersonate")?;
    ensure_user_exists(&state, user_id).await?;
    let token = proxy::mint_exchange_token(caller.user.id, user_id)?;
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair(proxy::ACCESS_PARAM, &token)
        .finish();
    Ok(Json(serde_json::json!({
        "url": format!("{}/?{query}", proxy::public_origin(&headers)),
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

/// POST /api/workspaces/me/restart: recreate the caller's own workspace now,
/// if it is running.
///
/// Agent credentials are baked into a container's environment at create time,
/// not injected live, so a credential saved after a workspace was created
/// only takes effect the next time that container is created. Without this, a
/// user who just added a key would have to wait for an idle stop or ask an
/// admin to restart them; this lets them apply it themselves instead of a
/// credential save silently leaving their running session on the old
/// environment.
pub async fn restart_mine(
    State(state): State<AppState>,
    caller: AuthUser,
) -> Result<StatusCode, AppError> {
    caller.require("workspaces.use")?;
    crate::workspaces::restart_if_running(&state, caller.user.id).await?;
    Ok(StatusCode::NO_CONTENT)
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
    /// Recreate currently-running workspaces onto the new version now
    /// instead of on their next start (default lazy).
    #[serde(default)]
    pub restart: bool,
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
    let pinned_version = normalize(body.pinned_version);
    pin_version(&state, user_id, pinned_version.clone()).await?;
    if body.restart {
        spawn_restarts(state.clone(), vec![user_id]);
    }
    Ok(Json(
        serde_json::json!({ "pinned_version": pinned_version }),
    ))
}

#[derive(Deserialize)]
pub struct BulkSetVersionRequest {
    pub user_ids: Vec<i32>,
    pub pinned_version: Option<String>,
    /// Recreate currently-running workspaces onto the new version now
    /// instead of on their next start (default lazy).
    #[serde(default)]
    pub restart: bool,
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
    let pinned_version = normalize(body.pinned_version);
    for user_id in &body.user_ids {
        pin_version(&state, *user_id, pinned_version.clone()).await?;
    }
    if body.restart {
        spawn_restarts(state.clone(), body.user_ids);
    }
    Ok(Json(
        serde_json::json!({ "pinned_version": pinned_version }),
    ))
}

/// Eager rollout: recreate the listed users' RUNNING workspaces in the
/// background, one at a time (a recreate can ride a multi-minute image
/// provisioning; the PATCH must not hang on it). Failures land in the
/// provisioning tracker / logs and the list keeps showing live status.
fn spawn_restarts(state: AppState, user_ids: Vec<i32>) {
    tokio::spawn(async move {
        for user_id in user_ids {
            if let Err(e) = workspaces::restart_if_running(&state, user_id).await {
                tracing::warn!(user_id, "eager workspace restart failed: {e}");
            }
        }
    });
}

/// A version is free text: it is rendered into the image template, so any tag
/// an operator has built and tagged is valid, not only a discovered release.
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
    active.pinned_version = Set(version);
    active.updated_at = Set(Utc::now());
    active.update(&state.db).await?;
    Ok(())
}

#[derive(Serialize)]
pub struct WorkspaceSettingsResponse {
    pub image_template: String,
    pub default_version: Option<String>,
    pub idle_stop_minutes: i32,
    /// The stored telemetry policy, verbatim, which is what takes effect once any
    /// override is removed.
    ///
    /// Raw rather than parsed, so a value only a newer CityHall understands can
    /// be echoed back on a save and survive it (see [`policy_to_store`]).
    /// `effective_telemetry_policy` is where such a value reads as `user_choice`.
    pub telemetry_policy: String,
    /// The deployment-level override, when `WORKSPACE_TELEMETRY_POLICY` is set.
    /// Reported separately from the stored value so the page can say the policy
    /// is pinned by the environment instead of showing a control that silently
    /// does nothing.
    pub telemetry_policy_override: Option<&'static str>,
    /// What workspaces actually run under: the override, or the stored value.
    pub effective_telemetry_policy: &'static str,
}

#[derive(Deserialize)]
pub struct UpdateWorkspaceSettingsRequest {
    pub image_template: String,
    pub default_version: Option<String>,
    pub idle_stop_minutes: i32,
    pub telemetry_policy: String,
    /// Recreate every RUNNING workspace so the saved policy applies now.
    /// Off by default, and the same opt-in the version rollout uses: a recreate
    /// ends whatever the user is running. Stopped workspaces do not need it;
    /// they come back on the new policy at their next start.
    #[serde(default)]
    pub restart_running: bool,
}

/// The telemetry policy value a save should store.
///
/// A submitted value identical to what is already stored is a retention, not a
/// choice, and is kept verbatim. That is what stops a save made by an older
/// CityHall from flattening a policy a newer one wrote: this build reads such a
/// value as `user_choice` and shows it that way, but it never overwrites it
/// unless the admin actually picks something else.
///
/// Anything else is an explicit selection, so it has to be a policy this build
/// knows. Rejected rather than coerced: silently storing `user_choice` for a
/// value the admin believed forced telemetry off is the failure worth avoiding.
fn policy_to_store(submitted: &str, stored: Option<&str>) -> Result<String, AppError> {
    let submitted = submitted.trim();
    if stored == Some(submitted) {
        return Ok(submitted.to_string());
    }
    TelemetryPolicy::parse(submitted)
        .map(|policy| policy.as_str().to_string())
        .ok_or(AppError::BadRequest(
            "telemetry policy must be user_choice, force_on, or force_off",
        ))
}

/// GET /api/settings/workspaces
pub async fn get_settings(
    State(state): State<AppState>,
    caller: AuthUser,
) -> Result<Json<WorkspaceSettingsResponse>, AppError> {
    caller.require("settings.read")?;
    let cfg = workspaces::settings(&state.db).await?;
    // Unwrapped rather than propagated: an invalid override already failed
    // CityHall's startup, so this cannot be an error by the time a request runs.
    let policy_override = telemetry_policy_override().unwrap_or_default();
    Ok(Json(WorkspaceSettingsResponse {
        telemetry_policy_override: policy_override.map(TelemetryPolicy::as_str),
        effective_telemetry_policy: workspaces::effective_telemetry_policy(&cfg).as_str(),
        telemetry_policy: cfg.telemetry_policy,
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
    let telemetry_policy = policy_to_store(
        &body.telemetry_policy,
        existing.as_ref().map(|r| r.telemetry_policy.as_str()),
    )?;
    let model = workspace_settings::ActiveModel {
        id: Set(SETTINGS_ID),
        image_template: Set(body.image_template.trim().to_string()),
        default_version: Set(default_version),
        idle_stop_minutes: Set(body.idle_stop_minutes),
        telemetry_policy: Set(telemetry_policy),
        updated_at: Set(Utc::now()),
    };
    if existing.is_some() {
        model.update(&state.db).await?;
    } else {
        model.insert(&state.db).await?;
    }
    if body.restart_running {
        let user_ids = workspace::Entity::find()
            .all(&state.db)
            .await?
            .into_iter()
            .map(|r| r.user_id)
            .collect();
        spawn_restarts(state.clone(), user_ids);
    }
    get_settings(State(state), caller).await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole point of keeping the stored policy a raw string: an admin on an
    /// older CityHall, which reads a policy it does not know as `user_choice`,
    /// must not flatten it just by saving the settings form.
    #[test]
    fn an_unknown_stored_policy_survives_a_save_that_does_not_change_it() {
        assert_eq!(
            policy_to_store("force_maybe", Some("force_maybe")).unwrap(),
            "force_maybe"
        );
    }

    /// Choosing a policy this build knows replaces whatever was there.
    #[test]
    fn an_explicit_choice_replaces_the_stored_value() {
        assert_eq!(
            policy_to_store("force_off", Some("force_maybe")).unwrap(),
            "force_off"
        );
        assert_eq!(
            policy_to_store(" force_on ", Some("user_choice")).unwrap(),
            "force_on"
        );
    }

    /// An unknown value that is not simply what is already stored has no
    /// provenance to preserve, so it is a bad request rather than a way to write
    /// arbitrary strings into the column.
    #[test]
    fn an_unknown_value_cannot_be_introduced() {
        assert!(policy_to_store("force_maybe", Some("user_choice")).is_err());
        // Including on the very first save, when there is no row yet.
        assert!(policy_to_store("force_maybe", None).is_err());
        assert!(policy_to_store("", None).is_err());
    }
}
