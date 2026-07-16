//! Shared application state threaded through axum.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use axum::extract::FromRef;
use sea_orm::DatabaseConnection;

use crate::orchestrator::{Orchestrator, ProvisioningRegistry};

#[derive(Clone)]
pub struct AppState {
    pub db: DatabaseConnection,
    pub orchestrator: Arc<dyn Orchestrator>,
    pub activity: Arc<ActivityRegistry>,
    pub locks: Arc<WorkspaceLocks>,
    pub endpoints: Arc<EndpointCache>,
    /// Read side of background artifact provisioning (pulls, builds,
    /// downloads); the backends write to it.
    pub provisioning: Arc<ProvisioningRegistry>,
    /// Cached aoe release discovery feeding the version dropdowns.
    pub versions: Arc<crate::workspaces::VersionCache>,
    /// Client for proxied HTTP requests to workspaces (no redirects, HTTP/1.1
    /// so WebSocket upgrades tunnel through).
    pub proxy_client: reqwest::Client,
}

/// Lets existing handlers keep extracting `State<DatabaseConnection>`.
impl FromRef<AppState> for DatabaseConnection {
    fn from_ref(state: &AppState) -> Self {
        state.db.clone()
    }
}

/// In-memory per-user workspace activity. The hot path (every proxied request)
/// only touches memory; the idle sweeper checkpoints coarse timestamps to the
/// database.
#[derive(Default)]
pub struct ActivityRegistry {
    entries: Mutex<HashMap<i32, ActivityEntry>>,
}

#[derive(Clone, Copy)]
pub struct ActivityEntry {
    pub last_seen: Instant,
    /// Open proxied WebSocket tunnels; a workspace with a live tunnel is never
    /// idle-stopped even if no HTTP requests arrive.
    pub active_websockets: u32,
}

impl ActivityRegistry {
    pub fn touch(&self, user_id: i32) {
        let mut entries = self.entries.lock().unwrap();
        let entry = entries.entry(user_id).or_insert(ActivityEntry {
            last_seen: Instant::now(),
            active_websockets: 0,
        });
        entry.last_seen = Instant::now();
    }

    pub fn websocket_started(&self, user_id: i32) {
        let mut entries = self.entries.lock().unwrap();
        let entry = entries.entry(user_id).or_insert(ActivityEntry {
            last_seen: Instant::now(),
            active_websockets: 0,
        });
        entry.active_websockets += 1;
        entry.last_seen = Instant::now();
    }

    pub fn websocket_ended(&self, user_id: i32) {
        let mut entries = self.entries.lock().unwrap();
        if let Some(entry) = entries.get_mut(&user_id) {
            entry.active_websockets = entry.active_websockets.saturating_sub(1);
            entry.last_seen = Instant::now();
        }
    }

    pub fn get(&self, user_id: i32) -> Option<ActivityEntry> {
        self.entries.lock().unwrap().get(&user_id).copied()
    }
}

/// Per-user cache of the reachable workspace address, keyed by the spec that
/// produced it. Without it every proxied request (each asset, each API call)
/// pays a backend round-trip (docker/kubectl shell-outs); with it only the
/// first request after a start, stop, or failure reconciles. Stop and destroy
/// invalidate; a failed proxied dial invalidates so the next request heals
/// out-of-band crashes.
#[derive(Default)]
pub struct EndpointCache {
    entries: Mutex<HashMap<i32, CachedEndpoint>>,
}

#[derive(Clone)]
struct CachedEndpoint {
    addr: String,
    version: String,
    image: String,
}

impl EndpointCache {
    /// The cached address, if it was produced by a spec with the same version
    /// and image (a pin or template change must miss and reconcile).
    pub fn get(&self, user_id: i32, version: &str, image: &str) -> Option<String> {
        self.entries
            .lock()
            .unwrap()
            .get(&user_id)
            .filter(|e| e.version == version && e.image == image)
            .map(|e| e.addr.clone())
    }

    pub fn put(&self, user_id: i32, version: &str, image: &str, addr: String) {
        self.entries.lock().unwrap().insert(
            user_id,
            CachedEndpoint {
                addr,
                version: version.to_string(),
                image: image.to_string(),
            },
        );
    }

    pub fn invalidate(&self, user_id: i32) {
        self.entries.lock().unwrap().remove(&user_id);
    }
}

/// Per-user async locks serializing workspace lifecycle transitions, so a
/// proxy-triggered start cannot race the idle sweeper's stop (or a concurrent
/// first-page-load double-start).
#[derive(Default)]
pub struct WorkspaceLocks {
    locks: Mutex<HashMap<i32, Arc<tokio::sync::Mutex<()>>>>,
}

impl WorkspaceLocks {
    pub fn lock_for(&self, user_id: i32) -> Arc<tokio::sync::Mutex<()>> {
        self.locks
            .lock()
            .unwrap()
            .entry(user_id)
            .or_default()
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn websocket_leases_track_open_tunnels() {
        let reg = ActivityRegistry::default();
        reg.websocket_started(1);
        reg.websocket_started(1);
        assert_eq!(reg.get(1).unwrap().active_websockets, 2);
        reg.websocket_ended(1);
        assert_eq!(reg.get(1).unwrap().active_websockets, 1);
        reg.websocket_ended(1);
        // Underflow is clamped rather than wrapping.
        reg.websocket_ended(1);
        assert_eq!(reg.get(1).unwrap().active_websockets, 0);
    }

    #[test]
    fn unknown_user_has_no_entry() {
        let reg = ActivityRegistry::default();
        assert!(reg.get(99).is_none());
    }

    #[test]
    fn endpoint_cache_hits_only_on_matching_spec() {
        let cache = EndpointCache::default();
        assert!(cache.get(1, "v1", "img:v1").is_none());

        cache.put(1, "v1", "img:v1", "127.0.0.1:5000".to_string());
        assert_eq!(
            cache.get(1, "v1", "img:v1").as_deref(),
            Some("127.0.0.1:5000")
        );
        // A version pin or template change must miss and reconcile.
        assert!(cache.get(1, "v2", "img:v2").is_none());
        assert!(cache.get(1, "v1", "other:v1").is_none());

        cache.invalidate(1);
        assert!(cache.get(1, "v1", "img:v1").is_none());
    }
}
