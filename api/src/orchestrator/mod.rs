//! Backend-agnostic orchestration seam for per-user aoe workspaces.
//!
//! A workspace is one long-lived aoe instance (container, process, pod...)
//! per user with a persistent data volume. Backends implement [`Orchestrator`];
//! CityHall stores only intent (pinned version, activity) in the database and
//! treats the runtime as the source of truth for liveness.

pub mod docker;
pub mod kubernetes;
#[cfg(unix)]
pub mod process;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

/// Everything a backend needs to materialize a user's workspace.
#[derive(Clone, Debug)]
pub struct WorkspaceSpec {
    pub user_id: i32,
    /// Fully rendered image reference (docker/kube backends).
    pub image: String,
    /// The aoe version the image serves; used to detect version drift.
    pub version: String,
    /// Where the workspace fetches its config bundle, when one is configured.
    pub bundle: Option<BundleAccess>,
    /// Agent credentials to inject, and a fingerprint identifying this exact set
    /// so a backend can detect a credential change as drift. `AgentEnv`'s
    /// `Debug` redacts every value, so `WorkspaceSpec`'s derived `Debug` stays
    /// safe; a bare `Vec<(String, String)>` here would be a leak waiting for a
    /// future tracing call.
    pub agent_env: crate::agent_credentials::AgentEnv,
}

/// How a workspace reaches its own config bundle.
///
/// Delivered as two environment variables rather than a file written into the
/// workspace, so one implementation covers every backend (each already builds
/// its own env list) and editing the bundle in CityHall plus a restart is enough
/// to roll it out, with no container recreation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BundleAccess {
    /// Absolute URL of CityHall's bundle endpoint, reachable from inside the
    /// workspace.
    pub url: String,
    /// Bearer token identifying which user's bundle to serve.
    pub token: String,
}

/// Runtime state of a workspace as reported by the backend.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkspaceStatus {
    /// No runtime object exists (never started, or destroyed).
    NotCreated,
    /// Exists but is not running; the data volume is retained.
    Stopped,
    /// Running and reachable at `addr` (`host:port`).
    Running { addr: String },
}

#[derive(Debug)]
pub enum OrchestratorError {
    /// The workspace artifact (docker image, aoe binary...) is not available;
    /// carries operator guidance.
    ArtifactMissing(String),
    /// The artifact is being fetched or built in the background; carries a
    /// progress message. Callers should retry shortly.
    Provisioning(String),
    /// Any other backend failure (daemon down, command failed...).
    Runtime(String),
}

impl std::fmt::Display for OrchestratorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OrchestratorError::ArtifactMissing(m)
            | OrchestratorError::Provisioning(m)
            | OrchestratorError::Runtime(m) => {
                write!(f, "{m}")
            }
        }
    }
}

impl std::error::Error for OrchestratorError {}

/// Lifecycle contract every workspace backend implements. All operations are
/// idempotent: stopping a missing workspace succeeds, destroying twice
/// succeeds.
#[async_trait]
pub trait Orchestrator: Send + Sync {
    /// Reconcile the user's workspace to "running with `spec`" and return its
    /// reachable address. Recreates the runtime object (keeping the volume)
    /// when the running version, OR the agent-credential fingerprint, differs
    /// from what is currently running.
    async fn ensure_started(&self, spec: &WorkspaceSpec) -> Result<String, OrchestratorError>;

    /// Stop the workspace, keeping its data volume.
    async fn stop(&self, user_id: i32) -> Result<(), OrchestratorError>;

    /// Remove the workspace AND its data volume.
    async fn destroy(&self, user_id: i32) -> Result<(), OrchestratorError>;

    /// Current runtime state.
    async fn status(&self, user_id: i32) -> Result<WorkspaceStatus, OrchestratorError>;
}

/// The backend selected by `WORKSPACE_BACKEND` (default `docker`), plus the
/// provisioning registry it reports slow artifact jobs through. Invalid
/// values fail CityHall startup instead of surfacing on first workspace use.
pub fn from_env() -> Result<(Arc<dyn Orchestrator>, Arc<ProvisioningRegistry>), String> {
    let registry = Arc::new(ProvisioningRegistry::default());
    let backend = std::env::var("WORKSPACE_BACKEND").unwrap_or_else(|_| "docker".to_string());
    let orchestrator: Arc<dyn Orchestrator> = match backend.as_str() {
        "docker" => Arc::new(docker::DockerCliOrchestrator::from_env(registry.clone())),
        "kubernetes" => Arc::new(kubernetes::KubectlOrchestrator::from_env()),
        #[cfg(unix)]
        "process" => Arc::new(process::ProcessOrchestrator::from_env(registry.clone())),
        other => {
            return Err(format!(
                "unknown WORKSPACE_BACKEND '{other}' (expected docker, kubernetes, or process)"
            ))
        }
    };
    Ok((orchestrator, registry))
}

/// Registry key for a version's process-backend binary; shared with the
/// workspaces handler so the admin list can look up progress for a user's
/// effective version whichever backend runs it.
pub fn binary_key(version: &str) -> String {
    format!("aoe-binary-{version}")
}

