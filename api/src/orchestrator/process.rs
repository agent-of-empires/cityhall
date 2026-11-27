//! Bare-process workspace backend for VPS hosts without docker.
//!
//! Spawns one detached `aoe serve` per user, with an isolated per-user HOME
//! directory as the volume equivalent. Versions are per-version binaries the
//! operator installs under `<root>/versions/<version>/aoe` (from the aoe
//! release tarball). Runtime state (`state.json` with pid, port, version)
//! lives next to each user's HOME so workspaces survive CityHall restarts;
//! the processes themselves are detached into their own session and keep
//! running when CityHall stops.
//!
//! Isolation note: every workspace runs as the CityHall OS user. This backend
//! provides persistence isolation between users, not security isolation; use
//! the docker or kubernetes backend where that matters.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::{
    binary_key, daemon_token, http_probe, plaintext_flag, proxy_allowed_host, proxy_allowed_origin,
    wait_ready, Begin, Orchestrator, OrchestratorError, ProvisioningRegistry, TelemetryPolicy,
    WorkspaceMode, WorkspaceSpec, WorkspaceStatus,
};

/// Grace period between SIGTERM and SIGKILL on stop.
const STOP_TIMEOUT: Duration = Duration::from_secs(10);
/// Bind races on the pre-allocated port are rare; one respawn absorbs them.
const START_ATTEMPTS: u32 = 2;
/// Release tarball downloads legitimately take minutes on slow links.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(600);
/// Sanity cap on the release tarball size.
const MAX_DOWNLOAD_BYTES: u64 = 1024 * 1024 * 1024;
/// Budget for the `aoe telemetry enable` a force-on policy runs before the
/// server starts. It only writes two small files, so anything near this means
/// it is blocked rather than slow.
const FORCE_ON_TIMEOUT: Duration = Duration::from_secs(30);

pub struct ProcessOrchestrator {
    root: PathBuf,
    provisioning: Arc<ProvisioningRegistry>,
}

/// Runtime state persisted across CityHall restarts.
#[derive(Serialize, Deserialize)]
struct RunState {
    pid: i32,
    port: u16,
    version: String,
    /// The agent-credential fingerprint this process was spawned with, so a
    /// credential change is detectable as drift the same way a version
    /// change is. `#[serde(default)]` so a state.json written before this
    /// field existed still deserializes, reading back as the empty
    /// fingerprint (which compares equal to an unconfigured user, not as
    /// drift).
    #[serde(default)]
    env_fingerprint: String,
    /// The telemetry policy this process was spawned under, so a policy change
    /// is drift like a version or credential change is. `#[serde(default)]` for
    /// the same reason as the fingerprint above: a state.json written before
    /// the policy existed reads back as `user_choice`, which is what such a
    /// process is in fact running under, so an upgrade alone restarts nothing.
    #[serde(default)]
    telemetry: String,
    /// `composer` or `full`. `#[serde(default)]` reads a state.json written
    /// before modes existed as Composer, which is what that process is, so an
    /// upgrade alone restarts nothing.
    #[serde(default)]
    mode: String,
    /// Fingerprint of the daemon token this process was spawned with. Absent
    /// (empty) in a state.json from the `--auth none` era, which never equals a
    /// real fingerprint, so every such workspace is respawned once on upgrade.
    #[serde(default)]
    daemon_token_fp: String,
}

impl RunState {
    /// Whether the running process was spawned from `spec`. Anything else is
    /// drift and respawns it.
    fn matches(&self, spec: &WorkspaceSpec) -> bool {
        self.version == spec.version
            && self.env_fingerprint == spec.agent_env.fingerprint
            && TelemetryPolicy::from_stored(&self.telemetry) == spec.telemetry
            && self.daemon_token_fp
                == spec
                    .daemon_token
                    .as_ref()
                    .map(|t| daemon_token::fingerprint(t.expose()))
                    .unwrap_or_default()
            // Only called for a live process, which keeps the mode it started
            // with unless this is an explicit restart.
            && (!spec.apply_mode_to_running
                || WorkspaceMode::from_stored(Some(&self.mode)) == spec.mode)
    }
}

impl ProcessOrchestrator {
    pub fn from_env(provisioning: Arc<ProvisioningRegistry>) -> Self {
        ProcessOrchestrator {
            root: std::env::var("WORKSPACE_PROCESS_DIR")
                .unwrap_or_else(|_| "/var/lib/cityhall/workspaces".to_string())
                .into(),
            provisioning,
        }
    }

    fn user_dir(&self, user_id: i32) -> PathBuf {
        self.root.join(format!("u{user_id}"))
    }

    fn state_path(&self, user_id: i32) -> PathBuf {
        self.user_dir(user_id).join("state.json")
    }

    fn binary_path(&self, version: &str) -> PathBuf {
        self.root.join("versions").join(version).join("aoe")
    }

