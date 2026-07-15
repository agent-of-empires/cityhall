//! Docker CLI workspace backend.
//!
//! Shells out to the `docker` binary (override with `CONTAINER_CLI`, e.g.
//! `podman`) rather than a docker API crate: the aoe ecosystem already drives
//! containers through the CLI, it needs no extra dependencies, and only
//! structured output (`--format '{{json .}}'`) is parsed. This backend assumes
//! CityHall runs natively on the docker host and reaches workspaces through
//! loopback-published ephemeral ports; other topologies are separate backends.

use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use tokio::process::Command;

use super::{Orchestrator, OrchestratorError, WorkspaceSpec, WorkspaceStatus};

/// Port aoe serves on inside the workspace container.
const AOE_PORT: u16 = 8080;
/// Where the aoe app dir lives inside the container (the reference image runs
/// as user `aoe`); the per-user volume is mounted here.
const AOE_DATA_DIR: &str = "/home/aoe/.config/agent-of-empires";
const CLI_TIMEOUT: Duration = Duration::from_secs(60);
/// How long to wait for aoe to accept connections after `docker start`.
const READY_TIMEOUT: Duration = Duration::from_secs(15);

pub fn container_name(user_id: i32) -> String {
    format!("cityhall-workspace-u{user_id}")
}

pub fn volume_name(user_id: i32) -> String {
    format!("cityhall-workspace-u{user_id}-data")
}

pub struct DockerCliOrchestrator {
    cli: String,
}

impl DockerCliOrchestrator {
    pub fn from_env() -> Self {
        DockerCliOrchestrator {
            cli: std::env::var("CONTAINER_CLI").unwrap_or_else(|_| "docker".to_string()),
        }
    }

