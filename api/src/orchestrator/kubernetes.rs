//! Kubernetes workspace backend.
//!
//! Shells out to `kubectl` (mirroring the docker backend's CLI approach: no
//! API-client dependencies, only structured `-o json` output is parsed) and
//! assumes CityHall runs in-cluster with a ServiceAccount allowed to manage
//! deployments, services, and PVCs in the workspace namespace (see
//! deploy/k8s/). Per user it reconciles one PVC (the data volume), one
//! ClusterIP Service (the stable endpoint), and one scale-to-zero Deployment
//! with `strategy: Recreate` so at most one pod ever mounts the volume.

use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use super::{http_probe, Orchestrator, OrchestratorError, WorkspaceSpec, WorkspaceStatus};

/// Port aoe serves on inside the workspace pod.
const AOE_PORT: u16 = 8080;
/// Where the aoe app dir lives inside the pod (the reference image runs as
/// user `aoe`); the PVC is mounted here.
const AOE_DATA_DIR: &str = "/home/aoe/.config/agent-of-empires";
const CLI_TIMEOUT: Duration = Duration::from_secs(60);
/// Pod scheduling plus a cold image pull can far exceed the shared 15s
/// readiness window, so this backend polls with its own budget.
const READY_TIMEOUT: Duration = Duration::from_secs(120);

/// In-cluster namespace file mounted into every pod with a ServiceAccount.
const NAMESPACE_FILE: &str = "/var/run/secrets/kubernetes.io/serviceaccount/namespace";

pub fn object_name(user_id: i32) -> String {
    format!("cityhall-workspace-u{user_id}")
}

pub fn pvc_name(user_id: i32) -> String {
    format!("cityhall-workspace-u{user_id}-data")
}

pub struct KubectlOrchestrator {
    namespace: String,
    /// PVC size request, e.g. `5Gi`.
    volume_size: String,
    /// Optional PVC storage class; the cluster default when unset.
    storage_class: Option<String>,
}

