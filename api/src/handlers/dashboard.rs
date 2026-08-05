//! Admin dashboard: fleet state, per-workspace usage, and system telemetry.
//!
//! This handler does no backend I/O. Every runtime figure comes from the
//! snapshot the background sampler publishes (see [`crate::metrics`]), so the
//! cost of a poll is one database read regardless of how many workspaces exist
//! or how many admins are watching.

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use chrono::{DateTime, Utc};
use sea_orm::{ActiveModelTrait, EntityTrait, Set};
use serde::{Deserialize, Serialize};

use crate::auth::AuthUser;
use crate::entities::{dashboard_layout, workspace};
use crate::error::AppError;
use crate::handlers::workspaces::{runtime_status, ProvisioningInfo};
use crate::metrics::SystemUsage;
use crate::orchestrator::{
    binary_key, render_image, UsageReport, WorkspaceRuntime, WorkspaceUsage,
};
use crate::state::AppState;
use crate::workspaces;

#[derive(Serialize)]
pub struct DashboardResponse {
    /// `null` until the sampler's first tick lands, which the UI shows as
    /// "collecting" rather than as an empty fleet.
    pub sampled_at: Option<DateTime<Utc>>,
    /// The sampler looks stuck, not merely between ticks.
    pub stale: bool,
    /// Which sources failed on the last tick. The figures beside them are the
    /// last good ones.
    pub errors: Vec<String>,
    pub system: Option<SystemUsage>,
    /// False only when the backend has no metrics source at all (kubernetes,
    /// bare process), which the UI states outright instead of showing zeros.
    /// Before the first sample this is optimistically true and `sampled_at`
    /// is what tells the UI to say "collecting".
    pub usage_supported: bool,
    pub usage: Vec<WorkspaceUsage>,
    pub workspaces: Vec<DashboardWorkspace>,
    pub summary: Summary,
    pub versions: Vec<VersionCount>,
}

#[derive(Serialize)]
pub struct DashboardWorkspace {
    pub user_id: i32,
    pub username: String,
    /// `not_created` | `stopped` | `running` | `unknown`.
    pub status: &'static str,
    /// What this user should be running, from their pin or the global default.
    pub effective_version: Option<String>,
    /// What the runtime object was actually created with, when the backend
    /// reports it. Differs from `effective_version` while a version change is
    /// waiting for a restart.
    pub running_version: Option<String>,
    pub last_active_at: Option<DateTime<Utc>>,
    pub provisioning: Option<ProvisioningInfo>,
}

/// Fleet counts, computed here rather than in the browser so that "running"
/// means one thing no matter what reads this endpoint.
#[derive(Debug, Default, PartialEq, Serialize)]
pub struct Summary {
    pub total_users: usize,
    pub running: usize,
    pub stopped: usize,
    pub not_created: usize,
    pub unknown: usize,
    pub provisioning: usize,
    /// How many workspaces the sampler currently has usage figures for.
    pub usage_available: usize,
}

#[derive(Debug, PartialEq, Serialize)]
pub struct VersionCount {
    pub version: String,
    pub count: usize,
}

