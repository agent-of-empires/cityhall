//! Per-user session activity for the admin workspaces page.
//!
//! A background sampler asks each RUNNING workspace's aoe for its session list
//! and publishes a small summary: counts, pending approvals, last session
//! access, last error. Rules that keep this safe to run unattended:
//!
//! - Read-only and passive. It never starts a workspace (no `ensure_started`,
//!   no `get_or_create`), never touches [`crate::state::ActivityRegistry`], and
//!   never stops anything, so a poll cannot keep a workspace alive or kill one.
//! - Only the four fields needed are declared on [`SessionRow`]. Titles,
//!   prompts and transcripts are never deserialized, because serde skips
//!   fields the target type does not name.
//! - The bearer lives only in a [`Secret`] and is never logged or returned.
//! - One 401 parks that user until their runtime changes: aoe rate-limits
//!   failed auths per proxy IP, so repeats would lock out every workspace.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use chrono::{DateTime, Utc};
use sea_orm::EntityTrait;
use serde::de::IgnoredAny;
use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::crypto::Secret;
use crate::entities::workspace;
use crate::orchestrator::{proxy_allowed_host, WorkspaceStatus};
use crate::state::AppState;
use crate::workspaces::decrypt_daemon_token;

/// Matched to the dashboard's own poll.
const POLL_INTERVAL: Duration = Duration::from_secs(10);
/// Per aoe call, so one wedged workspace cannot stall a tick.
const CALL_TIMEOUT: Duration = Duration::from_secs(3);
const CONCURRENCY: usize = 8;

/// The only parts of an aoe session row CityHall reads.
#[derive(Deserialize)]
struct SessionRow {
    #[serde(default)]
    status: String,
    /// Stamped by aoe on user interaction (attach, send), not on agent output.
    last_accessed_at: Option<String>,
    /// Only counted: the approvals' tool names and targets stay unread.
    #[serde(default)]
    pending_approvals: Vec<IgnoredAny>,
    last_error: Option<String>,
}

#[derive(Deserialize)]
struct SessionList {
    sessions: Vec<SessionRow>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Ok,
    /// The workspace did not answer, or answered something other than a list.
    Unreachable,
    /// aoe rejected the bearer; not polled again until the runtime changes.
    AuthError,
    NotRunning,
}

/// What one workspace's sessions add up to. All zero or empty unless the
/// user's state is `ok`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Summary {
    pub running_sessions: u32,
    pub waiting_sessions: u32,
    pub pending_approvals: u32,
    /// The newest `last_accessed_at` across sessions.
    pub last_session_access: Option<DateTime<Utc>>,
    /// The error of the most recently accessed session that has one.
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct UserActivity {
    pub user_id: i32,
    pub state: State,
    #[serde(flatten)]
    pub summary: Summary,
}

/// `users` holds only workspaces polled on the last tick (running ones); a user
/// absent from it is not running. `sampled_at` is `None` until the first tick.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Snapshot {
    pub sampled_at: Option<DateTime<Utc>>,
    pub users: Vec<UserActivity>,
}

/// The published snapshot, same shape as [`crate::metrics::Metrics`].
#[derive(Default)]
pub struct ActivityCache {
    snapshot: RwLock<Arc<Snapshot>>,
}

impl ActivityCache {
    pub fn snapshot(&self) -> Arc<Snapshot> {
        self.snapshot.read().unwrap().clone()
    }

    fn publish(&self, snapshot: Snapshot) {
        *self.snapshot.write().unwrap() = Arc::new(snapshot);
    }
}

fn parse_time(raw: &Option<String>) -> Option<DateTime<Utc>> {
    raw.as_deref()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|t| t.with_timezone(&Utc))
}

fn summarize(sessions: &[SessionRow]) -> Summary {
    let count = |status: &str| {
        sessions
            .iter()
            .filter(|s| s.status.eq_ignore_ascii_case(status))
            .count() as u32
    };
    Summary {
        running_sessions: count("running"),
        waiting_sessions: count("waiting"),
        pending_approvals: sessions
            .iter()
            .map(|s| s.pending_approvals.len() as u32)
            .sum(),
        last_session_access: sessions
            .iter()
            .filter_map(|s| parse_time(&s.last_accessed_at))
            .max(),
        last_error: sessions
            .iter()
            .filter(|s| s.last_error.is_some())
            .max_by_key(|s| parse_time(&s.last_accessed_at))
            .and_then(|s| s.last_error.clone()),
    }
}

