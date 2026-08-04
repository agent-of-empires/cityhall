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

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use tokio::process::Command;

use super::{
    git_version_sha, proxy_allowed_host, proxy_allowed_origin, wait_ready, Begin, Orchestrator,
    OrchestratorError, ProvisioningRegistry, WorkspaceSpec, WorkspaceStatus,
};

/// Port aoe serves on inside the workspace container.
const AOE_PORT: u16 = 8080;
/// Where the aoe app dir lives inside the container (the reference image runs
/// as user `aoe`); the per-user volume is mounted here.
const AOE_DATA_DIR: &str = "/home/aoe/.config/agent-of-empires";
const CLI_TIMEOUT: Duration = Duration::from_secs(60);
/// Image pulls and first builds legitimately take minutes.
const PROVISION_TIMEOUT: Duration = Duration::from_secs(600);
/// A source build compiles aoe's several hundred crates and its frontend, which
/// does not fit in the budget a download gets.
const SOURCE_BUILD_TIMEOUT: Duration = Duration::from_secs(3600);

/// The reference workspace image build, embedded so a running CityHall can
/// build missing images without a repo checkout. It needs no build context
/// (it fetches aoe itself, as a release tarball or a git clone), so it is
/// piped to `docker build -`.
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
                    env_label: labels.get("cityhall.workspace.env").cloned(),
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

        // The guard's Drop removes the file on every exit path below,
        // including an early `?` return from a failed or timed-out
        // `docker run`; the values must not linger on disk after the CLI
        // returns.
        let contents = env_file_contents(spec);
        let env_file = if contents.is_empty() {
            None
        } else {
            Some(write_env_file(spec.user_id, &contents)?)
        };
        let args = run_args(
            spec,
            self.network.as_deref(),
            env_file.as_ref().map(EnvFileGuard::path),
        );
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

/// Build arguments, timeout, and progress message for building `image`: a
/// release version downloads a tarball, a `git-<sha>` version compiles that
/// commit. Extracted as a pure function so the source-build path is testable
/// without a docker daemon.
fn build_plan(image: &str, version: &str) -> (Vec<String>, Duration, String) {
    match git_version_sha(version) {
        Some(sha) => (
            vec![
                "--build-arg".to_string(),
                "AOE_SOURCE=git".to_string(),
                "--build-arg".to_string(),
                format!("AOE_GIT_SHA={sha}"),
            ],
            SOURCE_BUILD_TIMEOUT,
            format!("building image {image} from aoe source (compiling aoe takes many minutes)"),
        ),
        None => (
            vec!["--build-arg".to_string(), format!("AOE_VERSION={version}")],
            PROVISION_TIMEOUT,
            format!("building image {image} (a first build takes a few minutes)"),
        ),
    }
}

