//! Admin dashboard: fleet state, per-workspace usage, and system telemetry.
//!
//! This handler does no backend I/O. Every runtime figure comes from the
//! snapshot the background sampler publishes (see [`crate::metrics`]), so the
//! cost of a poll is one database read regardless of how many workspaces exist
//! or how many admins are watching.

use axum::extract::State;
use axum::Json;
use chrono::{DateTime, Utc};
use sea_orm::EntityTrait;
use serde::Serialize;

use crate::auth::AuthUser;
use crate::entities::workspace;
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
