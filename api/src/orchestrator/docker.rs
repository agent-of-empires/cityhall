//! Docker CLI workspace backend.
//!
//! Shells out to the `docker` binary (override with `CONTAINER_CLI`, e.g.
//! `podman`) rather than a docker API crate: the aoe ecosystem already drives
//! containers through the CLI, it needs no extra dependencies, and only
//! structured output (`--format '{{json .}}'`) is parsed.
//!
//! Two addressing modes:
//! - Published (default): CityHall runs natively on the docker host and
//!   reaches workspaces through loopback-published ephemeral ports.
//! - Shared network (`WORKSPACE_DOCKER_NETWORK`): CityHall itself runs in a
//!   container on the named docker network (socket mounted); workspaces join
//!   that network with no published ports and are dialed by container DNS.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use tokio::process::Command;

use super::{
    wait_ready, Begin, Orchestrator, OrchestratorError, ProvisioningRegistry, WorkspaceSpec,
    WorkspaceStatus,
};

/// Port aoe serves on inside the workspace container.
const AOE_PORT: u16 = 8080;
/// Where the aoe app dir lives inside the container (the reference image runs
/// as user `aoe`); the per-user volume is mounted here.
const AOE_DATA_DIR: &str = "/home/aoe/.config/agent-of-empires";
const CLI_TIMEOUT: Duration = Duration::from_secs(60);
/// Image pulls and first builds legitimately take minutes.
const PROVISION_TIMEOUT: Duration = Duration::from_secs(600);

/// The reference workspace image build, embedded so a running CityHall can
/// build missing images without a repo checkout. It needs no build context
/// (it downloads the release tarball itself), so it is piped to
/// `docker build -`.
const AOE_IMAGE_DOCKERFILE: &str = include_str!("../../../deploy/aoe-image/Dockerfile");

pub fn container_name(user_id: i32) -> String {
    format!("cityhall-workspace-u{user_id}")
}

pub fn volume_name(user_id: i32) -> String {
    format!("cityhall-workspace-u{user_id}-data")
}

pub struct DockerCliOrchestrator {
    cli: String,
    /// Shared-network addressing mode: workspaces join this docker network
    /// (no published ports) and are dialed by container DNS name.
    network: Option<String>,
    provisioning: Arc<ProvisioningRegistry>,
}

