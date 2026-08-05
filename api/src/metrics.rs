//! System metrics for the admin dashboard.
//!
//! Two rules shape this module, both learned from how easy these numbers are to
//! misread:
//!
//! - Nothing here is called "host". CityHall commonly runs in a container next
//!   to the very workspaces it reports on, so the figures describe the system
//!   *visible to the CityHall process*, which may or may not be the docker
//!   host. Every payload carries its [`MetricScope`] so the UI can say which.
//! - A cgroup memory limit never overrides the system totals. Substituting it
//!   would produce system-wide CPU beside container-local memory under one
//!   label, which is worse than either number alone; it is reported as its own
//!   block instead.

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::Serialize;
use sysinfo::System;

use sea_orm::EntityTrait;

use crate::entities::workspace;
use crate::orchestrator::{UsageReport, WorkspaceRuntime, WorkspaceStatus};
use crate::state::AppState;

/// How often the fleet is sampled. Matched to the dashboard's own poll so a
/// viewer rarely sees the same numbers twice, and the load is the same three
/// backend calls whether one admin is watching or ten.
const SAMPLE_INTERVAL: Duration = Duration::from_secs(10);

/// Snapshots older than this are flagged in the UI: at three missed intervals
/// the sampler is stuck rather than merely between ticks.
const STALE_AFTER: Duration = Duration::from_secs(30);

/// What the system figures actually describe.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricScope {
    /// CityHall runs directly on the machine, so these are host figures.
    Host,
    /// CityHall runs in a container: the numbers are whatever its namespace
    /// sees, which is usually the host's CPU and memory rather than its own.
    CityHallContainer,
}

#[derive(Clone, Debug, Serialize)]
pub struct SystemUsage {
    pub scope: MetricScope,
    /// Percentage across all logical CPUs, so a fully busy 8-core box reads
    /// 100, not 800.
    pub cpu_percent: f32,
    pub cpu_count: usize,
    pub memory_used_bytes: u64,
    pub memory_total_bytes: u64,
    /// CityHall's own cgroup memory limit, when it has one that is actually
    /// narrower than the system's. Never a substitute for the fields above.
    pub cgroup_memory: Option<CgroupMemory>,
    /// Only present when `SYSTEM_METRICS_DISK_PATH` names a filesystem that
    /// could be resolved. There is no default: see [`disk_path`].
    pub disk: Option<DiskUsage>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CgroupMemory {
    pub used_bytes: u64,
    pub limit_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DiskUsage {
    /// The configured path that was asked about.
    pub path: String,
    /// The mount point actually holding it, which is often shorter than
    /// `path` and is worth showing so an operator can tell the two apart.
    pub mount_point: String,
    pub used_bytes: u64,
    pub total_bytes: u64,
}

/// Owns the long-lived `sysinfo::System`. CPU percentages are deltas between
/// two refreshes, so the sampler has to outlive a single measurement; a
/// per-request sampler would report either zero or nonsense.
pub struct SystemSampler {
    system: System,
    scope: MetricScope,
    disk_path: Option<PathBuf>,
}

impl SystemSampler {
    pub fn from_env() -> Self {
        SystemSampler {
            system: System::new(),
            scope: scope(),
            disk_path: disk_path(),
        }
    }

    /// Prime the CPU delta. `global_cpu_usage` compares against the previous
    /// refresh, so without this the first published snapshot reports 0%.
    pub async fn prime(&mut self) {
        self.system.refresh_cpu_usage();
        tokio::time::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL).await;
    }

    pub fn sample(&mut self) -> SystemUsage {
        self.system.refresh_cpu_usage();
        self.system.refresh_memory();

        let memory_total_bytes = self.system.total_memory();
        SystemUsage {
            scope: self.scope,
            cpu_percent: self.system.global_cpu_usage(),
            cpu_count: self.system.cpus().len(),
            memory_used_bytes: self.system.used_memory(),
            memory_total_bytes,
            cgroup_memory: cgroup_memory(self.system.cgroup_limits(), memory_total_bytes),
            disk: self
                .disk_path
                .as_deref()
                .and_then(|path| disk_usage(&sysinfo::Disks::new_with_refreshed_list(), path)),
        }
    }
}

/// Everything the dashboard needs about the runtime, as of one moment.
///
/// Sources fail independently. A tick that cannot reach the daemon keeps the
/// previous values for whatever it could not refresh and records why, because
/// blanking a dashboard is a worse answer than showing numbers that are ten
/// seconds old and labelled as such.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    /// `None` until the first tick completes, which the UI shows as
    /// "collecting" rather than as an absence of workspaces.
    pub sampled_at: Option<DateTime<Utc>>,
    pub runtimes: Vec<WorkspaceRuntime>,
    pub usage: Option<UsageReport>,
    pub system: Option<SystemUsage>,
    /// What failed on the most recent tick, for display. Empty is the happy
    /// path.
    pub errors: Vec<String>,
}