/// A bearer aoe refused, and the runtime it refused it on.
struct AuthMark {
    addr: String,
    token: Secret,
}

/// Per-user sampler memory, private to this module. The address comes from
/// `Orchestrator::status` on a miss or a connection failure, never from
/// `EndpointCache`.
#[derive(Default)]
struct Entry {
    addr: Option<String>,
    auth_error: Option<AuthMark>,
}

#[derive(Default)]
struct Poller {
    entries: HashMap<i32, Entry>,
}

enum Fetch {
    Sessions(Vec<SessionRow>),
    Unauthorized,
    Failed { connect: bool },
}

async fn fetch(state: &AppState, addr: &str, token: &Secret) -> Fetch {
    let response = state
        .proxy_client
        .get(format!("http://{addr}/api/sessions"))
        // aoe's DNS-rebinding gate answers any other Host 403.
        .header("host", proxy_allowed_host())
        .bearer_auth(token.expose())
        .timeout(CALL_TIMEOUT)
        .send()
        .await;
    let response = match response {
        Ok(r) => r,
        Err(e) => {
            return Fetch::Failed {
                connect: e.is_connect(),
            }
        }
    };
    if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Fetch::Unauthorized;
    }
    if !response.status().is_success() {
        return Fetch::Failed { connect: false };
    }
    let list = match response.bytes().await {
        Ok(body) => serde_json::from_slice::<SessionList>(&body).ok(),
        Err(_) => None,
    };
    match list {
        Some(list) => Fetch::Sessions(list.sessions),
        None => Fetch::Failed { connect: false },
    }
}

/// The workspace's current address, or why it has none.
async fn refresh(state: &AppState, user_id: i32) -> Result<String, State> {
    match state.orchestrator.status(user_id).await {
        Ok(WorkspaceStatus::Running { addr }) => Ok(addr),
        Ok(_) => Err(State::NotRunning),
        Err(_) => Err(State::Unreachable),
    }
}

fn bare(user_id: i32, state: State) -> UserActivity {
    UserActivity {
        user_id,
        state,
        summary: Summary::default(),
    }
}

/// Poll one running workspace. `None` when the user has no workspace row.
async fn poll_user(
    state: &AppState,
    user_id: i32,
    mut entry: Entry,
) -> (Entry, Option<UserActivity>) {
    let row = match workspace::Entity::find_by_id(user_id).one(&state.db).await {
        Ok(Some(row)) => row,
        Ok(None) => return (Entry::default(), None),
        Err(_) => return (entry, Some(bare(user_id, State::Unreachable))),
    };
    let Ok(token) = decrypt_daemon_token(&row).map(Secret::new) else {
        return (entry, Some(bare(user_id, State::Unreachable)));
    };

    if let Some(mark) = entry.auth_error.take() {
        // Parked: only a different runtime (another address, or a rotated
        // bearer) lifts it. A status error keeps the park, because the
        // runtime was not seen to change.
        match refresh(state, user_id).await {
            Ok(addr) if addr == mark.addr && token == mark.token => {
                entry.auth_error = Some(mark);
                return (entry, Some(bare(user_id, State::AuthError)));
            }
            Ok(addr) => entry.addr = Some(addr),
            Err(State::NotRunning) => {
                return (Entry::default(), Some(bare(user_id, State::NotRunning)));
            }
            Err(unreachable) => {
                entry.auth_error = Some(mark);
                return (entry, Some(bare(user_id, unreachable)));
            }
        }
    }

    let mut addr = match entry.addr.clone() {
        Some(addr) => addr,
        None => match refresh(state, user_id).await {
            Ok(addr) => addr,
            Err(st) => return (Entry::default(), Some(bare(user_id, st))),
        },
    };
    entry.addr = Some(addr.clone());
    let mut result = fetch(state, &addr, &token).await;
    if matches!(result, Fetch::Failed { connect: true }) {
        match refresh(state, user_id).await {
            Ok(fresh) if fresh != addr => {
                result = fetch(state, &fresh, &token).await;
                addr = fresh.clone();
                entry.addr = Some(fresh);
            }
            Ok(_) => {}
            Err(st) => return (Entry::default(), Some(bare(user_id, st))),
        }
    }

    let activity = match result {
        Fetch::Sessions(sessions) => UserActivity {
            user_id,
            state: State::Ok,
            summary: summarize(&sessions),
        },
        Fetch::Unauthorized => {
            tracing::warn!(user_id, "workspace rejected the activity poll; not polling it again until its runtime changes");
            entry.auth_error = Some(AuthMark { addr, token });
            bare(user_id, State::AuthError)
        }
        Fetch::Failed { .. } => bare(user_id, State::Unreachable),
    };
    (entry, Some(activity))
}

