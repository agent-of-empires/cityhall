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
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::{
    http_probe, wait_ready, Orchestrator, OrchestratorError, WorkspaceSpec, WorkspaceStatus,
};

/// Grace period between SIGTERM and SIGKILL on stop.
const STOP_TIMEOUT: Duration = Duration::from_secs(10);
/// Bind races on the pre-allocated port are rare; one respawn absorbs them.
const START_ATTEMPTS: u32 = 2;

pub struct ProcessOrchestrator {
    root: PathBuf,
}

/// Runtime state persisted across CityHall restarts.
#[derive(Serialize, Deserialize)]
struct RunState {
    pid: i32,
    port: u16,
    version: String,
}

impl ProcessOrchestrator {
    pub fn from_env() -> Self {
        ProcessOrchestrator {
            root: std::env::var("WORKSPACE_PROCESS_DIR")
                .unwrap_or_else(|_| "/var/lib/cityhall/workspaces".to_string())
                .into(),
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
            return Err(OrchestratorError::ArtifactMissing(format!(
                "aoe binary for version '{}' not found at {}; install the release \
                 tarball there (see docs/workspaces.md) or fix the version in the \
                 workspace settings",
                spec.version,
                binary.display()
            )));
        }
        let home = self.user_dir(spec.user_id).join("home");
        std::fs::create_dir_all(&home).map_err(|e| {
            OrchestratorError::Runtime(format!("failed to create workspace home: {e}"))
        })?;

        let mut last_err = None;
        for _ in 0..START_ATTEMPTS {
            let port = free_port()?;
            let addr = format!("127.0.0.1:{port}");
            let pid = self.spawn_once(spec, &binary, &home, port)?;
            self.write_state(
                spec.user_id,
                &RunState {
                    pid,
                    port,
                    version: spec.version.clone(),
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
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.user_dir(spec.user_id).join("serve.log"))
            .map_err(|e| {
                OrchestratorError::Runtime(format!("failed to open workspace log: {e}"))
            })?;
        let mut cmd = std::process::Command::new(binary);
        cmd.args([
            "serve",
            "--host",
            "127.0.0.1",
            "--port",
            &port.to_string(),
            // CityHall's session gates the proxy; the process only listens on
            // loopback.
            "--auth",
            "none",
            "--behind-proxy",
        ])
        .env("HOME", home)
        .current_dir(home)
        .stdin(std::process::Stdio::null())
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

#[async_trait]
impl Orchestrator for ProcessOrchestrator {
    async fn ensure_started(&self, spec: &WorkspaceSpec) -> Result<String, OrchestratorError> {
        validate_version(&spec.version)?;
        if let Some(state) = self.read_state(spec.user_id) {
            if alive(state.pid) {
                let addr = format!("127.0.0.1:{}", state.port);
                if state.version == spec.version && http_probe(&addr).await {
                    return Ok(addr);
                }
                // Version drift, a hung process, or a recycled PID that is
                // not our workspace: clear it and start fresh.
                tracing::info!(
                    user_id = spec.user_id,
                    from = %state.version,
                    to = %spec.version,
                    "restarting workspace process"
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

    fn scratch(name: &str) -> ProcessOrchestrator {
        let root = std::env::temp_dir().join(format!(
            "cityhall-process-test-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        ProcessOrchestrator { root }
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
            },
        )
        .unwrap();
        let state = orch.read_state(7).unwrap();
        assert_eq!(
            (state.pid, state.port, state.version.as_str()),
            (1234, 43210, "v1.0.0")
        );
        // No leftover temp file from the atomic write.
        assert!(!orch.state_path(7).with_extension("json.tmp").exists());
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
    async fn missing_binary_is_actionable() {
        let orch = scratch("binary");
        let spec = WorkspaceSpec {
            user_id: 9,
            image: "unused".to_string(),
            version: "v9.9.9".to_string(),
        };
        let err = orch.ensure_started(&spec).await.unwrap_err();
        match err {
            OrchestratorError::ArtifactMissing(msg) => {
                assert!(msg.contains("v9.9.9"));
                assert!(msg.contains("versions"));
            }
            other => panic!("expected ArtifactMissing, got {other:?}"),
        }
    }
}