    fn read_state(&self, user_id: i32) -> Option<RunState> {
        let raw = std::fs::read_to_string(self.state_path(user_id)).ok()?;
        match serde_json::from_str(&raw) {
            Ok(state) => Some(state),
            Err(e) => {
                tracing::warn!(user_id, "ignoring unparseable workspace state.json: {e}");
                None
            }
        }
    }

    fn write_state(&self, user_id: i32, state: &RunState) -> Result<(), OrchestratorError> {
        let path = self.state_path(user_id);
        let tmp = path.with_extension("json.tmp");
        let io = |e: std::io::Error| {
            OrchestratorError::Runtime(format!("failed to write workspace state: {e}"))
        };
        std::fs::write(
            &tmp,
            serde_json::to_string(state).expect("state serializes"),
        )
        .map_err(io)?;
        std::fs::rename(&tmp, &path).map_err(io)?;
        Ok(())
    }

    /// Spawn a fresh `aoe serve` for `spec`, returning its address. Kills the
    /// spawn and retries with a new port if it never answers HTTP (e.g. an
    /// unrelated process stole the pre-allocated port).
    async fn spawn(&self, spec: &WorkspaceSpec) -> Result<String, OrchestratorError> {
        let binary = self.binary_path(&spec.version);
        if !binary.is_file() {
            return Err(self.provision_binary(&spec.version, &binary));
        }
        let home = self.user_dir(spec.user_id).join("home");
        std::fs::create_dir_all(&home).map_err(|e| {
            OrchestratorError::Runtime(format!("failed to create workspace home: {e}"))
        })?;
        apply_force_on(spec, &binary, &home, FORCE_ON_TIMEOUT).await?;
        let token = spec
            .daemon_token
            .as_ref()
            .ok_or_else(daemon_token::missing_token)?;

        let mut last_err = None;
        for _ in 0..START_ATTEMPTS {
            let port = free_port()?;
            let addr = format!("127.0.0.1:{port}");
            // Every attempt, so aoe sees a fresh mtime on the token file.
            daemon_token::write_serve_token(&home, token.expose()).map_err(|e| {
                OrchestratorError::Runtime(format!("failed to write workspace serve token: {e}"))
            })?;
            let pid = self.spawn_once(spec, &binary, &home, port)?;
            self.write_state(
                spec.user_id,
                &RunState {
                    pid,
                    port,
                    version: spec.version.clone(),
                    env_fingerprint: spec.agent_env.fingerprint.clone(),
                    telemetry: spec.telemetry.as_str().to_string(),
                    mode: spec.mode.as_str().to_string(),
                    daemon_token_fp: daemon_token::fingerprint(token.expose()),
                },
            )?;
            match wait_ready(&addr).await {
                Ok(()) => return Ok(addr),
                Err(e) => {
                    kill_group(pid, libc::SIGKILL);
                    last_err = Some(e);
                }
            }
        }
        Err(last_err.expect("at least one start attempt"))
    }

    fn spawn_once(
        &self,
        spec: &WorkspaceSpec,
        binary: &Path,
        home: &Path,
        port: u16,
    ) -> Result<i32, OrchestratorError> {
        let log = open_log(&self.user_dir(spec.user_id).join("serve.log")).map_err(|e| {
            OrchestratorError::Runtime(format!("failed to open workspace log: {e}"))
        })?;
        let mut cmd = std::process::Command::new(binary);
        cmd.args(serve_args(spec, port))
            .env("HOME", home)
            .current_dir(home);
        // The telemetry policy's variables. `DO_NOT_TRACK` is removed first,
        // always: unlike a container, this process inherits CityHall's own
        // environment, so a `DO_NOT_TRACK` set for CityHall itself would
        // otherwise force every workspace off whatever the admin chose.
        cmd.env_remove("DO_NOT_TRACK");
        for (name, value) in spec.telemetry.env_pairs() {
            cmd.env(name, value);
        }
        // Where the workspace fetches its config bundle at boot.
        if let Some(bundle) = &spec.bundle {
            cmd.env("AOE_CITYHALL_BUNDLE_URL", &bundle.url)
                .env("AOE_CITYHALL_BUNDLE_TOKEN", &bundle.token);
        }
        // Command::env does not go through argv, so unlike the docker
        // backend this needs no env-file indirection: the value never
        // appears on a command line or in a `ps` listing. It is still
        // readable from /proc/<pid>/environ by anyone who can read that file,
        // an exposure inherent to spawning a plain OS process rather than a
        // container, and one that widens if workspaces ever run under a
        // shared UID instead of one process per user.
        for (name, value) in &spec.agent_env.pairs {
            cmd.env(name, value.expose());
        }
        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::from(log.try_clone().map_err(|e| {
                OrchestratorError::Runtime(format!("failed to clone workspace log: {e}"))
            })?))
            .stderr(std::process::Stdio::from(log));
        // Detach into its own session so the workspace survives CityHall
        // restarts and group signals target only this workspace.
        unsafe {
            use std::os::unix::process::CommandExt;
            cmd.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = cmd
            .spawn()
            .map_err(|e| OrchestratorError::Runtime(format!("failed to spawn aoe: {e}")))?;
        Ok(child.id() as i32)
    }