    /// Run the container CLI, returning stdout on success. `NotFound` is
    /// reported as `Ok(None)` so callers can treat missing objects as state,
    /// not failure.
    async fn run(&self, args: &[&str]) -> Result<Option<String>, OrchestratorError> {
        let mut cmd = Command::new(&self.cli);
        cmd.args(args).kill_on_drop(true);
        let output = tokio::time::timeout(CLI_TIMEOUT, cmd.output())
            .await
            .map_err(|_| {
                OrchestratorError::Runtime(format!("{} {} timed out", self.cli, args.join(" ")))
            })?
            .map_err(|e| OrchestratorError::Runtime(format!("failed to run {}: {e}", self.cli)))?;

        if output.status.success() {
            return Ok(Some(String::from_utf8_lossy(&output.stdout).into_owned()));
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        let lower = stderr.to_lowercase();
        if lower.contains("no such") || lower.contains("not found") {
            return Ok(None);
        }
        Err(OrchestratorError::Runtime(format!(
            "{} {} failed: {}",
            self.cli,
            args.join(" "),
            stderr.trim()
        )))
    }

    async fn inspect(&self, name: &str) -> Result<Option<ContainerState>, OrchestratorError> {
        let out = self
            .run(&["container", "inspect", "--format", "{{json .}}", name])
            .await?;
        match out {
            Some(json) => {
                let raw: InspectOutput = serde_json::from_str(json.trim()).map_err(|e| {
                    OrchestratorError::Runtime(format!("unparseable inspect output: {e}"))
                })?;
                Ok(Some(ContainerState {
                    running: raw.state.running,
                    version_label: raw
                        .config
                        .labels
                        .and_then(|l| l.get("cityhall.workspace.version").cloned()),
                }))
            }
            None => Ok(None),
        }
    }

    /// The loopback address of the container's published aoe port.
    async fn published_addr(&self, name: &str) -> Result<String, OrchestratorError> {
        let port_arg = format!("{AOE_PORT}/tcp");
        let out = self
            .run(&["port", name, &port_arg])
            .await?
            .ok_or_else(|| OrchestratorError::Runtime(format!("container {name} not found")))?;
        parse_published_addr(&out).ok_or_else(|| {
            OrchestratorError::Runtime(format!("no published port for {name}: {out}"))
        })
    }

    async fn image_exists(&self, image: &str) -> Result<bool, OrchestratorError> {
        Ok(self
            .run(&["image", "inspect", "--format", "{{.Id}}", image])
            .await?
            .is_some())
    }

    async fn create_and_start(&self, spec: &WorkspaceSpec) -> Result<(), OrchestratorError> {
        if !self.image_exists(&spec.image).await? {
            return Err(OrchestratorError::ImageMissing(format!(
                "workspace image '{}' not found; build it first (see deploy/aoe-image/) \
                 or fix the image template / version in the workspace settings",
                spec.image
            )));
        }
        let volume = volume_name(spec.user_id);
        self.run(&[
            "volume",
            "create",
            "--label",
            "cityhall.managed=true",
            &volume,
        ])
        .await?;

        let name = container_name(spec.user_id);
        let user_label = format!("cityhall.user_id={}", spec.user_id);
        let version_label = format!("cityhall.workspace.version={}", spec.version);
        let mount = format!("{volume}:{AOE_DATA_DIR}");
        let publish = format!("127.0.0.1:0:{AOE_PORT}");
        let port = AOE_PORT.to_string();
        self.run(&[
            "run",
            "-d",
            "--name",
            &name,
            "--label",
            "cityhall.managed=true",
            "--label",
            &user_label,
            "--label",
            &version_label,
            "-v",
            &mount,
            "-p",
            &publish,
            &spec.image,
            "aoe",
            "serve",
            "--host",
            "0.0.0.0",
            "--port",
            &port,
            // CityHall's session gates the proxy; the container port is only
            // published on loopback.
            "--auth",
            "none",
            "--behind-proxy",
        ])
        .await?;
        Ok(())
    }

    /// Wait until the workspace answers HTTP so the first proxied request does
    /// not race aoe's startup.
    async fn wait_ready(&self, addr: &str) -> Result<(), OrchestratorError> {
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
}

#[async_trait]
impl Orchestrator for DockerCliOrchestrator {
    async fn ensure_started(&self, spec: &WorkspaceSpec) -> Result<String, OrchestratorError> {
        let name = container_name(spec.user_id);
        match self.inspect(&name).await? {
            Some(state) if state.version_label.as_deref() != Some(spec.version.as_str()) => {
                // Version drift: recreate the container, keeping the volume.
                tracing::info!(
                    user_id = spec.user_id,
                    from = state.version_label.as_deref().unwrap_or("unknown"),
                    to = %spec.version,
                    "recreating workspace for version change"
                );
                self.run(&["rm", "-f", &name]).await?;
                self.create_and_start(spec).await?;
            }
            Some(state) if state.running => {}
            Some(_) => {
                self.run(&["start", &name]).await?;
            }
            None => {
                self.create_and_start(spec).await?;
            }
        }
        let addr = self.published_addr(&name).await?;
        self.wait_ready(&addr).await?;
        Ok(addr)
    }

    async fn stop(&self, user_id: i32) -> Result<(), OrchestratorError> {
        self.run(&["stop", "-t", "10", &container_name(user_id)])
            .await?;
        Ok(())
    }

    async fn destroy(&self, user_id: i32) -> Result<(), OrchestratorError> {
        self.run(&["rm", "-f", &container_name(user_id)]).await?;
        self.run(&["volume", "rm", &volume_name(user_id)]).await?;
        Ok(())
    }

    async fn status(&self, user_id: i32) -> Result<WorkspaceStatus, OrchestratorError> {
        let name = container_name(user_id);
        match self.inspect(&name).await? {
            None => Ok(WorkspaceStatus::NotCreated),
            Some(state) if state.running => Ok(WorkspaceStatus::Running {
                addr: self.published_addr(&name).await?,
            }),
            Some(_) => Ok(WorkspaceStatus::Stopped),
        }
    }
}

struct ContainerState {
    running: bool,
    version_label: Option<String>,
}

#[derive(Deserialize)]
struct InspectOutput {
    #[serde(rename = "State")]
    state: InspectState,
    #[serde(rename = "Config")]
    config: InspectConfig,
}

#[derive(Deserialize)]
struct InspectState {
    #[serde(rename = "Running")]
    running: bool,
}

#[derive(Deserialize)]
struct InspectConfig {
    #[serde(rename = "Labels")]
    labels: Option<std::collections::HashMap<String, String>>,
}

/// Whether an HTTP server answers at `addr`. A bare TCP connect is not
/// enough: docker's userland proxy accepts connections on the published port
/// before the service inside the container listens, so the probe must
/// actually exchange bytes.
async fn http_probe(addr: &str) -> bool {
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

/// Parse `docker port` output (`127.0.0.1:55000`, possibly multiple lines with
/// an IPv6 line like `[::1]:55000`) into a dialable loopback address.
fn parse_published_addr(out: &str) -> Option<String> {
    out.lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('['))
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_deterministic() {
        assert_eq!(container_name(42), "cityhall-workspace-u42");
        assert_eq!(volume_name(42), "cityhall-workspace-u42-data");
    }

    #[test]
    fn parse_port_output() {
        assert_eq!(
            parse_published_addr("127.0.0.1:55000\n").as_deref(),
            Some("127.0.0.1:55000")
        );
        // IPv4 line preferred over the IPv6 one regardless of order.
        assert_eq!(
            parse_published_addr("[::1]:55000\n127.0.0.1:55000\n").as_deref(),
            Some("127.0.0.1:55000")
        );
        assert_eq!(parse_published_addr(""), None);
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
    fn inspect_json_parses() {
        let json = r#"{"State":{"Running":true},"Config":{"Labels":{"cityhall.workspace.version":"v1.0.0"}}}"#;
        let raw: InspectOutput = serde_json::from_str(json).unwrap();
        assert!(raw.state.running);
        assert_eq!(
            raw.config.labels.unwrap().get("cityhall.workspace.version"),
            Some(&"v1.0.0".to_string())
        );
    }
}
