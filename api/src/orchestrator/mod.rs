//! Backend-agnostic orchestration seam for per-user aoe workspaces.
//!
//! A workspace is one long-lived aoe instance (container, process, pod...)
//! per user with a persistent data volume. Backends implement [`Orchestrator`];
//! CityHall stores only intent (pinned version, activity) in the database and
//! treats the runtime as the source of truth for liveness.

pub mod docker;
pub mod kubernetes;

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
    /// Any other backend failure (daemon down, command failed...).
    Runtime(String),
}

impl std::fmt::Display for OrchestratorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OrchestratorError::ArtifactMissing(m) | OrchestratorError::Runtime(m) => {
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

/// The backend selected by `WORKSPACE_BACKEND` (default `docker`). Invalid
/// values fail CityHall startup instead of surfacing on first workspace use.
pub fn from_env() -> Result<Arc<dyn Orchestrator>, String> {
    let backend = std::env::var("WORKSPACE_BACKEND").unwrap_or_else(|_| "docker".to_string());
    match backend.as_str() {
        "docker" => Ok(Arc::new(docker::DockerCliOrchestrator::from_env())),
        "kubernetes" => Ok(Arc::new(kubernetes::KubectlOrchestrator::from_env())),
        other => Err(format!(
            "unknown WORKSPACE_BACKEND '{other}' (expected docker or kubernetes)"
        )),
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