    /// Kick off (or report) background download of a missing version's aoe
    /// binary from the GitHub release tarball. Detached from the request so
    /// a closed tab cannot abort the download mid-flight.
    fn provision_binary(&self, version: &str, target: &Path) -> OrchestratorError {
        let key = binary_key(version);
        let message = format!("downloading aoe {version}");
        match self.provisioning.begin(&key, &message) {
            Begin::AlreadyRunning(msg) => OrchestratorError::Provisioning(msg),
            Begin::RecentlyFailed(msg) => OrchestratorError::ArtifactMissing(msg),
            Begin::Started => {
                let registry = self.provisioning.clone();
                let version = version.to_string();
                let target = target.to_path_buf();
                tokio::spawn(async move {
                    match download_release_binary(&version, &target).await {
                        Ok(()) => {
                            tracing::info!(%version, "installed aoe release binary");
                            registry.succeed(&binary_key(&version));
                        }
                        Err(e) => {
                            tracing::warn!(%version, "aoe binary provisioning failed: {e}");
                            registry.fail(
                                &binary_key(&version),
                                format!(
                                    "downloading aoe {version} failed: {e}; install the \
                                     release tarball manually at {} (see docs/workspaces.md)",
                                    target.display()
                                ),
                            );
                        }
                    }
                });
                OrchestratorError::Provisioning(message)
            }
        }
    }