/// GET /api/dashboard
pub async fn overview(
    State(state): State<AppState>,
    caller: AuthUser,
) -> Result<Json<DashboardResponse>, AppError> {
    caller.require("dashboard.read")?;

    let snapshot = state.metrics.snapshot();
    let cfg = workspaces::settings(&state.db).await?;
    let users = crate::service::list(&state.db).await?;
    let rows = workspace::Entity::find().all(&state.db).await?;

    let mut items = Vec::with_capacity(users.len());
    for user in users {
        let row = rows.iter().find(|r| r.user_id == user.id);
        let effective_version = row
            .and_then(|r| workspaces::effective_version(&cfg, r))
            .or_else(|| cfg.default_version.clone());
        let provisioning = effective_version
            .as_ref()
            .and_then(|v| {
                let image = render_image(&cfg.image_template, v);
                state
                    .provisioning
                    .state(&image)
                    .or_else(|| state.provisioning.state(&binary_key(v)))
            })
            .map(|(message, failed)| ProvisioningInfo { message, failed });
        let runtime = attributed_runtime(row, snapshot.runtime(user.id));
        items.push(DashboardWorkspace {
            user_id: user.id,
            username: user.username,
            status: match (row, snapshot.sampled_at) {
                // No intent row: the workspace was never used, and no sample
                // is needed to know that.
                (None, _) => "not_created",
                // Nothing sampled yet, so the runtime state is genuinely not
                // known; claiming "not created" here would be a guess.
                (Some(_), None) => "unknown",
                (Some(_), Some(_)) => runtime_status(runtime),
            },
            running_version: runtime.and_then(|r| r.version.clone()),
            effective_version,
            last_active_at: row.and_then(|r| r.last_active_at),
            provisioning,
        });
    }

    let (usage_supported, usage) = match snapshot.usage.as_ref() {
        Some(UsageReport::Sampled(rows)) => (true, rows.clone()),
        Some(UsageReport::Unsupported) => (false, Vec::new()),
        None => (true, Vec::new()),
    };

    Ok(Json(DashboardResponse {
        sampled_at: snapshot.sampled_at,
        stale: snapshot.stale(),
        errors: snapshot.errors.clone(),
        system: snapshot.system.clone(),
        summary: summarize(&items, &usage),
        versions: version_counts(&items),
        usage_supported,
        usage,
        workspaces: items,
    }))
}

/// The one layout shape this build understands. Bumped only on a breaking
/// change; a stored row at any other version is ignored and the caller gets
/// the catalog defaults, which is why old rows can simply be left in place.
const LAYOUT_SCHEMA_VERSION: u32 = 1;
/// Enough for every widget the catalog is ever likely to hold, and small
/// enough that a row cannot be used as storage.
const MAX_LAYOUT_ITEMS: usize = 32;
/// Applies to the serialized form, checked after parsing so the limit is on
/// what actually gets stored.
const MAX_LAYOUT_BYTES: usize = 8 * 1024;
/// The grid width the frontend lays out against.
const LAYOUT_COLUMNS: u32 = 12;