/// How long a failed provisioning attempt stays sticky before a new request
/// may retry it. Prevents every page load from re-running a doomed
/// multi-minute build while keeping the failure visible.
const FAILED_RETRY_AFTER: Duration = Duration::from_secs(60);

/// What a backend should do after asking to begin provisioning an artifact.
pub enum Begin {
    /// No job was running: the caller must spawn one.
    Started,
    /// A job is already running with this progress message.
    AlreadyRunning(String),
    /// The last attempt failed recently; carries its error.
    RecentlyFailed(String),
}

enum ProvisioningState {
    Running(String),
    Failed {
        message: String,
        at: tokio::time::Instant,
    },
}

/// Tracks background artifact provisioning (image pulls/builds, binary
/// downloads) by artifact key, single-flight per artifact with sticky
/// failures. Shared between the backends (writers) and the admin API
/// (reader).
#[derive(Default)]
pub struct ProvisioningRegistry {
    entries: std::sync::Mutex<std::collections::HashMap<String, ProvisioningState>>,
}

impl ProvisioningRegistry {
    /// Atomically claim the right to provision `key`, marking it running.
    pub fn begin(&self, key: &str, message: &str) -> Begin {
        let mut entries = self.entries.lock().unwrap();
        match entries.get(key) {
            Some(ProvisioningState::Running(msg)) => Begin::AlreadyRunning(msg.clone()),
            Some(ProvisioningState::Failed { message, at })
                if at.elapsed() < FAILED_RETRY_AFTER =>
            {
                Begin::RecentlyFailed(message.clone())
            }
            _ => {
                entries.insert(
                    key.to_string(),
                    ProvisioningState::Running(message.to_string()),
                );
                Begin::Started
            }
        }
    }

    /// Update the progress message of a running job.
    pub fn progress(&self, key: &str, message: &str) {
        self.entries.lock().unwrap().insert(
            key.to_string(),
            ProvisioningState::Running(message.to_string()),
        );
    }

    pub fn succeed(&self, key: &str) {
        self.entries.lock().unwrap().remove(key);
    }

    pub fn fail(&self, key: &str, message: String) {
        self.entries.lock().unwrap().insert(
            key.to_string(),
            ProvisioningState::Failed {
                message,
                at: tokio::time::Instant::now(),
            },
        );
    }

    /// Current state of `key` for display: `(message, failed)`.
    pub fn state(&self, key: &str) -> Option<(String, bool)> {
        match self.entries.lock().unwrap().get(key) {
            Some(ProvisioningState::Running(msg)) => Some((msg.clone(), false)),
            Some(ProvisioningState::Failed { message, .. }) => Some((message.clone(), true)),
            None => None,
        }
    }
}

/// Render an image template by substituting the `{version}` placeholder.
pub fn render_image(template: &str, version: &str) -> String {
    template.replace("{version}", version)
}

/// Prefix marking a version as a build of an unreleased aoe commit rather than
/// a published release tag. The remainder is the commit sha, so an image tag
/// and a version drift label both name one immutable build even when the ref
/// it came from moves. A hyphen and not a colon, because a version becomes a
/// docker tag and a filesystem path component and a colon is legal in neither.
pub const GIT_VERSION_PREFIX: &str = "git-";

/// The commit a source-build version names, or `None` for a release tag.
pub fn git_version_sha(version: &str) -> Option<&str> {
    version.strip_prefix(GIT_VERSION_PREFIX)
}

/// The `Host` value browsers reach workspaces on, for aoe's DNS-rebinding
/// gate: `aoe serve --behind-proxy` refuses to start without at least one
/// `--allowed-host`, and the proxy forwards the public host it was called on.
/// A port in the value is harmless (aoe strips it before matching).
pub fn proxy_allowed_host() -> String {
    allowed_host_from_origin(&proxy_allowed_origin())
}

/// Default public origin: the proxy's own default bind, reached on loopback.
const DEFAULT_PROXY_ORIGIN: &str = "http://localhost:3001";

/// The browser `Origin` the proxy forwards to workspaces. aoe derives allowed
/// origins from `--allowed-host` only for the standard ports, so a proxy on
/// `:3001` (or any nonstandard port) has to be spelled out or every fetch and
/// WebSocket from the dashboard is refused.
pub fn proxy_allowed_origin() -> String {
    let origin = std::env::var("WORKSPACE_PROXY_PUBLIC_ORIGIN").unwrap_or_default();
    let origin = origin.trim().trim_end_matches('/');
    if origin.is_empty() {
        DEFAULT_PROXY_ORIGIN.to_string()
    } else {
        origin.to_string()
    }
}

/// Path CityHall serves a workspace's own config bundle on.
const BUNDLE_PATH: &str = "/api/workspace-bundle";