    /// SIGTERM the workspace's process group, escalating to SIGKILL after the
    /// grace period.
    async fn terminate(&self, pid: i32) {
        kill_group(pid, libc::SIGTERM);
        let deadline = tokio::time::Instant::now() + STOP_TIMEOUT;
        while alive(pid) {
            if tokio::time::Instant::now() >= deadline {
                kill_group(pid, libc::SIGKILL);
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

/// Open the workspace log for appending, owner-only. aoe prints its token URL to
/// stdout, so the log carries the workspace's bearer. A log created by an older
/// CityHall keeps its old mode unless it is tightened here.
fn open_log(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)?;
    log.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    Ok(log)
}

#[async_trait]
impl Orchestrator for ProcessOrchestrator {
    async fn ensure_started(&self, spec: &WorkspaceSpec) -> Result<String, OrchestratorError> {
        validate_version(&spec.version)?;
        daemon_token::require_token_auth_support(&spec.version)?;
        if spec.daemon_token.is_none() {
            return Err(daemon_token::missing_token());
        }
        if let Some(state) = self.read_state(spec.user_id) {
            if alive(state.pid) {
                let addr = format!("127.0.0.1:{}", state.port);
                if state.matches(spec) && http_probe(&addr).await {
                    return Ok(addr);
                }
                // Version drift, a credential change, a telemetry policy
                // change, a daemon token change, a hung process, or a recycled PID that is not our
                // workspace: clear it and start fresh. A credential or policy
                // change must terminate and respawn because Command::env is
                // only set once, at spawn.
                tracing::info!(
                    user_id = spec.user_id,
                    from = %state.version,
                    to = %spec.version,
                    "restarting workspace process for a version, credential, telemetry policy, or mode change"
                );
                self.terminate(state.pid).await;
            }
        }
        self.spawn(spec).await
    }

    async fn stop(&self, user_id: i32) -> Result<(), OrchestratorError> {
        if let Some(state) = self.read_state(user_id) {
            if alive(state.pid) {
                self.terminate(state.pid).await;
            }
        }
        Ok(())
    }

    async fn destroy(&self, user_id: i32) -> Result<(), OrchestratorError> {
        self.stop(user_id).await?;
        let dir = self.user_dir(user_id);
        if dir.exists() {
            std::fs::remove_dir_all(&dir).map_err(|e| {
                OrchestratorError::Runtime(format!(
                    "failed to remove workspace dir {}: {e}",
                    dir.display()
                ))
            })?;
        }
        Ok(())
    }

    async fn status(&self, user_id: i32) -> Result<WorkspaceStatus, OrchestratorError> {
        match self.read_state(user_id) {
            None if self.user_dir(user_id).exists() => Ok(WorkspaceStatus::Stopped),
            None => Ok(WorkspaceStatus::NotCreated),
            Some(state) if alive(state.pid) => Ok(WorkspaceStatus::Running {
                addr: format!("127.0.0.1:{}", state.port),
            }),
            Some(_) => Ok(WorkspaceStatus::Stopped),
        }
    }
}

/// The release tarball's binary name for this host, e.g. `aoe-linux-amd64`.
fn release_binary_name() -> Result<String, String> {
    let os = match std::env::consts::OS {
        "linux" => "linux",
        "macos" => "darwin",
        other => return Err(format!("no aoe release binaries for OS '{other}'")),
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        other => return Err(format!("no aoe release binaries for arch '{other}'")),
    };
    Ok(format!("aoe-{os}-{arch}"))
}

fn release_tarball_url(version: &str, member: &str) -> String {
    format!(
        "https://github.com/agent-of-empires/agent-of-empires/releases/download/{version}/{member}.tar.gz"
    )
}

/// Download the release tarball for `version`, extract the single expected
/// binary, and install it at `target` atomically. The version is validated
/// by the caller before it reaches a URL or path.
async fn download_release_binary(version: &str, target: &Path) -> Result<(), String> {
    use tokio::io::AsyncWriteExt;

    let member = release_binary_name()?;
    let url = release_tarball_url(version, &member);
    let dir = target.parent().expect("binary path has a parent");
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create versions dir: {e}"))?;

    let client = reqwest::Client::builder()
        .timeout(DOWNLOAD_TIMEOUT)
        .build()
        .map_err(|e| e.to_string())?;
    let mut resp = client
        .get(&url)
        // GitHub's API rejects requests without a User-Agent.
        .header("User-Agent", "cityhall")
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?
        .error_for_status()
        .map_err(|e| format!("release download failed: {e}"))?;
    if resp.content_length().unwrap_or(0) > MAX_DOWNLOAD_BYTES {
        return Err("release tarball is implausibly large".to_string());
    }

    // Stream to disk; tarballs are tens of megabytes.
    let tarball = dir.join(format!("{member}.tar.gz.partial"));
    let mut file = tokio::fs::File::create(&tarball)
        .await
        .map_err(|e| format!("cannot write tarball: {e}"))?;
    let mut written: u64 = 0;
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| format!("download interrupted: {e}"))?
    {
        written += chunk.len() as u64;
        if written > MAX_DOWNLOAD_BYTES {
            return Err("release tarball is implausibly large".to_string());
        }
        file.write_all(&chunk)
            .await
            .map_err(|e| format!("cannot write tarball: {e}"))?;
    }
    file.flush().await.map_err(|e| e.to_string())?;
    drop(file);

    // Extract only the expected member; anything else in the archive is
    // ignored rather than written to disk.
    let out = tokio::process::Command::new("tar")
        .arg("-xzf")
        .arg(&tarball)
        .arg("-C")
        .arg(dir)
        .arg(&member)
        .output()
        .await
        .map_err(|e| format!("failed to run tar: {e}"))?;
    let _ = std::fs::remove_file(&tarball);
    if !out.status.success() {
        return Err(format!(
            "tar extraction failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }

    let extracted = dir.join(&member);
    let meta = std::fs::symlink_metadata(&extracted).map_err(|e| e.to_string())?;
    if !meta.is_file() {
        let _ = std::fs::remove_file(&extracted);
        return Err("release tarball did not contain a regular binary".to_string());
    }
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&extracted, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| e.to_string())?;
    }
    std::fs::rename(&extracted, target).map_err(|e| format!("cannot install binary: {e}"))?;
    Ok(())
}

/// The `aoe serve` arguments for one workspace process.
fn serve_args(spec: &WorkspaceSpec, port: u16) -> Vec<String> {
    let mut args: Vec<String> = [
        "serve",
        "--host",
        "127.0.0.1",
        "--port",
        &port.to_string(),
        // aoe adopts the per-workspace token written to serve.token (see
        // `daemon_token`); the proxy presents it as the bearer.
        "--auth",
        "token",
        "--behind-proxy",
        // The proxy forwards the public Host and Origin, both of which
        // aoe's DNS-rebinding gate requires on the allowlist.
        "--allowed-host",
        &proxy_allowed_host(),
        "--allowed-origin",
        &proxy_allowed_origin(),
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    // Composer mode is the locked-down end-user client (composer + structured
    // view only); Full mode omits the flag and serves aoe's whole dashboard.
    args.extend(spec.mode.cityhall_flag().map(String::from));
    args.extend(plaintext_flag(&spec.version));
    args
}

/// Versions become path components; reject anything that could escape the
/// versions directory.
fn validate_version(version: &str) -> Result<(), OrchestratorError> {
    let ok = !version.is_empty()
        && !version.starts_with('.')
        && version
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '+'));
    if ok {
        Ok(())
    } else {
        Err(OrchestratorError::Runtime(format!(
            "invalid workspace version '{version}'"
        )))
    }
}

/// Assert a force-on telemetry policy against this workspace's data, before its
/// server starts, by running aoe's own `telemetry enable`. That is what both
/// enables telemetry and records the consent as answered, so CityHall never has
/// to touch aoe's config or state files itself.
///
/// No shell wrapper here, unlike the container backends: CityHall spawns both
/// commands, so it can run one, check it, then run the other. A failure fails
/// the start, naming the command, rather than leaving a workspace up under a
/// policy that was not applied.
///
/// Bounded, and run through tokio like every other CLI this crate shells out
/// to. aoe takes a lock on the data dir it is about to write, so a concurrent
/// aoe holding it makes this command wait; unbounded, that would hold a runtime
/// worker and hang the workspace start with no explanation. `kill_on_drop`
/// means the expired child is killed and reaped when the timeout drops it.
async fn apply_force_on(
    spec: &WorkspaceSpec,
    binary: &Path,
    home: &Path,
    timeout: Duration,
) -> Result<(), OrchestratorError> {
    if spec.telemetry != TelemetryPolicy::ForceOn {
        return Ok(());
    }
    let mut cmd = tokio::process::Command::new(binary);
    cmd.args(["telemetry", "enable"])
        .env("HOME", home)
        // Removed for the same reason as in the spawn: an inherited
        // `DO_NOT_TRACK` would make aoe report the opt-in as suppressed.
        .env_remove("DO_NOT_TRACK")
        .current_dir(home)
        .kill_on_drop(true);
    let output = tokio::time::timeout(timeout, cmd.output())
        .await
        .map_err(|_| {
            OrchestratorError::Runtime(format!(
                "`aoe telemetry enable` did not finish within {timeout:?} while forcing \
                 telemetry on; another aoe may be holding the workspace's config lock"
            ))
        })?
        .map_err(|e| {
            OrchestratorError::Runtime(format!("failed to run `aoe telemetry enable`: {e}"))
        })?;
    if !output.status.success() {
        return Err(OrchestratorError::Runtime(format!(
            "`aoe telemetry enable` failed while forcing telemetry on: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}

fn alive(pid: i32) -> bool {
    // Signal 0 probes existence without touching the process.
    unsafe { libc::kill(pid, 0) == 0 }
}

/// Signal the whole process group (setsid makes the pgid the child's pid).
fn kill_group(pid: i32, signal: i32) {
    unsafe {
        libc::kill(-pid, signal);
    }
}

/// A currently-free loopback port. The listener is dropped before the spawn,
/// so a race with an unrelated bind is possible; the caller absorbs it by
/// probing readiness and retrying with a fresh port.
fn free_port() -> Result<u16, OrchestratorError> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")
        .map_err(|e| OrchestratorError::Runtime(format!("failed to allocate a port: {e}")))?;
    Ok(listener
        .local_addr()
        .map_err(|e| OrchestratorError::Runtime(format!("failed to read allocated port: {e}")))?
        .port())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestrator::PLAINTEXT_FLAG;

    /// Linux refuses to exec a file (ETXTBSY) while any process holds it open
    /// for writing, and a fork in another test thread inherits our write fd
    /// until its own exec. Tests that write a fake aoe and run it hold this
    /// from the write until the last exec so no fork can overlap a write.
    static FAKE_AOE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    /// Force-on is asserted by aoe's own CLI, and a failure has to fail the
    /// start: a workspace that came up anyway would be serving under a policy
    /// the admin set and CityHall did not apply. The other policies must not run
    /// the command at all, which is what makes them work with an aoe too old to
    /// have it.
    #[tokio::test]
    async fn force_on_fails_the_start_when_aoe_cannot_apply_it() {
        let _guard = FAKE_AOE_LOCK.lock().await;
        let orch = scratch("telemetry");
        let home = orch.user_dir(1).join("home");
        std::fs::create_dir_all(&home).unwrap();

        let works = fake_aoe(&orch, "aoe-ok", "#!/bin/sh\nexit 0\n");
        let broken = fake_aoe(
            &orch,
            "aoe-old",
            "#!/bin/sh\necho 'unknown subcommand' >&2\nexit 1\n",
        );

        let spec = WorkspaceSpec {
            telemetry: TelemetryPolicy::ForceOn,
            ..spec_for(1)
        };
        assert!(apply_force_on(&spec, &works, &home, FORCE_ON_TIMEOUT)
            .await
            .is_ok());
        let err = apply_force_on(&spec, &broken, &home, FORCE_ON_TIMEOUT)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("telemetry enable"), "{err}");
        assert!(
            err.contains("unknown subcommand"),
            "must carry aoe's own stderr: {err}"
        );

        // Nothing is run for the other two, so a missing binary is fine.
        let missing = orch.root.join("does-not-exist");
        for policy in [TelemetryPolicy::UserChoice, TelemetryPolicy::ForceOff] {
            let spec = WorkspaceSpec {
                telemetry: policy,
                ..spec_for(1)
            };
            assert!(apply_force_on(&spec, &missing, &home, FORCE_ON_TIMEOUT)
                .await
                .is_ok());
        }
    }

    /// aoe locks the data dir it is about to write, so a concurrent aoe holding
    /// that lock makes this command wait. Unbounded it would hold a runtime
    /// worker and hang the start with nothing to read; the deadline turns that
    /// into a start failure that says what is likely wrong.
    #[tokio::test]
    async fn a_hung_telemetry_enable_hits_the_deadline() {
        let _guard = FAKE_AOE_LOCK.lock().await;
        let orch = scratch("telemetry-hang");
        let home = orch.user_dir(1).join("home");
        std::fs::create_dir_all(&home).unwrap();
        let hangs = fake_aoe(&orch, "aoe-hangs", "#!/bin/sh\nsleep 30\n");

        let spec = WorkspaceSpec {
            telemetry: TelemetryPolicy::ForceOn,
            ..spec_for(1)
        };
        let err = apply_force_on(&spec, &hangs, &home, Duration::from_millis(100))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("did not finish"), "{err}");
        // `kill_on_drop` reaps the child when the expired timeout drops it, so
        // no `sleep` is left behind holding the workspace's lock.
        assert!(
            err.contains("config lock"),
            "must name the likely cause: {err}"
        );
    }

    /// An executable stand-in for the aoe binary.
    fn fake_aoe(orch: &ProcessOrchestrator, name: &str, body: &str) -> PathBuf {
        let path = orch.root.join(name);
        std::fs::write(&path, body).unwrap();
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
        std::fs::set_permissions(&path, perms).unwrap();
        path
    }

    fn spec_for(user_id: i32) -> WorkspaceSpec {
        WorkspaceSpec {
            user_id,
            image: "unused".to_string(),
            version: "v1.0.0".to_string(),
            bundle: None,
            agent_env: crate::agent_credentials::AgentEnv::default(),
            daemon_token: None,
            telemetry: TelemetryPolicy::default(),
            agents: String::new(),
            mode: WorkspaceMode::Composer,
            apply_mode_to_running: false,
        }
    }

    #[test]
    fn serve_args_pass_the_plaintext_flag_only_for_aoe_1_22_and_later() {
        for (version, want) in [("v1.21.0", false), ("v1.22.0", true), ("v1.26.0", true)] {
            let spec = WorkspaceSpec {
                version: version.to_string(),
                ..spec_for(1)
            };
            let args = serve_args(&spec, 4000);
            assert_eq!(args.iter().any(|a| a == PLAINTEXT_FLAG), want, "{version}");
        }
    }

    #[test]
    fn serve_args_launch_under_token_auth_never_none() {
        let args = serve_args(&spec_for(1), 4000);
        let at = args.iter().position(|a| a == "--auth").unwrap();
        assert_eq!(args[at + 1], "token");
        assert!(!args.iter().any(|a| a == "none"), "{args:?}");
    }

    fn token_spec(token: &str) -> WorkspaceSpec {
        WorkspaceSpec {
            version: "v1.26.0".to_string(),
            daemon_token: Some(crate::crypto::Secret::new(token.to_string())),
            ..spec_for(5)
        }
    }

    /// A live process in its own group for `terminate` to signal, reaped in the
    /// background so `alive` goes false once it dies.
    fn live_process() -> i32 {
        use std::os::unix::process::CommandExt;
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .process_group(0)
            .spawn()
            .unwrap();
        let pid = child.id() as i32;
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        pid
    }

    /// A live, healthy process whose recorded token fingerprint is absent (the
    /// `--auth none` era) or different must be terminated and respawned; one
    /// that matches is reused.
    #[tokio::test]
    async fn token_fingerprint_drift_respawns_and_a_match_reuses() {
        let orch = scratch("token-drift");
        std::fs::create_dir_all(orch.user_dir(5)).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            while let Ok((mut s, _)) = listener.accept().await {
                let _ = s.write_all(b"HTTP/1.0 200 OK\r\n\r\n").await;
            }
        });
        let spec = token_spec(&"b".repeat(64));
        // No download: a recorded failure makes the respawn report the missing
        // binary instead of fetching one.
        orch.provisioning
            .fail(&binary_key(&spec.version), "not installed".to_string());
        let state = |pid: i32, fp: &str| RunState {
            pid,
            port,
            version: spec.version.clone(),
            env_fingerprint: String::new(),
            telemetry: TelemetryPolicy::default().as_str().to_string(),
            mode: String::new(),
            daemon_token_fp: fp.to_string(),
        };

        let pid = live_process();
        orch.write_state(5, &state(pid, &daemon_token::fingerprint(&"b".repeat(64))))
            .unwrap();
        assert_eq!(
            orch.ensure_started(&spec).await.unwrap(),
            format!("127.0.0.1:{port}")
        );
        assert!(alive(pid), "a matching process must be left running");
        kill_group(pid, libc::SIGKILL);

        for fp in ["", "0000000000000000"] {
            let pid = live_process();
            orch.write_state(5, &state(pid, fp)).unwrap();
            // Terminated, then a respawn that stops at the missing binary.
            let err = orch.ensure_started(&spec).await.unwrap_err();
            assert!(
                matches!(err, OrchestratorError::ArtifactMissing(_)),
                "{fp:?}: {err:?}"
            );
            assert!(!alive(pid), "{fp:?}: drifted process was not terminated");
        }
    }

    #[tokio::test]
    async fn ensure_started_refuses_old_aoe_and_a_missing_token() {
        let orch = scratch("gate");
        let old = WorkspaceSpec {
            version: "v1.17.0".to_string(),
            ..token_spec(&"c".repeat(64))
        };
        assert!(orch
            .ensure_started(&old)
            .await
            .unwrap_err()
            .to_string()
            .contains("v1.17.0"));
        let no_token = WorkspaceSpec {
            version: "v1.26.0".to_string(),
            ..spec_for(1)
        };
        let err = orch.ensure_started(&no_token).await.unwrap_err();
        assert!(err.to_string().contains("daemon token"), "{err}");
    }

    #[test]
    fn state_without_daemon_token_fp_reads_back_empty() {
        let state: RunState =
            serde_json::from_str(r#"{"pid":1,"port":2,"version":"v1.0.0"}"#).unwrap();
        assert_eq!(state.daemon_token_fp, "");
    }

    #[test]
    fn open_log_is_owner_only_new_or_existing() {
        use std::os::unix::fs::PermissionsExt;
        let orch = scratch("log-mode");
        let path = orch.root.join("serve.log");
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;

        open_log(&path).unwrap();
        assert_eq!(mode(&path), 0o600);

        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        open_log(&path).unwrap();
        assert_eq!(mode(&path), 0o600);
    }

    fn scratch(name: &str) -> ProcessOrchestrator {
        let root = std::env::temp_dir().join(format!(
            "cityhall-process-test-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        ProcessOrchestrator {
            root,
            provisioning: Arc::default(),
        }
    }

    #[test]
    fn version_validation_blocks_path_escapes() {
        assert!(validate_version("v0.5.0").is_ok());
        assert!(validate_version("1.2.3-rc.1+build").is_ok());
        assert!(validate_version("").is_err());
        assert!(validate_version("../../etc").is_err());
        assert!(validate_version("v1/../..").is_err());
        assert!(validate_version(".hidden").is_err());
        assert!(validate_version("a b").is_err());
    }

    #[test]
    fn state_round_trips_atomically() {
        let orch = scratch("state");
        std::fs::create_dir_all(orch.user_dir(7)).unwrap();
        orch.write_state(
            7,
            &RunState {
                pid: 1234,
                port: 43210,
                version: "v1.0.0".to_string(),
                env_fingerprint: "somefingerprint".to_string(),
                telemetry: TelemetryPolicy::default().as_str().to_string(),
                mode: String::new(),
                daemon_token_fp: String::new(),
            },
        )
        .unwrap();
        let state = orch.read_state(7).unwrap();
        assert_eq!(
            (
                state.pid,
                state.port,
                state.version.as_str(),
                state.env_fingerprint.as_str()
            ),
            (1234, 43210, "v1.0.0", "somefingerprint")
        );
        // No leftover temp file from the atomic write.
        assert!(!orch.state_path(7).with_extension("json.tmp").exists());
    }

    /// A state.json written before `env_fingerprint` existed must still
    /// deserialize, reading back as the empty fingerprint rather than
    /// failing to load (which would otherwise strand every workspace running
    /// before an upgrade).
    #[test]
    fn state_without_env_fingerprint_still_deserializes() {
        let json = r#"{"pid":1234,"port":43210,"version":"v1.0.0"}"#;
        let state: RunState = serde_json::from_str(json).unwrap();
        assert_eq!(state.pid, 1234);
        assert_eq!(state.port, 43210);
        assert_eq!(state.version, "v1.0.0");
        assert_eq!(state.env_fingerprint, "");
    }

    /// A process from before modes existed has no `mode` in its state.json and
    /// is a Composer workspace, so a Composer spec is not drift.
    #[test]
    fn state_without_mode_reads_as_composer_and_does_not_drift() {
        let json = r#"{"pid":1,"port":2,"version":"v1.0.0"}"#;
        let state: RunState = serde_json::from_str(json).unwrap();
        let spec = spec_for(1);
        assert_eq!(spec.mode, WorkspaceMode::Composer);
        assert!(state.matches(&WorkspaceSpec {
            version: "v1.0.0".to_string(),
            ..spec.clone()
        }));
        assert!(!state.matches(&WorkspaceSpec {
            version: "v1.0.0".to_string(),
            mode: WorkspaceMode::Full,
            apply_mode_to_running: true,
            ..spec
        }));
    }

    #[test]
    fn a_mode_change_is_drift_only_on_explicit_restart() {
        let full = WorkspaceSpec {
            mode: WorkspaceMode::Full,
            apply_mode_to_running: true,
            ..spec_for(1)
        };
        let state = RunState {
            pid: 1,
            port: 2,
            version: full.version.clone(),
            env_fingerprint: full.agent_env.fingerprint.clone(),
            telemetry: full.telemetry.as_str().to_string(),
            mode: "full".to_string(),
            daemon_token_fp: String::new(),
        };
        assert!(state.matches(&full));
        // Permission removed, explicit restart: drift.
        assert!(!state.matches(&WorkspaceSpec {
            apply_mode_to_running: true,
            ..spec_for(1)
        }));
        // The same mismatch on a request-driven start leaves the live process.
        assert!(state.matches(&spec_for(1)));
        let composer = RunState {
            mode: String::new(),
            ..state
        };
        assert!(composer.matches(&full_without_restart()));
    }

    fn full_without_restart() -> WorkspaceSpec {
        WorkspaceSpec {
            mode: WorkspaceMode::Full,
            ..spec_for(1)
        }
    }

    #[test]
    fn serve_args_omit_cityhall_only_in_full_mode() {
        let composer = serve_args(&spec_for(1), 4000);
        assert!(composer.iter().any(|a| a == "--cityhall"));
        let full = serve_args(
            &WorkspaceSpec {
                mode: WorkspaceMode::Full,
                ..spec_for(1)
            },
            4000,
        );
        assert!(!full.iter().any(|a| a == "--cityhall"));
        let at = full.iter().position(|a| a == "--auth").unwrap();
        assert_eq!(full[at + 1], "token");
        assert!(!full.iter().any(|a| a == "none"));
    }

    #[tokio::test]
    async fn status_reflects_state_file_and_liveness() {
        let orch = scratch("status");
        // Nothing on disk: never created.
        assert_eq!(orch.status(1).await.unwrap(), WorkspaceStatus::NotCreated);

        // Dead pid recorded: stopped. PID 1 is init and never our child, but
        // a PID that cannot exist is the reliable "dead" case.
        std::fs::create_dir_all(orch.user_dir(2)).unwrap();
        orch.write_state(
            2,
            &RunState {
                pid: i32::MAX - 1,
                port: 1,
                version: "v1".to_string(),
                env_fingerprint: String::new(),
                telemetry: TelemetryPolicy::default().as_str().to_string(),
                mode: String::new(),
                daemon_token_fp: String::new(),
            },
        )
        .unwrap();
        assert_eq!(orch.status(2).await.unwrap(), WorkspaceStatus::Stopped);

        // A live pid (this test process): running at the recorded port.
        std::fs::create_dir_all(orch.user_dir(3)).unwrap();
        orch.write_state(
            3,
            &RunState {
                pid: std::process::id() as i32,
                port: 45678,
                version: "v1".to_string(),
                env_fingerprint: String::new(),
                telemetry: TelemetryPolicy::default().as_str().to_string(),
                mode: String::new(),
                daemon_token_fp: String::new(),
            },
        )
        .unwrap();
        assert_eq!(
            orch.status(3).await.unwrap(),
            WorkspaceStatus::Running {
                addr: "127.0.0.1:45678".to_string()
            }
        );
    }

    #[tokio::test]
    async fn missing_binary_reports_provisioning_state() {
        let orch = scratch("binary");
        let spec = WorkspaceSpec {
            user_id: 9,
            image: "unused".to_string(),
            version: "v9.9.9".to_string(),
            bundle: None,
            agent_env: crate::agent_credentials::AgentEnv::default(),
            daemon_token: Some(crate::crypto::Secret::new("a".repeat(64))),
            telemetry: TelemetryPolicy::default(),
            agents: String::new(),
            mode: WorkspaceMode::Composer,
            apply_mode_to_running: false,
        };
        // A recent failed download attempt is surfaced as guidance instead of
        // re-spawning a download on every request.
        orch.provisioning.fail(
            &binary_key("v9.9.9"),
            "downloading aoe v9.9.9 failed".to_string(),
        );
        let err = orch.ensure_started(&spec).await.unwrap_err();
        match err {
            OrchestratorError::ArtifactMissing(msg) => assert!(msg.contains("v9.9.9")),
            other => panic!("expected ArtifactMissing, got {other:?}"),
        }
    }

    #[test]
    fn release_tarball_url_is_versioned() {
        assert_eq!(
            release_tarball_url("v1.12.1", "aoe-linux-amd64"),
            "https://github.com/agent-of-empires/agent-of-empires/releases/download/v1.12.1/aoe-linux-amd64.tar.gz"
        );
    }
}