async fn provision_image_job(
    cli: String,
    image: String,
    version: String,
    registry: Arc<ProvisioningRegistry>,
) {
    // A source build's image is published nowhere, so pulling it can only fail
    // and add that failure to the message a real build error would carry.
    let git_sha = git_version_sha(&version).map(str::to_string);
    let pull_err = match git_sha {
        Some(_) => None,
        None => {
            match provision_run(&cli, &["pull", &image], None, &image, PROVISION_TIMEOUT).await {
                Ok(()) => {
                    tracing::info!(%image, "pulled workspace image");
                    registry.succeed(&image);
                    return;
                }
                Err(e) => Some(e),
            }
        }
    };

    let (build_args, timeout, message) = build_plan(&image, &version);
    registry.progress(&image, &message);

    let mut args: Vec<&str> = vec!["build"];
    args.extend(build_args.iter().map(String::as_str));
    args.extend(["-t", image.as_str(), "-"]);
    match provision_run(&cli, &args, Some(AOE_IMAGE_DOCKERFILE), &image, timeout).await {
        Ok(()) => {
            tracing::info!(%image, "built workspace image from the reference Dockerfile");
            registry.succeed(&image);
        }
        Err(build_err) => {
            tracing::warn!(%image, "workspace image provisioning failed");
            let detail = match &pull_err {
                Some(pull) => format!("pull: {pull}; build: {build_err}"),
                None => format!("build: {build_err}"),
            };
            registry.fail(
                &image,
                format!("provisioning image {image} failed; {detail}"),
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
    timeout: Duration,
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
        // The reference image needs BuildKit: it writes its entrypoint with a
        // COPY heredoc and selects a build stage by argument, and the classic
        // builder supports neither, silently building every stage instead.
        // Asking for it explicitly turns a missing buildx plugin into a clear
        // error here rather than a confusing failure inside the wrong stage.
        .env("DOCKER_BUILDKIT", "1")
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
    let status = tokio::time::timeout(timeout, run)
        .await
        .map_err(|_| format!("{cli} {} timed out after {timeout:?}", args[0]))??;

    if status.success() {
        Ok(())
    } else {
        Err(log_tail(&log_path))
    }
}

/// The tail of a provisioning log, for error messages. Whole lines only: a
/// byte-counted tail cuts mid-word, and the result reads as corruption rather
/// than as the end of a build log.
fn log_tail(path: &std::path::Path) -> String {
    const MAX_LINES: usize = 12;
    const MAX_BYTES: usize = 2000;
    match std::fs::read_to_string(path) {
        Ok(s) => {
            let mut tail = String::new();
            for line in s.lines().rev().take(MAX_LINES) {
                if tail.len() + line.len() > MAX_BYTES {
                    break;
                }
                tail.insert_str(0, line);
                tail.insert(0, '\n');
            }
            let tail = tail.trim();
            if tail.is_empty() {
                "command failed (no output)".to_string()
            } else {
                tail.to_string()
            }
        }
        Err(_) => "command failed (no log available)".to_string(),
    }
}

/// `KEY=value` lines for everything the workspace's environment needs: the
/// bundle location and token (when configured), then every agent credential.
/// One line per pair, in that order; `agent_credentials::validate_value`
/// guarantees a value can contain no NUL, newline, or carriage return before
/// it ever reaches here, which this line-oriented format depends on.
fn env_file_contents(spec: &WorkspaceSpec) -> String {
    let mut out = String::new();
    if let Some(bundle) = &spec.bundle {
        out.push_str(&format!("AOE_CITYHALL_BUNDLE_URL={}\n", bundle.url));
        out.push_str(&format!("AOE_CITYHALL_BUNDLE_TOKEN={}\n", bundle.token));
    }
    for (name, value) in &spec.agent_env.pairs {
        out.push_str(&format!("{name}={}\n", value.expose()));
    }
    out
}

/// A workspace's `--env-file` on disk, deleted as soon as it goes out of
/// scope. `docker run` only needs the file to exist for the moment it reads
/// it client-side; an RAII guard rather than an explicit remove-on-every-path
/// means an early `?` return from a failed or timed-out `docker run` still
/// cleans it up.
struct EnvFileGuard(PathBuf);

impl EnvFileGuard {
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for EnvFileGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Write `contents` to a fresh, mode-0600 file in the process temp dir (never
/// a directory mounted into a container), named so a concurrent create for
/// any user cannot collide with it. The permission is set at creation time
/// (`mode` on `OpenOptions`) rather than after writing, so there is no window
/// where the file is world-readable.
fn write_env_file(user_id: i32, contents: &str) -> Result<EnvFileGuard, OrchestratorError> {
    use std::io::Write;

    let mut suffix = [0u8; 8];
    getrandom::fill(&mut suffix)
        .map_err(|_| OrchestratorError::Runtime("secure RNG failure".to_string()))?;
    let suffix = u64::from_le_bytes(suffix);
    let path = std::env::temp_dir().join(format!(
        "cityhall-workspace-env-u{user_id}-{suffix:016x}.env"
    ));

    let open_result = {
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)
        }
        #[cfg(not(unix))]
        {
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
        }
    };
    let mut file = open_result
        .map_err(|e| OrchestratorError::Runtime(format!("failed to create env file: {e}")))?;
    // Wrapped in the guard immediately: a write failure below must still
    // delete the file rather than leave credentials sitting on disk.
    let guard = EnvFileGuard(path);
    file.write_all(contents.as_bytes())
        .map_err(|e| OrchestratorError::Runtime(format!("failed to write env file: {e}")))?;
    Ok(guard)
}

/// The full `docker run` invocation for a workspace container.
fn run_args(spec: &WorkspaceSpec, network: Option<&str>, env_file: Option<&Path>) -> Vec<String> {
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
        // Empty (never absent) when the user has no stored credentials, so an
        // unconfigured user's fingerprint and a pre-feature container's
        // absent label compare equal and neither is treated as drift.
        "--label".into(),
        format!("cityhall.workspace.env={}", spec.agent_env.fingerprint),
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
    // The bundle token and every agent credential travel through
    // `--env-file` rather than `-e`. `--env-file` is read client-side, so the
    // values still reach the container config and `docker inspect` still
    // shows them; what it buys is keeping them out of argv, out of the host
    // process list, and out of the `args.join(" ")` error strings in `run`.
    if let Some(path) = env_file {
        args.push("--env-file".into());
        args.push(path.display().to_string());
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
            // The proxy forwards the public Host and Origin, both of which
            // aoe's DNS-rebinding gate requires on the allowlist.
            "--allowed-host",
            &proxy_allowed_host(),
            "--allowed-origin",
            &proxy_allowed_origin(),
            // Locked-down end-user client: composer + structured view only.
            "--cityhall",
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
            Some(state) if needs_recreate(&state, spec, self.network.as_deref()) => {
                // Version, addressing, or credential drift: recreate the
                // container, keeping the volume. `docker start` reuses the
                // stored container config, so it cannot pick up a new
                // environment; recreation is the only way a credential
                // change ever reaches the workspace process.
                tracing::info!(
                    user_id = spec.user_id,
                    from = state.version_label.as_deref().unwrap_or("unknown"),
                    to = %spec.version,
                    "recreating workspace for version, addressing, or credential change"
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
    env_label: Option<String>,
}

/// Whether a running container must be recreated rather than reused or
/// resumed: the pinned version changed, the addressing mode changed, or the
/// injected credential set changed. Extracted as a pure function so the
/// credential-drift condition (the part this feature adds) is testable
/// without a docker daemon.
///
/// The env label is absent on any container created before this feature.
/// Treating that absence as the empty string means a user with no stored
/// credentials (whose fingerprint is also empty) is never recreated on
/// upgrade just because the label didn't exist yet.
fn needs_recreate(state: &ContainerState, spec: &WorkspaceSpec, network: Option<&str>) -> bool {
    state.version_label.as_deref() != Some(spec.version.as_str())
        || state.network_label.as_deref() != network
        || state.env_label.as_deref().unwrap_or("") != spec.agent_env.fingerprint
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
            bundle: None,
            agent_env: crate::agent_credentials::AgentEnv::default(),
        }
    }

    fn spec_with_bundle() -> WorkspaceSpec {
        WorkspaceSpec {
            bundle: Some(super::super::BundleAccess {
                url: "http://cityhall:3000/api/workspace-bundle".to_string(),
                token: "tok".to_string(),
            }),
            ..spec()
        }
    }

    /// A distinctive credential value, checked for absence in argv and error
    /// strings elsewhere: any test that finds this substring outside the
    /// env file content has found a leak.
    const SECRET_VALUE: &str = "sk-super-secret-value";

    fn spec_with_agent_env() -> WorkspaceSpec {
        WorkspaceSpec {
            agent_env: crate::agent_credentials::AgentEnv {
                pairs: vec![(
                    "ANTHROPIC_API_KEY".to_string(),
                    crate::crypto::Secret::new(SECRET_VALUE.to_string()),
                )],
                fingerprint: "abc123fingerprint".to_string(),
            },
            ..spec()
        }
    }

    fn spec_with_bundle_and_agent_env() -> WorkspaceSpec {
        WorkspaceSpec {
            agent_env: spec_with_agent_env().agent_env,
            ..spec_with_bundle()
        }
    }

    #[test]
    fn env_file_contents_lists_bundle_and_agent_pairs_one_per_line() {
        let contents = env_file_contents(&spec_with_bundle_and_agent_env());
        let lines: Vec<&str> = contents.lines().collect();
        assert_eq!(
            lines,
            vec![
                "AOE_CITYHALL_BUNDLE_URL=http://cityhall:3000/api/workspace-bundle",
                "AOE_CITYHALL_BUNDLE_TOKEN=tok",
                &format!("ANTHROPIC_API_KEY={SECRET_VALUE}"),
            ]
        );
    }

    #[test]
    fn env_file_contents_is_empty_with_nothing_to_inject() {
        assert_eq!(env_file_contents(&spec()), "");
    }

    /// Neither the bundle token nor an agent credential may ever reach argv:
    /// they travel through `--env-file` instead of `-e`, because `run()`
    /// joins argv into its error strings on a failed or timed-out command.
    #[test]
    fn run_args_never_puts_credentials_in_argv() {
        let args = run_args(&spec_with_bundle_and_agent_env(), None, None);
        assert!(!args.iter().any(|a| a == "-e"), "{args:?}");
        assert!(
            !args.iter().any(|a| a.contains(SECRET_VALUE)),
            "credential value leaked into argv: {args:?}"
        );
        assert!(
            !args.iter().any(|a| a.contains("tok")),
            "bundle token leaked into argv: {args:?}"
        );
    }

    /// The flag has to precede the image argument, or docker treats it as an
    /// argument to aoe instead of as container configuration. This replaces the
    /// same guard the old `-e` pairs had.
    #[test]
    fn run_args_passes_the_given_env_file_path_before_the_image() {
        let path = std::path::Path::new("/tmp/cityhall-workspace-env-test.env");
        let args = run_args(&spec_with_bundle(), None, Some(path));
        let at = args.iter().position(|a| a == "--env-file").unwrap();
        assert_eq!(args[at + 1], path.display().to_string());
        let image_at = args
            .iter()
            .position(|a| a == "cityhall/aoe:v1.0.0")
            .unwrap();
        assert!(at < image_at, "--env-file must precede the image: {args:?}");
    }

    /// Nothing to inject means no file was created, so there is nothing to
    /// point `--env-file` at.
    #[test]
    fn no_bundle_or_credentials_means_no_env_file_flag() {
        let args = run_args(&spec(), None, None);
        assert!(!args.iter().any(|a| a == "--env-file"), "{args:?}");
        assert!(!args.iter().any(|a| a == "-e"), "{args:?}");
    }

    #[test]
    fn published_mode_publishes_loopback_and_no_network() {
        let args = run_args(&spec(), None, None);
        let publish_at = args.iter().position(|a| a == "-p").unwrap();
        assert_eq!(args[publish_at + 1], "127.0.0.1:0:8080");
        assert!(!args.iter().any(|a| a == "--network"));
        assert!(!args
            .iter()
            .any(|a| a.starts_with("cityhall.workspace.network=")));
    }

    #[test]
    fn workspaces_run_in_cityhall_client_mode() {
        let args = run_args(&spec(), None, None);
        // Locked-down end-user client, never the full aoe dashboard.
        assert!(args.iter().any(|a| a == "--cityhall"));
        // --behind-proxy without this makes `aoe serve` refuse to start.
        assert!(args.iter().any(|a| a == "--allowed-host"));
    }

    #[test]
    fn network_mode_joins_network_and_publishes_nothing() {
        let args = run_args(&spec(), Some("cityhall-workspaces"), None);
        let net_at = args.iter().position(|a| a == "--network").unwrap();
        assert_eq!(args[net_at + 1], "cityhall-workspaces");
        assert!(!args.iter().any(|a| a == "-p"));
        // The mode is recorded so flipping WORKSPACE_DOCKER_NETWORK recreates.
        assert!(args
            .iter()
            .any(|a| a == "cityhall.workspace.network=cityhall-workspaces"));
    }

    fn container_state(
        version: &str,
        network: Option<&str>,
        env_label: Option<&str>,
    ) -> ContainerState {
        ContainerState {
            running: true,
            version_label: Some(version.to_string()),
            network_label: network.map(str::to_string),
            env_label: env_label.map(str::to_string),
        }
    }

    /// A container from before this feature has no env label at all. A user
    /// with no stored credentials has an empty fingerprint. Those two must
    /// compare equal, or every pre-feature container would be recreated once
    /// on upgrade.
    #[test]
    fn absent_env_label_and_empty_fingerprint_do_not_trigger_recreate() {
        let spec = spec();
        assert_eq!(spec.agent_env.fingerprint, "");
        let state = container_state("v1.0.0", None, None);
        assert!(!needs_recreate(&state, &spec, None));
    }

    #[test]
    fn a_differing_env_label_triggers_recreate() {
        let spec = spec_with_agent_env();
        let stale = container_state("v1.0.0", None, Some("some-other-fingerprint"));
        assert!(needs_recreate(&stale, &spec, None));

        let current = container_state("v1.0.0", None, Some(&spec.agent_env.fingerprint));
        assert!(!needs_recreate(&current, &spec, None));
    }

    #[test]
    fn version_and_network_drift_still_trigger_recreate() {
        let spec = spec();
        let wrong_version = container_state("v0.9.0", None, None);
        assert!(needs_recreate(&wrong_version, &spec, None));

        let wrong_network = container_state("v1.0.0", Some("net-a"), None);
        assert!(needs_recreate(&wrong_network, &spec, Some("net-b")));
    }

    #[test]
    fn a_log_tail_keeps_whole_lines() {
        let dir = std::env::temp_dir().join(format!("cityhall-tail-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("log");

        // Long enough that a byte-counted tail would land mid-line, which is
        // what made a real failure read as "SION build arg is required".
        let body: String = (0..80)
            .map(|i| format!("step {i}: AOE_VERSION build arg is required, and then some\n"))
            .collect();
        std::fs::write(&path, &body).unwrap();
        let tail = log_tail(&path);
        assert!(tail.lines().count() <= 12);
        assert!(tail.starts_with("step "), "cut mid-line: {tail}");
        assert!(tail.ends_with("some"));

        std::fs::write(&path, "").unwrap();
        assert_eq!(log_tail(&path), "command failed (no output)");
        assert_eq!(
            log_tail(&dir.join("absent")),
            "command failed (no log available)"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_release_version_builds_by_downloading_its_tarball() {
        let (args, timeout, _) = build_plan("cityhall/aoe:v1.0.0", "v1.0.0");
        assert_eq!(args, ["--build-arg", "AOE_VERSION=v1.0.0"]);
        assert_eq!(timeout, PROVISION_TIMEOUT);
    }

    #[test]
    fn a_source_version_builds_the_commit_it_names() {
        let (args, timeout, message) = build_plan("cityhall/aoe:git-c0ffee", "git-c0ffee");
        assert_eq!(
            args,
            [
                "--build-arg",
                "AOE_SOURCE=git",
                "--build-arg",
                "AOE_GIT_SHA=c0ffee",
            ]
        );
        // The version itself is never passed as AOE_VERSION: the Dockerfile
        // would try to download a release tarball named after a commit.
        assert!(!args.iter().any(|a| a.starts_with("AOE_VERSION=")));
        // Compiling aoe does not fit the budget a download gets.
        assert!(timeout > PROVISION_TIMEOUT);
        assert!(message.contains("source"));
    }
}