/// The bundle URL to hand a workspace, or `None` when no internal origin is
/// configured.
///
/// `WORKSPACE_BUNDLE_ORIGIN` is the origin a *workspace* uses to reach CityHall,
/// which is not the public one: on the docker backend it is the compose service
/// name on the shared network (`http://cityhall:3000`), and on kubernetes the
/// in-cluster Service. There is no safe default, because CityHall's own
/// container hostname is not resolvable by its peers, so an unset value simply
/// turns the feature off and workspaces start unconfigured as before.
pub fn bundle_url() -> Option<String> {
    let origin = std::env::var("WORKSPACE_BUNDLE_ORIGIN").unwrap_or_default();
    let origin = origin.trim().trim_end_matches('/');
    (!origin.is_empty()).then(|| format!("{origin}{BUNDLE_PATH}"))
}

/// The host part of a proxy origin.
fn allowed_host_from_origin(origin: &str) -> String {
    origin
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(origin)
        .split('/')
        .next()
        .unwrap_or_default()
        .to_string()
}

/// How long to wait for aoe to accept connections after a start.
const READY_TIMEOUT: Duration = Duration::from_secs(15);

/// Wait until the workspace answers HTTP so the first proxied request does
/// not race aoe's startup.
pub(crate) async fn wait_ready(addr: &str) -> Result<(), OrchestratorError> {
    let deadline = tokio::time::Instant::now() + READY_TIMEOUT;
    loop {
        if http_probe(addr).await {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(OrchestratorError::Runtime(format!(
                "workspace at {addr} did not become ready within {READY_TIMEOUT:?}"
            )));
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// Whether an HTTP server answers at `addr`. A bare TCP connect is not
/// enough: docker's userland proxy accepts connections on the published port
/// before the service inside the container listens, so the probe must
/// actually exchange bytes.
pub(crate) async fn http_probe(addr: &str) -> bool {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let probe = async {
        let mut stream = tokio::net::TcpStream::connect(addr).await.ok()?;
        stream
            .write_all(b"GET / HTTP/1.0\r\nHost: workspace\r\n\r\n")
            .await
            .ok()?;
        let mut buf = [0u8; 1];
        match stream.read(&mut buf).await {
            Ok(n) if n > 0 => Some(()),
            _ => None,
        }
    };
    tokio::time::timeout(Duration::from_secs(2), probe)
        .await
        .ok()
        .flatten()
        .is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowed_host_comes_from_the_proxy_origin() {
        assert_eq!(
            allowed_host_from_origin("https://ws.example.com"),
            "ws.example.com"
        );
        // A port is kept: aoe strips it before matching the Host header.
        assert_eq!(
            allowed_host_from_origin("http://localhost:3001"),
            "localhost:3001"
        );
    }

    #[test]
    fn render_image_substitutes_version() {
        assert_eq!(
            render_image("cityhall/aoe:{version}", "v1.2.3"),
            "cityhall/aoe:v1.2.3"
        );
        // A template without the placeholder pins every user to one image.
        assert_eq!(
            render_image("cityhall/aoe:latest", "v1"),
            "cityhall/aoe:latest"
        );
    }

    #[tokio::test]
    async fn provisioning_registry_is_single_flight_with_sticky_failures() {
        let reg = ProvisioningRegistry::default();
        assert!(reg.state("img").is_none());

        // First begin claims the job; a second sees it running.
        assert!(matches!(reg.begin("img", "pulling"), Begin::Started));
        assert!(matches!(
            reg.begin("img", "pulling"),
            Begin::AlreadyRunning(m) if m == "pulling"
        ));
        reg.progress("img", "building");
        assert_eq!(reg.state("img"), Some(("building".to_string(), false)));

        // Fresh failures are sticky: no immediate re-run, message visible.
        reg.fail("img", "boom".to_string());
        assert!(matches!(
            reg.begin("img", "pulling"),
            Begin::RecentlyFailed(m) if m == "boom"
        ));
        assert_eq!(reg.state("img"), Some(("boom".to_string(), true)));

        // Success clears the entry; unrelated keys are independent.
        assert!(matches!(reg.begin("other", "pulling"), Begin::Started));
        reg.succeed("other");
        assert!(reg.state("other").is_none());
    }

    #[tokio::test]
    async fn http_probe_requires_a_response_not_just_a_connect() {
        use tokio::io::AsyncWriteExt;

        // Accepts and answers: ready.
        let responder = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = responder.local_addr().unwrap().to_string();
        tokio::spawn(async move {
            let (mut sock, _) = responder.accept().await.unwrap();
            let _ = sock.write_all(b"HTTP/1.0 200 OK\r\n\r\n").await;
        });
        assert!(http_probe(&addr).await);

        // Accepts but closes without a byte (docker-proxy with no backend
        // yet): not ready.
        let closer = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = closer.local_addr().unwrap().to_string();
        tokio::spawn(async move {
            let (sock, _) = closer.accept().await.unwrap();
            drop(sock);
        });
        assert!(!http_probe(&addr).await);
    }

    #[test]
    fn only_a_prefixed_version_names_a_commit() {
        assert_eq!(git_version_sha("git-abc123"), Some("abc123"));
        assert_eq!(git_version_sha("v1.13.2"), None);
        // Not a source build: the marker is a prefix, not a substring.
        assert_eq!(git_version_sha("v1-git-2"), None);
    }
}