impl Snapshot {
    pub fn runtime(&self, user_id: i32) -> Option<&WorkspaceRuntime> {
        self.runtimes.iter().find(|r| r.user_id == user_id)
    }

    /// Whether the sampler looks stuck rather than simply between ticks.
    pub fn stale(&self) -> bool {
        match self.sampled_at {
            None => true,
            Some(at) => (Utc::now() - at)
                .to_std()
                .map(|age| age > STALE_AFTER)
                .unwrap_or(false),
        }
    }
}

/// The published snapshot. Readers clone an `Arc`, so a dashboard request never
/// waits on the sampler and never touches the container runtime.
#[derive(Default)]
pub struct Metrics {
    snapshot: RwLock<Arc<Snapshot>>,
}

impl Metrics {
    pub fn snapshot(&self) -> Arc<Snapshot> {
        self.snapshot.read().unwrap().clone()
    }

    fn publish(&self, snapshot: Snapshot) {
        *self.snapshot.write().unwrap() = Arc::new(snapshot);
    }
}

/// Background loop owning every backend read the dashboard displays.
///
/// The dashboard deliberately does no collection of its own: doing it per
/// request meant one process spawn per user per poll per watching admin, which
/// is how a control plane ends up load-testing its own container runtime.
pub async fn sampler(state: AppState) {
    let mut system = SystemSampler::from_env();
    // Without a primed delta the first published CPU figure is always 0%.
    system.prime().await;

    let mut interval = tokio::time::interval(SAMPLE_INTERVAL);
    // Delay, not Burst: a tick that overran must not be followed by a queued
    // pile of catch-up samples hammering the daemon.
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        interval.tick().await;
        let snapshot = sample_once(&state, &mut system).await;
        state.metrics.publish(snapshot);
    }
}

async fn sample_once(state: &AppState, system: &mut SystemSampler) -> Snapshot {
    let previous = state.metrics.snapshot();
    let mut errors = Vec::new();

    let runtimes = match statuses(state).await {
        Ok(rows) => rows,
        Err(e) => {
            errors.push(format!("workspace status: {e}"));
            previous.runtimes.clone()
        }
    };

    let usage = match state.orchestrator.usage().await {
        Ok(report) => Some(report),
        Err(e) => {
            errors.push(format!("workspace usage: {e}"));
            previous.usage.clone()
        }
    };

    Snapshot {
        sampled_at: Some(Utc::now()),
        runtimes,
        usage,
        system: Some(system.sample()),
        errors,
    }
}