impl DockerCliOrchestrator {
    pub fn from_env(provisioning: Arc<ProvisioningRegistry>) -> Self {
        DockerCliOrchestrator {
            cli: std::env::var("CONTAINER_CLI").unwrap_or_else(|_| "docker".to_string()),
            network: std::env::var("WORKSPACE_DOCKER_NETWORK")
                .ok()
                .filter(|n| !n.trim().is_empty()),
            provisioning,
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
                let labels = raw.config.labels.unwrap_or_default();
                Ok(Some(ContainerState {
                    running: raw.state.running,
                    version_label: labels.get("cityhall.workspace.version").cloned(),
                    network_label: labels.get("cityhall.workspace.network").cloned(),
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
            return Err(self.provision_image(spec));
        }
        if let Some(net) = &self.network {
            // Surface a clear error now instead of `docker run`'s "not found"
            // (which run() would misread as a missing container).
            if self
                .run(&["network", "inspect", "--format", "{{.Id}}", net])
                .await?
                .is_none()
            {
                return Err(OrchestratorError::Runtime(format!(
                    "docker network '{net}' (WORKSPACE_DOCKER_NETWORK) does not exist"
                )));
            }
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

        let args = run_args(spec, self.network.as_deref());
        self.run(&args.iter().map(String::as_str).collect::<Vec<_>>())
            .await?;
        Ok(())
    }

    /// The address the proxy dials: container DNS on the shared network, or
    /// the loopback published port.
    async fn addr(&self, name: &str) -> Result<String, OrchestratorError> {
        match &self.network {
            Some(_) => Ok(format!("{name}:{AOE_PORT}")),
            None => self.published_addr(name).await,
        }
    }

    /// Kick off (or report) background provisioning of a missing image:
    /// `docker pull`, then a local build from the embedded reference
    /// Dockerfile. Detached from the request so a closed browser tab cannot
    /// kill a multi-minute build; callers get a retry-shortly error.
    fn provision_image(&self, spec: &WorkspaceSpec) -> OrchestratorError {
        let image = spec.image.clone();
        let message = format!("pulling image {image}");
        match self.provisioning.begin(&image, &message) {
            Begin::AlreadyRunning(msg) => OrchestratorError::Provisioning(msg),
            Begin::RecentlyFailed(msg) => OrchestratorError::ArtifactMissing(msg),
            Begin::Started => {
                let cli = self.cli.clone();
                let registry = self.provisioning.clone();
                let version = spec.version.clone();
                tokio::spawn(provision_image_job(cli, image, version, registry));
                OrchestratorError::Provisioning(message)
            }
        }
    }
}

async fn provision_image_job(
    cli: String,
    image: String,
    version: String,
    registry: Arc<ProvisioningRegistry>,
) {
    let pull_err = match provision_run(&cli, &["pull", &image], None, &image).await {
        Ok(()) => {
            tracing::info!(%image, "pulled workspace image");
            registry.succeed(&image);
            return;
        }
        Err(e) => e,
    };
    registry.progress(
        &image,
        &format!("building image {image} (a first build takes a few minutes)"),
    );
    let build_arg = format!("AOE_VERSION={version}");
    match provision_run(
        &cli,
        &["build", "--build-arg", &build_arg, "-t", &image, "-"],
        Some(AOE_IMAGE_DOCKERFILE),
        &image,
    )
    .await
    {
        Ok(()) => {
            tracing::info!(%image, "built workspace image from the reference Dockerfile");
            registry.succeed(&image);
        }
        Err(build_err) => {
            tracing::warn!(%image, "workspace image provisioning failed");
            registry.fail(
                &image,
                format!("provisioning image {image} failed; pull: {pull_err}; build: {build_err}"),
            );
        }
    }
}

/// Run a slow provisioning command with its output streamed to a log file
/// (build logs can be megabytes); failures return the log tail.
async fn provision_run(
    cli: &str,
    args: &[&str],
    stdin: Option<&str>,
    log_name: &str,
) -> Result<(), String> {
    let sanitized: String = log_name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let log_path = std::env::temp_dir().join(format!("cityhall-provision-{sanitized}.log"));
    let log = std::fs::File::create(&log_path).map_err(|e| format!("cannot open log: {e}"))?;
    let log_err = log
        .try_clone()
        .map_err(|e| format!("cannot open log: {e}"))?;

    let mut cmd = Command::new(cli);
    cmd.args(args)
        // kill_on_drop reaps the CLI when the timeout fires; the docker
        // daemon may still finish server-side, which the next existence
        // check picks up.
        .kill_on_drop(true)
        .stdin(if stdin.is_some() {
            std::process::Stdio::piped()
        } else {
            std::process::Stdio::null()
        })
        .stdout(std::process::Stdio::from(log))
        .stderr(std::process::Stdio::from(log_err));

    let run = async {
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("failed to run {cli}: {e}"))?;
        if let Some(payload) = stdin {
            use tokio::io::AsyncWriteExt;
            let mut pipe = child.stdin.take().expect("stdin piped");
            pipe.write_all(payload.as_bytes())
                .await
                .map_err(|e| format!("failed to feed {cli} stdin: {e}"))?;
            drop(pipe);
        }
        child.wait().await.map_err(|e| format!("{cli} failed: {e}"))
    };
    let status = tokio::time::timeout(PROVISION_TIMEOUT, run)
        .await
        .map_err(|_| format!("{cli} {} timed out after {PROVISION_TIMEOUT:?}", args[0]))??;

    if status.success() {
        Ok(())
    } else {
        Err(log_tail(&log_path))
    }
}

/// The last ~500 bytes of a provisioning log, for error messages.
fn log_tail(path: &std::path::Path) -> String {
    match std::fs::read_to_string(path) {
        Ok(s) => {
            let tail: String = s
                .chars()
                .rev()
                .take(500)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            tail.trim().to_string()
        }
        Err(_) => "command failed (no log available)".to_string(),
    }
}

/// The full `docker run` invocation for a workspace container.
fn run_args(spec: &WorkspaceSpec, network: Option<&str>) -> Vec<String> {
    let name = container_name(spec.user_id);
    let volume = volume_name(spec.user_id);
    let mut args: Vec<String> = vec![
        "run".into(),
        "-d".into(),
        "--name".into(),
        name,
        "--label".into(),
        "cityhall.managed=true".into(),
        "--label".into(),
        format!("cityhall.user_id={}", spec.user_id),
        "--label".into(),
        format!("cityhall.workspace.version={}", spec.version),
        "-v".into(),
        format!("{volume}:{AOE_DATA_DIR}"),
    ];
    match network {
        Some(net) => {
            // Reachable by container DNS from inside the network only; the
            // addressing mode is recorded so flipping it recreates the
            // container.
            args.push("--label".into());
            args.push(format!("cityhall.workspace.network={net}"));
            args.push("--network".into());
            args.push(net.into());
        }
        None => {
            args.push("-p".into());
            args.push(format!("127.0.0.1:0:{AOE_PORT}"));
        }
    }
    args.extend(
        [
            &spec.image,
            "aoe",
            "serve",
            "--host",
            "0.0.0.0",
            "--port",
            &AOE_PORT.to_string(),
            // CityHall's session gates the proxy; the container port is never
            // reachable from outside (loopback publish or internal network).
            "--auth",
            "none",
            "--behind-proxy",
        ]
        .iter()
        .map(|s| s.to_string()),
    );
    args
}

#[async_trait]
impl Orchestrator for DockerCliOrchestrator {
    async fn ensure_started(&self, spec: &WorkspaceSpec) -> Result<String, OrchestratorError> {
        let name = container_name(spec.user_id);
        match self.inspect(&name).await? {
            Some(state)
                if state.version_label.as_deref() != Some(spec.version.as_str())
                    || state.network_label != self.network =>
            {
                // Version or addressing drift: recreate the container,
                // keeping the volume.
                tracing::info!(
                    user_id = spec.user_id,
                    from = state.version_label.as_deref().unwrap_or("unknown"),
                    to = %spec.version,
                    "recreating workspace for version or addressing change"
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
        let addr = self.addr(&name).await?;
        wait_ready(&addr).await?;
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
                addr: self.addr(&name).await?,
            }),
            Some(_) => Ok(WorkspaceStatus::Stopped),
        }
    }
}

struct ContainerState {
    running: bool,
    version_label: Option<String>,
    network_label: Option<String>,
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

    fn spec() -> WorkspaceSpec {
        WorkspaceSpec {
            user_id: 42,
            image: "cityhall/aoe:v1.0.0".to_string(),
            version: "v1.0.0".to_string(),
        }
    }

    #[test]
    fn published_mode_publishes_loopback_and_no_network() {
        let args = run_args(&spec(), None);
        let publish_at = args.iter().position(|a| a == "-p").unwrap();
        assert_eq!(args[publish_at + 1], "127.0.0.1:0:8080");
        assert!(!args.iter().any(|a| a == "--network"));
        assert!(!args
            .iter()
            .any(|a| a.starts_with("cityhall.workspace.network=")));
    }

    #[test]
    fn network_mode_joins_network_and_publishes_nothing() {
        let args = run_args(&spec(), Some("cityhall-workspaces"));
        let net_at = args.iter().position(|a| a == "--network").unwrap();
        assert_eq!(args[net_at + 1], "cityhall-workspaces");
        assert!(!args.iter().any(|a| a == "-p"));
        // The mode is recorded so flipping WORKSPACE_DOCKER_NETWORK recreates.
        assert!(args
            .iter()
            .any(|a| a == "cityhall.workspace.network=cityhall-workspaces"));
    }
}