/// One tick over the running workspaces, at most [`CONCURRENCY`] at a time.
async fn poll_once(state: &AppState, poller: &mut Poller, running: &[i32]) -> Vec<UserActivity> {
    poller.entries.retain(|id, _| running.contains(id));
    let permits = Arc::new(Semaphore::new(CONCURRENCY));
    let mut tasks = JoinSet::new();
    for &user_id in running {
        let entry = poller.entries.remove(&user_id).unwrap_or_default();
        let (state, permits) = (state.clone(), permits.clone());
        tasks.spawn(async move {
            let _permit = permits.acquire().await;
            (user_id, poll_user(&state, user_id, entry).await)
        });
    }
    let mut users = Vec::with_capacity(running.len());
    while let Some(Ok((user_id, (entry, activity)))) = tasks.join_next().await {
        poller.entries.insert(user_id, entry);
        users.extend(activity);
    }
    users.sort_by_key(|u| u.user_id);
    users
}

/// Background loop publishing the activity snapshot. Which workspaces are
/// running comes from the metrics sampler's last snapshot, so this adds no
/// backend status round-trip per stopped workspace.
pub async fn sampler(state: AppState) {
    let mut poller = Poller::default();
    let mut interval = tokio::time::interval(POLL_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        interval.tick().await;
        let running: Vec<i32> = state
            .metrics
            .snapshot()
            .runtimes
            .iter()
            .filter(|r| r.running)
            .map(|r| r.user_id)
            .collect();
        let users = poll_once(&state, &mut poller, &running).await;
        state.session_activity.publish(Snapshot {
            sampled_at: Some(Utc::now()),
            users,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    use axum::http::{HeaderMap, StatusCode};
    use axum::Router;
    use base64::engine::general_purpose::STANDARD as B64;
    use base64::Engine;
    use sea_orm::{ActiveModelTrait, ConnectOptions, Database, DatabaseConnection, Set};
    use sea_orm_migration::MigratorTrait;

    use crate::auth::AuthUser;
    use crate::entities::{role, workspace_settings};
    use crate::error::AppError;
    use crate::migration::Migrator;
    use crate::orchestrator::{
        Orchestrator, OrchestratorError, TelemetryPolicy, WorkspaceMode, WorkspaceSpec,
    };
    use crate::rbac::Perms;
    use crate::workspaces::{get_or_create, SETTINGS_ID};

    fn row(
        status: &str,
        accessed: Option<&str>,
        approvals: usize,
        error: Option<&str>,
    ) -> SessionRow {
        SessionRow {
            status: status.to_string(),
            last_accessed_at: accessed.map(String::from),
            pending_approvals: (0..approvals).map(|_| IgnoredAny).collect(),
            last_error: error.map(String::from),
        }
    }

    #[test]
    fn summarize_counts_and_picks_the_newest_access_and_its_error() {
        let sessions = [
            row(
                "running",
                Some("2026-01-01T10:00:00Z"),
                1,
                Some("old error"),
            ),
            row(
                "Waiting",
                Some("2026-01-02T10:00:00Z"),
                2,
                Some("new error"),
            ),
            row("idle", None, 0, None),
            row("running", Some("garbage"), 0, None),
        ];
        let got = summarize(&sessions);
        assert_eq!(got.running_sessions, 2);
        assert_eq!(got.waiting_sessions, 1);
        assert_eq!(got.pending_approvals, 3);
        assert_eq!(
            got.last_session_access.unwrap().to_rfc3339(),
            "2026-01-02T10:00:00+00:00"
        );
        assert_eq!(got.last_error.as_deref(), Some("new error"));
        assert_eq!(summarize(&[]), Summary::default());
    }

    #[test]
    fn only_the_needed_fields_are_read_from_a_session() {
        let json = r#"{"sessions":[{"id":"a","title":"secret plan","status":"waiting",
            "last_accessed_at":"2026-01-01T00:00:00Z","last_error":null,
            "pending_approvals":[{"nonce":"n","tool_name":"rm","target":"/","destructive":true}]}],
            "trashed_count":0}"#;
        let list: SessionList = serde_json::from_str(json).unwrap();
        let got = summarize(&list.sessions);
        assert_eq!((got.waiting_sessions, got.pending_approvals), (1, 1));
    }

    /// A stand-in for a workspace's aoe: counts requests and answers a fixed
    /// status and body.
    struct Daemon {
        addr: String,
        hits: Arc<AtomicUsize>,
        seen: Arc<Mutex<Vec<(String, String)>>>,
    }

    async fn daemon(status: StatusCode, body: &'static str) -> Daemon {
        let hits = Arc::new(AtomicUsize::new(0));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let app = Router::new().fallback({
            let (hits, seen) = (hits.clone(), seen.clone());
            move |headers: HeaderMap| {
                let header = |n: &str| {
                    headers
                        .get(n)
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("")
                        .to_string()
                };
                seen.lock()
                    .unwrap()
                    .push((header("authorization"), header("host")));
                hits.fetch_add(1, Ordering::SeqCst);
                async move { (status, body) }
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Daemon { addr, hits, seen }
    }

    const LIST: &str = r#"{"sessions":[{"id":"a","status":"running"}]}"#;

    /// Reports whatever address the test sets, and counts any attempt to start
    /// or stop a workspace.
    #[derive(Default)]
    struct Fake {
        addr: Mutex<String>,
        starts: AtomicUsize,
        stops: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl Orchestrator for Fake {
        async fn ensure_started(&self, _: &WorkspaceSpec) -> Result<String, OrchestratorError> {
            self.starts.fetch_add(1, Ordering::SeqCst);
            Ok(self.addr.lock().unwrap().clone())
        }
        async fn stop(&self, _: i32) -> Result<(), OrchestratorError> {
            self.stops.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        async fn destroy(&self, _: i32) -> Result<(), OrchestratorError> {
            Ok(())
        }
        async fn status(&self, _: i32) -> Result<WorkspaceStatus, OrchestratorError> {
            Ok(WorkspaceStatus::Running {
                addr: self.addr.lock().unwrap().clone(),
            })
        }
        async fn running_mode(&self, _: i32) -> Result<Option<WorkspaceMode>, OrchestratorError> {
            Ok(None)
        }
    }

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

    async fn user_with_perms(
        db: &DatabaseConnection,
        name: &str,
        perms: &[&str],
    ) -> crate::entities::user::Model {
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
    }

    async fn state_with(db: DatabaseConnection, orch: Arc<Fake>) -> AppState {
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

    /// A state with one user whose workspace row exists, and a fake backend
    /// reporting `addr`.
    async fn fixture(addr: &str) -> (AppState, Arc<Fake>, i32) {
        let db = setup().await;
        let user = user_with_perms(&db, "dev", &[]).await;
        get_or_create(&db, user.id).await.unwrap();
        let orch = Arc::new(Fake {
            addr: Mutex::new(addr.to_string()),
            ..Default::default()
        });
        (state_with(db, orch.clone()).await, orch, user.id)
    }

    #[tokio::test]
    async fn a_running_workspace_is_summarized_with_its_bearer_and_host() {
        let _key = test_key();
        let d = daemon(StatusCode::OK, LIST).await;
        let (state, _orch, uid) = fixture(&d.addr).await;
        let token = decrypt_daemon_token(&get_or_create(&state.db, uid).await.unwrap()).unwrap();

        let users = poll_once(&state, &mut Poller::default(), &[uid]).await;

        assert_eq!(users.len(), 1);
        assert_eq!(users[0].state, State::Ok);
        assert_eq!(users[0].summary.running_sessions, 1);
        let seen = d.seen.lock().unwrap();
        assert_eq!(seen[0].0, format!("Bearer {token}"));
        assert_eq!(seen[0].1, proxy_allowed_host());
    }

    #[tokio::test]
    async fn one_401_parks_the_user_and_a_new_runtime_resumes_polling() {
        let _key = test_key();
        let rejecting = daemon(StatusCode::UNAUTHORIZED, "").await;
        let (state, orch, uid) = fixture(&rejecting.addr).await;
        let mut poller = Poller::default();

        let first = poll_once(&state, &mut poller, &[uid]).await;
        assert_eq!(first[0].state, State::AuthError);
        assert_eq!(rejecting.hits.load(Ordering::SeqCst), 1);

        // Same runtime: parked, the daemon is not asked again.
        let second = poll_once(&state, &mut poller, &[uid]).await;
        assert_eq!(second[0].state, State::AuthError);
        assert_eq!(rejecting.hits.load(Ordering::SeqCst), 1);

        // The runtime changed (new address): polling resumes.
        let healthy = daemon(StatusCode::OK, LIST).await;
        *orch.addr.lock().unwrap() = healthy.addr.clone();
        let third = poll_once(&state, &mut poller, &[uid]).await;
        assert_eq!(third[0].state, State::Ok);
        assert_eq!(healthy.hits.load(Ordering::SeqCst), 1);
        assert_eq!(rejecting.hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_rotated_bearer_lifts_the_park() {
        let _key = test_key();
        let d = daemon(StatusCode::UNAUTHORIZED, "").await;
        let (state, _orch, uid) = fixture(&d.addr).await;
        let mut poller = Poller::default();
        poll_once(&state, &mut poller, &[uid]).await;
        assert_eq!(d.hits.load(Ordering::SeqCst), 1);

        let row = get_or_create(&state.db, uid).await.unwrap();
        let mut active: workspace::ActiveModel = row.into();
        active.daemon_token = Set(Some(
            crate::crypto::encrypt(
                &"f".repeat(64),
                &crate::crypto::Aad::DaemonToken { user_id: uid },
            )
            .unwrap(),
        ));
        active.update(&state.db).await.unwrap();

        poll_once(&state, &mut poller, &[uid]).await;
        assert_eq!(d.hits.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_dead_address_is_unreachable_and_retried_next_tick() {
        let _key = test_key();
        let (state, _orch, uid) = fixture("127.0.0.1:1").await;
        let mut poller = Poller::default();
        let first = poll_once(&state, &mut poller, &[uid]).await;
        assert_eq!(first[0].state, State::Unreachable);
        let second = poll_once(&state, &mut poller, &[uid]).await;
        assert_eq!(second[0].state, State::Unreachable);
    }

    #[tokio::test]
    async fn polling_never_starts_stops_or_touches_a_workspace() {
        let _key = test_key();
        let d = daemon(StatusCode::OK, LIST).await;
        let (state, orch, uid) = fixture(&d.addr).await;
        let before = get_or_create(&state.db, uid).await.unwrap().last_active_at;

        let mut poller = Poller::default();
        poll_once(&state, &mut poller, &[uid]).await;
        poll_once(&state, &mut poller, &[uid]).await;

        assert!(state.activity.get(uid).is_none(), "idle-stop clock touched");
        assert_eq!(orch.starts.load(Ordering::SeqCst), 0);
        assert_eq!(orch.stops.load(Ordering::SeqCst), 0);
        let after = workspace::Entity::find_by_id(uid)
            .one(&state.db)
            .await
            .unwrap()
            .unwrap()
            .last_active_at;
        assert_eq!(before, after);
    }

    fn caller(user: crate::entities::user::Model, perms: &[&str]) -> AuthUser {
        let permissions =
            crate::rbac::encode(&perms.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        let role = role::Model {
            id: 1,
            name: "r".into(),
            description: None,
            permissions,
            is_system: false,
            created_at: Utc::now(),
        };
        AuthUser {
            user,
            perms: Perms::from_role(Some(&role)),
        }
    }

    #[tokio::test]
    async fn the_endpoint_needs_workspaces_activity() {
        use crate::handlers::workspaces::activity;
        let db = setup().await;
        let user = user_with_perms(&db, "dev", &[]).await;
        let state = state_with(db, Arc::default()).await;
        state.session_activity.publish(Snapshot {
            sampled_at: Some(Utc::now()),
            users: vec![bare(7, State::Ok)],
        });

        // `workspaces.read` is not enough: this is a separate exposure.
        let denied = activity(
            axum::extract::State(state.clone()),
            caller(user.clone(), &["workspaces.read"]),
        )
        .await;
        assert!(matches!(denied, Err(AppError::Forbidden(_))));

        let allowed = activity(
            axum::extract::State(state),
            caller(user, &["workspaces.activity"]),
        )
        .await
        .unwrap();
        assert_eq!(allowed.0.users[0].user_id, 7);
    }
}