/// Fleet runtime state, batched when the backend can and per-user when it
/// cannot. The fallback lives here rather than in the request path so the
/// dashboard costs the same on every backend.
async fn statuses(state: &AppState) -> Result<Vec<WorkspaceRuntime>, String> {
    if let Some(rows) = state
        .orchestrator
        .statuses()
        .await
        .map_err(|e| e.to_string())?
    {
        return Ok(rows);
    }

    let rows = workspace::Entity::find()
        .all(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    let mut runtimes = Vec::with_capacity(rows.len());
    for row in rows {
        // A per-user failure is that workspace's problem, not the fleet's: it
        // simply has no runtime row and reads as unknown.
        if let Ok(status) = state.orchestrator.status(row.user_id).await {
            runtimes.push(WorkspaceRuntime {
                user_id: row.user_id,
                running: matches!(status, WorkspaceStatus::Running { .. }),
                // This path has no version to report; the effective version
                // from the database covers the display.
                version: None,
            });
        }
    }
    Ok(runtimes)
}

/// `SYSTEM_METRICS_SCOPE` wins when set; otherwise detect, so the common
/// compose deployment is labelled correctly without the operator doing
/// anything. Detection is a heuristic, which is exactly why the override
/// exists.
fn scope() -> MetricScope {
    match parse_scope(std::env::var("SYSTEM_METRICS_SCOPE").ok().as_deref()) {
        Some(scope) => scope,
        None => detect_scope(),
    }
}

fn parse_scope(value: Option<&str>) -> Option<MetricScope> {
    match value.map(str::trim).unwrap_or_default() {
        "host" => Some(MetricScope::Host),
        "container" => Some(MetricScope::CityHallContainer),
        other => {
            if !other.is_empty() {
                tracing::warn!(
                    "ignoring SYSTEM_METRICS_SCOPE '{other}' (expected host or container)"
                );
            }
            None
        }
    }
}

/// `/.dockerenv` covers docker and compose; PID 1's cgroup path covers
/// podman, containerd, and kubernetes. Neither is authoritative, hence the
/// override above.
fn detect_scope() -> MetricScope {
    if Path::new("/.dockerenv").exists() {
        return MetricScope::CityHallContainer;
    }
    match std::fs::read_to_string("/proc/1/cgroup") {
        Ok(cgroup) if containerized_cgroup(&cgroup) => MetricScope::CityHallContainer,
        _ => MetricScope::Host,
    }
}

fn containerized_cgroup(cgroup: &str) -> bool {
    ["docker", "kubepods", "containerd", "libpod", "lxc"]
        .iter()
        .any(|marker| cgroup.contains(marker))
}

/// The filesystem to report, or `None` to omit disk entirely.
///
/// There is deliberately no default. Under an overlay filesystem `/` measures
/// the container's own image layers, not the volumes workspaces live on, so a
/// default would put a confidently wrong number on the dashboard. Unset means
/// the widget disappears, which is honest.
fn disk_path() -> Option<PathBuf> {
    let raw = std::env::var("SYSTEM_METRICS_DISK_PATH").unwrap_or_default();
    let trimmed = raw.trim();
    (!trimmed.is_empty()).then(|| PathBuf::from(trimmed))
}

/// A cgroup limit is only worth showing when it is actually narrower than the
/// system's memory. An unconstrained cgroup on Linux reports the host total,
/// and echoing that back as a second identical block just invites the reader
/// to think one of the two numbers is wrong.
fn cgroup_memory(limits: Option<sysinfo::CGroupLimits>, system_total: u64) -> Option<CgroupMemory> {
    let limits = limits?;
    (limits.total_memory > 0 && limits.total_memory < system_total).then(|| CgroupMemory {
        used_bytes: limits.total_memory.saturating_sub(limits.free_memory),
        limit_bytes: limits.total_memory,
    })
}

fn disk_usage(disks: &sysinfo::Disks, path: &Path) -> Option<DiskUsage> {
    let mounts: Vec<(&Path, u64, u64)> = disks
        .list()
        .iter()
        .map(|d| (d.mount_point(), d.total_space(), d.available_space()))
        .collect();
    select_disk(&mounts, path)
}

/// The mount point holding `path`: the longest one that is a prefix of it.
///
/// Longest wins because mount points nest. With `/` and `/var/lib/docker` both
/// mounted, a path under the latter has to resolve to it, and picking the
/// first prefix match would report the wrong filesystem's free space.
fn select_disk(mounts: &[(&Path, u64, u64)], path: &Path) -> Option<DiskUsage> {
    mounts
        .iter()
        .filter(|(mount, _, _)| path.starts_with(mount))
        .max_by_key(|(mount, _, _)| mount.as_os_str().len())
        .map(|(mount, total, available)| DiskUsage {
            path: path.display().to_string(),
            mount_point: mount.display().to_string(),
            used_bytes: total.saturating_sub(*available),
            total_bytes: *total,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_explicit_scope_overrides_detection() {
        assert_eq!(parse_scope(Some("host")), Some(MetricScope::Host));
        assert_eq!(
            parse_scope(Some(" container ")),
            Some(MetricScope::CityHallContainer)
        );
        // Unset or unparseable falls through to detection rather than
        // guessing, so a typo cannot silently mislabel every metric.
        assert_eq!(parse_scope(None), None);
        assert_eq!(parse_scope(Some("")), None);
        assert_eq!(parse_scope(Some("HOST")), None);
        assert_eq!(parse_scope(Some("yes")), None);
    }

    #[test]
    fn container_runtimes_are_recognized_in_a_cgroup_path() {
        assert!(containerized_cgroup(
            "0::/docker/3f2a1b0c9d8e7f6a5b4c3d2e1f0a9b8c"
        ));
        assert!(containerized_cgroup("0::/kubepods/besteffort/pod1234"));
        assert!(containerized_cgroup("0::/libpod-abc.scope"));
        // A bare systemd host slice is not a container.
        assert!(!containerized_cgroup("0::/init.scope"));
        assert!(!containerized_cgroup("0::/user.slice/user-1000.slice"));
    }

    #[test]
    fn a_cgroup_limit_is_reported_only_when_narrower_than_the_system() {
        let limits = |total, free| {
            Some(sysinfo::CGroupLimits {
                total_memory: total,
                free_memory: free,
                free_swap: 0,
                rss: 0,
            })
        };

        // A real container limit: reported, with used derived from free.
        assert_eq!(
            cgroup_memory(limits(1_000, 250), 8_000),
            Some(CgroupMemory {
                used_bytes: 750,
                limit_bytes: 1_000
            })
        );

        // An unconstrained cgroup reports the host total. Echoing that back as
        // a second, identical memory block reads like one of the two numbers
        // is broken.
        assert_eq!(cgroup_memory(limits(8_000, 4_000), 8_000), None);
        assert_eq!(cgroup_memory(limits(9_000, 4_000), 8_000), None);
        assert_eq!(cgroup_memory(limits(0, 0), 8_000), None);
        // Not Linux: nothing to report.
        assert_eq!(cgroup_memory(None, 8_000), None);
    }

    #[test]
    fn the_longest_matching_mount_holds_the_path() {
        let root = Path::new("/");
        let docker = Path::new("/var/lib/docker");
        let mounts = [(root, 100, 40), (docker, 1_000, 100)];

        // Nested mounts: the workspace volume path resolves to the volume
        // filesystem, not to the root one that also prefixes it.
        let picked = select_disk(&mounts, Path::new("/var/lib/docker/volumes/x")).unwrap();
        assert_eq!(picked.mount_point, "/var/lib/docker");
        assert_eq!(picked.total_bytes, 1_000);
        assert_eq!(picked.used_bytes, 900);
        // The configured path is echoed back so an operator can tell which
        // path produced which mount.
        assert_eq!(picked.path, "/var/lib/docker/volumes/x");

        let picked = select_disk(&mounts, Path::new("/srv/data")).unwrap();
        assert_eq!(picked.mount_point, "/");
        assert_eq!(picked.used_bytes, 60);

        // Order must not decide the winner.
        let reversed = [(docker, 1_000, 100), (root, 100, 40)];
        assert_eq!(
            select_disk(&reversed, Path::new("/var/lib/docker/volumes/x"))
                .unwrap()
                .mount_point,
            "/var/lib/docker"
        );
    }

    #[test]
    fn an_unresolvable_path_yields_no_disk() {
        // Nothing mounted at all, and a relative path that prefixes nothing:
        // omitted rather than defaulted to some arbitrary filesystem.
        assert!(select_disk(&[], Path::new("/anything")).is_none());
        let mounts = [(Path::new("/mnt/data"), 10, 5)];
        assert!(select_disk(&mounts, Path::new("/other")).is_none());
    }

    #[test]
    fn disk_is_omitted_unless_configured() {
        // The guard the whole "no default mount" decision rests on.
        assert!(disk_path_from("").is_none());
        assert!(disk_path_from("   ").is_none());
        assert_eq!(
            disk_path_from("/var/lib/docker"),
            Some(PathBuf::from("/var/lib/docker"))
        );
    }

    /// `disk_path` reads the environment, which is process-global and so
    /// unsafe to mutate from a test that runs beside others. This mirrors its
    /// trimming rules on an injected value.
    fn disk_path_from(raw: &str) -> Option<PathBuf> {
        let trimmed = raw.trim();
        (!trimmed.is_empty()).then(|| PathBuf::from(trimmed))
    }

    /// The sampler must produce plausible figures on the machine running the
    /// tests, whatever that machine is. Guards against a refresh call being
    /// dropped, which would silently report zeros forever.
    #[test]
    fn a_sample_reports_real_memory_and_cpu_bounds() {
        let mut sampler = SystemSampler {
            system: System::new(),
            scope: MetricScope::Host,
            disk_path: None,
        };
        let usage = sampler.sample();
        assert!(usage.memory_total_bytes > 0);
        assert!(usage.memory_used_bytes <= usage.memory_total_bytes);
        assert!(usage.cpu_count > 0);
        // Averaged across cores, so it cannot exceed 100 however busy the box
        // is. The first sample has no delta to compare against, hence 0.0 is
        // legitimate here.
        assert!((0.0..=100.0).contains(&usage.cpu_percent));
        assert!(usage.disk.is_none());
    }
}