/// A saved widget arrangement.
///
/// Validated rather than stored opaquely. Only its own author reads it back, so
/// this is not a breach vector; what it prevents is a caller persisting a
/// payload that breaks their dashboard on every load, or using the row as
/// unbounded storage. `deny_unknown_fields` keeps a future field from being
/// silently accepted by an older build that would then drop it on the next
/// save.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DashboardLayout {
    pub schema_version: u32,
    pub items: Vec<LayoutItem>,
    /// Widgets the user turned off. Kept separate from `items` so hiding one
    /// and re-showing it does not lose where it used to sit.
    #[serde(default)]
    pub hidden: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LayoutItem {
    pub id: String,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// Widget ids come from the frontend catalog and are compared as strings, so
/// the charset is pinned rather than left to whatever a caller sends.
fn valid_widget_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id.starts_with(|c: char| c.is_ascii_lowercase())
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Structural checks only. Geometry is clamped by the frontend merge, which has
/// to be defensive anyway; what matters here is that nothing unbounded or
/// self-contradictory reaches the database.
fn validate_layout(layout: &DashboardLayout) -> Result<(), AppError> {
    if layout.schema_version != LAYOUT_SCHEMA_VERSION {
        return Err(AppError::BadRequest("unsupported layout schema version"));
    }
    if layout.items.len() > MAX_LAYOUT_ITEMS || layout.hidden.len() > MAX_LAYOUT_ITEMS {
        return Err(AppError::BadRequest("too many layout widgets"));
    }
    for item in &layout.items {
        if !valid_widget_id(&item.id) {
            return Err(AppError::BadRequest("invalid layout widget id"));
        }
        if item.w == 0 || item.h == 0 {
            return Err(AppError::BadRequest("layout widgets need a size"));
        }
        // Saturating: `x + w` on the raw values overflows for an `x` near
        // `u32::MAX`, which panics in debug and wraps past this check in
        // release.
        if item.x.saturating_add(item.w) > LAYOUT_COLUMNS {
            return Err(AppError::BadRequest("layout widget exceeds the grid width"));
        }
    }
    if layout.hidden.iter().any(|id| !valid_widget_id(id)) {
        return Err(AppError::BadRequest("invalid layout widget id"));
    }
    // A duplicate id would make the merge nondeterministic: which of the two
    // rectangles wins depends on iteration order.
    let mut ids: Vec<&str> = layout.items.iter().map(|i| i.id.as_str()).collect();
    ids.sort_unstable();
    let count = ids.len();
    ids.dedup();
    if ids.len() != count {
        return Err(AppError::BadRequest("duplicate layout widget id"));
    }
    Ok(())
}

/// GET /api/me/dashboard-layout: the caller's saved layout, or `null` when
/// they have never customized it.
pub async fn get_layout(
    State(state): State<AppState>,
    caller: AuthUser,
) -> Result<Json<serde_json::Value>, AppError> {
    caller.require("dashboard.read")?;
    let row = dashboard_layout::Entity::find_by_id(caller.user.id)
        .one(&state.db)
        .await?;
    // A row written by a newer build, or hand-edited into nonsense, must not
    // break the dashboard: it reads as "no saved layout" and the next save
    // replaces it.
    let layout = row
        .and_then(|r| serde_json::from_str::<DashboardLayout>(&r.layout).ok())
        .filter(|l| validate_layout(l).is_ok());
    Ok(Json(serde_json::json!({ "layout": layout })))
}

/// PUT /api/me/dashboard-layout
pub async fn put_layout(
    State(state): State<AppState>,
    caller: AuthUser,
    Json(layout): Json<DashboardLayout>,
) -> Result<StatusCode, AppError> {
    caller.require("dashboard.read")?;
    validate_layout(&layout)?;

    // Serialized from the parsed value, not echoed from the request body, so
    // only the shape above is ever stored.
    let encoded = serde_json::to_string(&layout)
        .map_err(|_| AppError::BadRequest("layout could not be stored"))?;
    if encoded.len() > MAX_LAYOUT_BYTES {
        return Err(AppError::BadRequest("layout is too large"));
    }

    let model = dashboard_layout::ActiveModel {
        user_id: Set(caller.user.id),
        layout: Set(encoded),
        updated_at: Set(Utc::now()),
    };
    if dashboard_layout::Entity::find_by_id(caller.user.id)
        .one(&state.db)
        .await?
        .is_some()
    {
        model.update(&state.db).await?;
    } else {
        model.insert(&state.db).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// The runtime state belonging to a user, which is none at all unless they
/// have an intent row.
///
/// A container can outlive the row that created it: a reset database, a
/// hand-removed row. It is not that user's workspace, and consulting it anyway
/// reported the orphan's version beside a `not_created` status. Status and
/// version have to come from one decision, so both go through here.
fn attributed_runtime<'a>(
    row: Option<&workspace::Model>,
    runtime: Option<&'a WorkspaceRuntime>,
) -> Option<&'a WorkspaceRuntime> {
    row.and(runtime)
}

fn summarize(items: &[DashboardWorkspace], usage: &[WorkspaceUsage]) -> Summary {
    let mut summary = Summary {
        total_users: items.len(),
        usage_available: usage.len(),
        ..Summary::default()
    };
    for item in items {
        match item.status {
            "running" => summary.running += 1,
            "stopped" => summary.stopped += 1,
            "not_created" => summary.not_created += 1,
            _ => summary.unknown += 1,
        }
        if item.provisioning.is_some() {
            summary.provisioning += 1;
        }
    }
    summary
}

/// How many users sit on each effective version, most-used first, so a
/// half-finished rollout is visible at a glance. Users with no version
/// configured are not a version and are left out.
fn version_counts(items: &[DashboardWorkspace]) -> Vec<VersionCount> {
    let mut counts: Vec<VersionCount> = Vec::new();
    for version in items.iter().filter_map(|i| i.effective_version.as_deref()) {
        match counts.iter_mut().find(|c| c.version == version) {
            Some(existing) => existing.count += 1,
            None => counts.push(VersionCount {
                version: version.to_string(),
                count: 1,
            }),
        }
    }
    // Version as the tiebreaker so equal counts do not reorder between polls
    // and make the widget flicker.
    counts.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| a.version.cmp(&b.version))
    });
    counts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace(status: &'static str, version: Option<&str>) -> DashboardWorkspace {
        DashboardWorkspace {
            user_id: 1,
            username: "u".to_string(),
            status,
            effective_version: version.map(String::from),
            running_version: None,
            last_active_at: None,
            provisioning: None,
        }
    }

    #[test]
    fn the_summary_counts_every_status() {
        let items = vec![
            workspace("running", Some("v1")),
            workspace("running", Some("v1")),
            workspace("stopped", Some("v1")),
            workspace("not_created", None),
            workspace("unknown", Some("v2")),
        ];
        let usage = vec![WorkspaceUsage {
            user_id: 1,
            cpu_percent: 1.0,
            memory_bytes: 2,
            memory_limit_bytes: None,
        }];
        assert_eq!(
            summarize(&items, &usage),
            Summary {
                total_users: 5,
                running: 2,
                stopped: 1,
                not_created: 1,
                unknown: 1,
                provisioning: 0,
                usage_available: 1,
            }
        );
    }

    /// Provisioning is a state a workspace is in *besides* its runtime status,
    /// so it is counted separately rather than replacing one of the others.
    #[test]
    fn provisioning_is_counted_without_hiding_a_status() {
        let mut item = workspace("not_created", Some("v1"));
        item.provisioning = Some(ProvisioningInfo {
            message: "pulling".to_string(),
            failed: false,
        });
        let summary = summarize(&[item], &[]);
        assert_eq!(summary.provisioning, 1);
        assert_eq!(summary.not_created, 1);
        assert_eq!(summary.total_users, 1);
    }

    /// Caught by a smoke test against a machine holding a container from an
    /// earlier database: the dashboard printed `not_created` beside
    /// `running_version: main-20260804`.
    #[test]
    fn a_runtime_without_an_intent_row_is_not_adopted() {
        let runtime = WorkspaceRuntime {
            user_id: 1,
            running: true,
            version: Some("main-20260804".to_string()),
        };

        let orphan = attributed_runtime(None, Some(&runtime));
        assert!(orphan.is_none());
        // Both fields go through the same decision, so neither can leak the
        // orphan's state on its own.
        assert_eq!(runtime_status(orphan), "not_created");
        assert_eq!(orphan.and_then(|r| r.version.clone()), None);

        let row = workspace::Model {
            user_id: 1,
            pinned_version: None,
            last_active_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            bundle_token: None,
        };
        let owned = attributed_runtime(Some(&row), Some(&runtime));
        assert_eq!(runtime_status(owned), "running");
        assert_eq!(
            owned.and_then(|r| r.version.clone()),
            Some("main-20260804".to_string())
        );
    }

    fn layout(items: Vec<(&str, u32, u32)>) -> DashboardLayout {
        DashboardLayout {
            schema_version: LAYOUT_SCHEMA_VERSION,
            items: items
                .into_iter()
                .map(|(id, x, w)| LayoutItem {
                    id: id.to_string(),
                    x,
                    y: 0,
                    w,
                    h: 3,
                })
                .collect(),
            hidden: Vec::new(),
        }
    }

    #[test]
    fn a_well_formed_layout_is_accepted() {
        let mut ok = layout(vec![("system-usage", 0, 6), ("fleet-status", 6, 6)]);
        assert!(validate_layout(&ok).is_ok());
        ok.hidden = vec!["version-spread".to_string()];
        assert!(validate_layout(&ok).is_ok());
        // A widget may span the full grid, just not exceed it.
        assert!(validate_layout(&layout(vec![("system-usage", 0, 12)])).is_ok());
    }

    #[test]
    fn a_layout_from_another_schema_version_is_refused() {
        let mut wrong = layout(vec![("system-usage", 0, 6)]);
        wrong.schema_version = LAYOUT_SCHEMA_VERSION + 1;
        assert!(validate_layout(&wrong).is_err());
        wrong.schema_version = 0;
        assert!(validate_layout(&wrong).is_err());
    }

    /// A saved row is only ever read back by its own author, so this is not
    /// about a breach: it stops a caller persisting something that breaks
    /// their own dashboard on every load, or using the row as storage.
    #[test]
    fn unbounded_and_self_contradictory_layouts_are_refused() {
        let many: Vec<(&str, u32, u32)> = (0..MAX_LAYOUT_ITEMS + 1)
            .map(|_| ("system-usage", 0, 1))
            .collect();
        assert!(validate_layout(&layout(many)).is_err());

        let mut hidden = layout(vec![("system-usage", 0, 6)]);
        hidden.hidden = (0..MAX_LAYOUT_ITEMS + 1).map(|_| "w".to_string()).collect();
        assert!(validate_layout(&hidden).is_err());

        // Two rectangles for one widget: which one wins would depend on
        // iteration order.
        assert!(validate_layout(&layout(vec![
            ("system-usage", 0, 6),
            ("system-usage", 6, 6)
        ]))
        .is_err());

        // Off the grid, and zero-sized.
        assert!(validate_layout(&layout(vec![("system-usage", 8, 6)])).is_err());
        assert!(validate_layout(&layout(vec![("system-usage", 0, 0)])).is_err());

        // Extreme coordinates must be rejected, not overflow the bounds check
        // into passing it.
        assert!(validate_layout(&layout(vec![("system-usage", u32::MAX, 1)])).is_err());
        assert!(validate_layout(&layout(vec![("system-usage", 1, u32::MAX)])).is_err());
        let mut tall = layout(vec![("system-usage", 0, 6)]);
        tall.items[0].y = u32::MAX;
        // A huge `y` only makes a very long page for its own author, so it is
        // the frontend merge that clamps it; it must at least not panic here.
        assert!(validate_layout(&tall).is_ok());
    }

    #[test]
    fn widget_ids_are_restricted_to_the_catalog_charset() {
        assert!(valid_widget_id("system-usage"));
        assert!(valid_widget_id("w1"));
        assert!(!valid_widget_id(""));
        assert!(!valid_widget_id("System-Usage"));
        assert!(!valid_widget_id("1widget"));
        assert!(!valid_widget_id("-widget"));
        assert!(!valid_widget_id("widget_name"));
        assert!(!valid_widget_id("../../etc/passwd"));
        assert!(!valid_widget_id(&"w".repeat(65)));

        assert!(validate_layout(&layout(vec![("Bad Id", 0, 6)])).is_err());
        let mut bad_hidden = layout(vec![("system-usage", 0, 6)]);
        bad_hidden.hidden = vec!["Bad Id".to_string()];
        assert!(validate_layout(&bad_hidden).is_err());
    }

    /// An older build must not silently accept and then drop a field a newer
    /// one added, which is what turns a forward-compatible save into data loss.
    #[test]
    fn an_unknown_layout_field_is_refused_at_parse() {
        let json = r#"{"schema_version":1,"items":[],"hidden":[],"columns":24}"#;
        assert!(serde_json::from_str::<DashboardLayout>(json).is_err());
        // `hidden` is genuinely optional, though.
        let json = r#"{"schema_version":1,"items":[]}"#;
        let parsed: DashboardLayout = serde_json::from_str(json).unwrap();
        assert!(parsed.hidden.is_empty());
    }

    #[test]
    fn version_counts_are_ordered_and_stable() {
        let items = vec![
            workspace("running", Some("v2")),
            workspace("running", Some("v1")),
            workspace("running", Some("v1")),
            // No version configured is not a version.
            workspace("not_created", None),
        ];
        assert_eq!(
            version_counts(&items),
            vec![
                VersionCount {
                    version: "v1".to_string(),
                    count: 2
                },
                VersionCount {
                    version: "v2".to_string(),
                    count: 1
                },
            ]
        );

        // Equal counts order by version, so a poll cannot reshuffle the widget.
        let tied = vec![
            workspace("running", Some("v2")),
            workspace("running", Some("v1")),
        ];
        assert_eq!(
            version_counts(&tied)
                .iter()
                .map(|c| c.version.as_str())
                .collect::<Vec<_>>(),
            vec!["v1", "v2"]
        );
        assert!(version_counts(&[]).is_empty());
    }
}
