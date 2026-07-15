//! Backend-agnostic orchestration seam for per-user aoe workspaces.
//!
//! A workspace is one long-lived aoe instance (container, process, pod...)
//! per user with a persistent data volume. Backends implement [`Orchestrator`];
//! CityHall stores only intent (pinned version, activity) in the database and
//! treats the runtime as the source of truth for liveness.

pub mod docker;

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
    /// The workspace image is not available; carries operator guidance.
    ImageMissing(String),
    /// Any other backend failure (daemon down, command failed...).
    Runtime(String),
}

impl std::fmt::Display for OrchestratorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OrchestratorError::ImageMissing(m) | OrchestratorError::Runtime(m) => {
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

/// Render an image template by substituting the `{version}` placeholder.
pub fn render_image(template: &str, version: &str) -> String {
    template.replace("{version}", version)
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
}