impl KubectlOrchestrator {
    pub fn from_env() -> Self {
        let namespace = std::env::var("WORKSPACE_K8S_NAMESPACE")
            .ok()
            .filter(|n| !n.trim().is_empty())
            .or_else(|| {
                std::fs::read_to_string(NAMESPACE_FILE)
                    .ok()
                    .map(|n| n.trim().to_string())
                    .filter(|n| !n.is_empty())
            })
            .unwrap_or_else(|| "default".to_string());
        KubectlOrchestrator {
            namespace,
            volume_size: std::env::var("WORKSPACE_K8S_VOLUME_SIZE")
                .ok()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| "5Gi".to_string()),
            storage_class: std::env::var("WORKSPACE_K8S_STORAGE_CLASS")
                .ok()
                .filter(|s| !s.trim().is_empty()),
        }
    }

    /// Run kubectl (namespace pinned), returning stdout on success. NotFound
    /// is reported as `Ok(None)` so callers can treat missing objects as
    /// state, not failure.
    async fn run(
        &self,
        args: &[&str],
        stdin: Option<String>,
    ) -> Result<Option<String>, OrchestratorError> {
        let mut cmd = Command::new("kubectl");
        cmd.arg("-n")
            .arg(&self.namespace)
            .args(args)
            .kill_on_drop(true)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let run = async {
            let mut child = cmd
                .spawn()
                .map_err(|e| OrchestratorError::Runtime(format!("failed to run kubectl: {e}")))?;
            if let Some(payload) = stdin {
                let mut pipe = child.stdin.take().expect("stdin piped");
                pipe.write_all(payload.as_bytes()).await.map_err(|e| {
                    OrchestratorError::Runtime(format!("failed to feed kubectl stdin: {e}"))
                })?;
                drop(pipe);
            }
            child
                .wait_with_output()
                .await
                .map_err(|e| OrchestratorError::Runtime(format!("kubectl failed: {e}")))
        };
        let output = tokio::time::timeout(CLI_TIMEOUT, run).await.map_err(|_| {
            OrchestratorError::Runtime(format!("kubectl {} timed out", args.join(" ")))
        })??;

        if output.status.success() {
            return Ok(Some(String::from_utf8_lossy(&output.stdout).into_owned()));
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.to_lowercase().contains("not found") {
            return Ok(None);
        }
        Err(OrchestratorError::Runtime(format!(
            "kubectl {} failed: {}",
            args.join(" "),
            stderr.trim()
        )))
    }

    /// The stable service DNS address the proxy dials from in-cluster.
    fn addr(&self, user_id: i32) -> String {
        format!("{}.{}.svc:{AOE_PORT}", object_name(user_id), self.namespace)
    }

    /// Poll until the workspace answers HTTP, surfacing image-pull failures
    /// as `ArtifactMissing` instead of a generic timeout.
    async fn wait_ready_or_diagnose(&self, spec: &WorkspaceSpec) -> Result<(), OrchestratorError> {
        let addr = self.addr(spec.user_id);
        let deadline = tokio::time::Instant::now() + READY_TIMEOUT;
        loop {
            if http_probe(&addr).await {
                return Ok(());
            }
            if let Some(reason) = self.pod_waiting_reason(spec.user_id).await? {
                if matches!(
                    reason.as_str(),
                    "ErrImagePull" | "ImagePullBackOff" | "InvalidImageName"
                ) {
                    return Err(OrchestratorError::ArtifactMissing(format!(
                        "workspace image '{}' cannot be pulled ({reason}); push it to a \
                         registry the cluster can reach or fix the image template / \
                         version in the workspace settings",
                        spec.image
                    )));
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(OrchestratorError::Runtime(format!(
                    "workspace at {addr} did not become ready within {READY_TIMEOUT:?}"
                )));
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    /// The waiting reason of the workspace pod's container, if any.
    async fn pod_waiting_reason(&self, user_id: i32) -> Result<Option<String>, OrchestratorError> {
        let selector = format!("cityhall.workspace={}", object_name(user_id));
        let out = self
            .run(&["get", "pods", "-l", &selector, "-o", "json"], None)
            .await?;
        let Some(json) = out else { return Ok(None) };
        let pods: PodList = serde_json::from_str(json.trim())
            .map_err(|e| OrchestratorError::Runtime(format!("unparseable pod list: {e}")))?;
        Ok(pods
            .items
            .into_iter()
            .flat_map(|p| p.status.container_statuses.unwrap_or_default())
            .find_map(|c| c.state.and_then(|s| s.waiting).map(|w| w.reason)))
    }
}

#[async_trait]
impl Orchestrator for KubectlOrchestrator {
    async fn ensure_started(&self, spec: &WorkspaceSpec) -> Result<String, OrchestratorError> {
        // Declarative reconcile: apply always renders the desired running
        // state (replicas 1, current image), so first start, resume, and
        // version drift are all the same operation.
        let manifests = render_manifests(
            spec,
            &self.namespace,
            &self.volume_size,
            self.storage_class.as_deref(),
        );
        self.run(&["apply", "-f", "-"], Some(manifests.to_string()))
            .await?;
        self.wait_ready_or_diagnose(spec).await?;
        Ok(self.addr(spec.user_id))
    }

    async fn stop(&self, user_id: i32) -> Result<(), OrchestratorError> {
        self.run(
            &["scale", "deployment", &object_name(user_id), "--replicas=0"],
            None,
        )
        .await?;
        Ok(())
    }

    async fn destroy(&self, user_id: i32) -> Result<(), OrchestratorError> {
        let selector = format!("cityhall.user_id={user_id}");
        // --wait=false: PVC finalizers can hold deletion for minutes; the
        // objects are already Terminating and the admin action should not
        // hang on storage teardown.
        self.run(
            &[
                "delete",
                "deployment,service,pvc",
                "-l",
                &selector,
                "--ignore-not-found=true",
                "--wait=false",
            ],
            None,
        )
        .await?;
        Ok(())
    }

    async fn status(&self, user_id: i32) -> Result<WorkspaceStatus, OrchestratorError> {
        let out = self
            .run(
                &["get", "deployment", &object_name(user_id), "-o", "json"],
                None,
            )
            .await?;
        let Some(json) = out else {
            return Ok(WorkspaceStatus::NotCreated);
        };
        let deployment: Deployment = serde_json::from_str(json.trim())
            .map_err(|e| OrchestratorError::Runtime(format!("unparseable deployment: {e}")))?;
        if deployment.status.ready_replicas.unwrap_or(0) >= 1 {
            Ok(WorkspaceStatus::Running {
                addr: self.addr(user_id),
            })
        } else {
            // Scaled to zero, scheduling, or pulling: not reachable yet.
            Ok(WorkspaceStatus::Stopped)
        }
    }
}

/// The desired-state manifests for one workspace: PVC + Service + Deployment,
/// as a `kind: List` JSON document `kubectl apply` consumes from stdin.
fn render_manifests(
    spec: &WorkspaceSpec,
    namespace: &str,
    volume_size: &str,
    storage_class: Option<&str>,
) -> serde_json::Value {
    let name = object_name(spec.user_id);
    let labels = json!({
        "cityhall.managed": "true",
        "cityhall.user_id": spec.user_id.to_string(),
        "cityhall.workspace": name,
    });
    let mut pvc_spec = json!({
        "accessModes": ["ReadWriteOnce"],
        "resources": { "requests": { "storage": volume_size } },
    });
    if let Some(class) = storage_class {
        pvc_spec["storageClassName"] = json!(class);
    }
    json!({
        "apiVersion": "v1",
        "kind": "List",
        "items": [
            {
                "apiVersion": "v1",
                "kind": "PersistentVolumeClaim",
                "metadata": { "name": pvc_name(spec.user_id), "namespace": namespace, "labels": labels },
                "spec": pvc_spec,
            },
            {
                "apiVersion": "v1",
                "kind": "Service",
                "metadata": { "name": name, "namespace": namespace, "labels": labels },
                "spec": {
                    "selector": { "cityhall.workspace": name },
                    "ports": [{ "port": AOE_PORT, "targetPort": AOE_PORT }],
                },
            },
            {
                "apiVersion": "apps/v1",
                "kind": "Deployment",
                "metadata": { "name": name, "namespace": namespace, "labels": labels },
                "spec": {
                    "replicas": 1,
                    // Never two pods on one RWO volume during a version change.
                    "strategy": { "type": "Recreate" },
                    "selector": { "matchLabels": { "cityhall.workspace": name } },
                    "template": {
                        "metadata": {
                            "labels": labels,
                            // Version drives pod recreation on apply.
                            "annotations": { "cityhall.workspace.version": spec.version },
                        },
                        "spec": {
                            "containers": [{
                                "name": "aoe",
                                "image": spec.image,
                                "command": ["aoe"],
                                "args": [
                                    "serve",
                                    "--host", "0.0.0.0",
                                    "--port", AOE_PORT.to_string(),
                                    // CityHall's session gates the proxy; the
                                    // shipped NetworkPolicy keeps other pods out.
                                    "--auth", "none",
                                    "--behind-proxy",
                                ],
                                "ports": [{ "containerPort": AOE_PORT }],
                                "volumeMounts": [{ "name": "data", "mountPath": AOE_DATA_DIR }],
                            }],
                            "volumes": [{
                                "name": "data",
                                "persistentVolumeClaim": { "claimName": pvc_name(spec.user_id) },
                            }],
                        },
                    },
                },
            },
        ],
    })
}

#[derive(Deserialize)]
struct Deployment {
    status: DeploymentStatus,
}

#[derive(Deserialize)]
struct DeploymentStatus {
    #[serde(rename = "readyReplicas")]
    ready_replicas: Option<i32>,
}

#[derive(Deserialize)]
struct PodList {
    items: Vec<Pod>,
}

#[derive(Deserialize)]
struct Pod {
    status: PodStatus,
}

#[derive(Deserialize)]
struct PodStatus {
    #[serde(rename = "containerStatuses")]
    container_statuses: Option<Vec<ContainerStatus>>,
}

#[derive(Deserialize)]
struct ContainerStatus {
    state: Option<ContainerStateK8s>,
}

#[derive(Deserialize)]
struct ContainerStateK8s {
    waiting: Option<WaitingState>,
}

#[derive(Deserialize)]
struct WaitingState {
    reason: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> WorkspaceSpec {
        WorkspaceSpec {
            user_id: 42,
            image: "registry.example.com/aoe:v1.0.0".to_string(),
            version: "v1.0.0".to_string(),
        }
    }

    #[test]
    fn names_are_deterministic() {
        assert_eq!(object_name(42), "cityhall-workspace-u42");
        assert_eq!(pvc_name(42), "cityhall-workspace-u42-data");
    }

    #[test]
    fn manifests_render_pvc_service_and_recreate_deployment() {
        let m = render_manifests(&spec(), "cityhall", "5Gi", None);
        let items = m["items"].as_array().unwrap();
        assert_eq!(items.len(), 3);
        let (pvc, svc, dep) = (&items[0], &items[1], &items[2]);

        assert_eq!(pvc["kind"], "PersistentVolumeClaim");
        assert_eq!(pvc["metadata"]["name"], "cityhall-workspace-u42-data");
        assert_eq!(pvc["spec"]["resources"]["requests"]["storage"], "5Gi");
        // No storageClassName key unless configured: the cluster default applies.
        assert!(pvc["spec"].get("storageClassName").is_none());

        assert_eq!(
            svc["spec"]["selector"]["cityhall.workspace"],
            "cityhall-workspace-u42"
        );

        assert_eq!(dep["spec"]["strategy"]["type"], "Recreate");
        assert_eq!(dep["spec"]["replicas"], 1);
        assert_eq!(
            dep["spec"]["template"]["metadata"]["annotations"]["cityhall.workspace.version"],
            "v1.0.0"
        );
        let container = &dep["spec"]["template"]["spec"]["containers"][0];
        assert_eq!(container["image"], "registry.example.com/aoe:v1.0.0");
        assert_eq!(
            container["volumeMounts"][0]["mountPath"],
            "/home/aoe/.config/agent-of-empires"
        );
    }

    #[test]
    fn manifests_pin_storage_class_when_configured() {
        let m = render_manifests(&spec(), "cityhall", "10Gi", Some("fast-ssd"));
        assert_eq!(m["items"][0]["spec"]["storageClassName"], "fast-ssd");
    }

    #[test]
    fn deployment_status_parses() {
        let running: Deployment =
            serde_json::from_str(r#"{"status":{"readyReplicas":1}}"#).unwrap();
        assert_eq!(running.status.ready_replicas, Some(1));
        let stopped: Deployment = serde_json::from_str(r#"{"status":{}}"#).unwrap();
        assert_eq!(stopped.status.ready_replicas, None);
    }

    #[test]
    fn pod_waiting_reason_parses() {
        let json = r#"{"items":[{"status":{"containerStatuses":[
            {"state":{"waiting":{"reason":"ImagePullBackOff","message":"..."}}}
        ]}}]}"#;
        let pods: PodList = serde_json::from_str(json).unwrap();
        let reason = pods
            .items
            .into_iter()
            .flat_map(|p| p.status.container_statuses.unwrap_or_default())
            .find_map(|c| c.state.and_then(|s| s.waiting).map(|w| w.reason));
        assert_eq!(reason.as_deref(), Some("ImagePullBackOff"));
    }
}
