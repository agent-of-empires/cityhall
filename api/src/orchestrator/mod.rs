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
    /// when the running version differs from `spec.version`.
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
}
