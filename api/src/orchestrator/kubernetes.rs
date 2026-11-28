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

use super::{
    daemon_token, http_probe, plaintext_flag, proxy_allowed_host, proxy_allowed_origin,
    refused_error, Orchestrator, OrchestratorError, Probe, TelemetryPolicy, WorkspaceMode,
    WorkspaceRuntime, WorkspaceSpec, WorkspaceStatus,
};

/// Pod template annotation recording a Full mode workspace. Absent means Composer.
const MODE_ANNOTATION: &str = "cityhall.workspace/mode";
/// Fingerprint of the bundle token on the pod template. The Secret alone
/// changing does not restart a running pod, so a rotation has to change the
/// template to reach a pod, and only on an explicit restart (see `ensure_started`).
const BUNDLE_TOKEN_ANNOTATION: &str = "cityhall.workspace/bundle-token";

/// Port aoe serves on inside the workspace pod.
const AOE_PORT: u16 = 8080;
/// The `$HOME` of the reference image's user `aoe`; the PVC is mounted here, so
/// everything under it (aoe's app dir, agent logins, installs) persists.
const HOME_DIR: &str = "/home/aoe";
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

/// Name of the per-user Secret carrying the bundle token and agent
/// credentials, when there is anything to inject.
pub fn secret_name(user_id: i32) -> String {
    format!("cityhall-workspace-u{user_id}-env")
}

pub struct KubectlOrchestrator {
    namespace: String,
    /// PVC size request, e.g. `20Gi`.
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
                .unwrap_or_else(|| "20Gi".to_string()),
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
            match http_probe(&addr).await {
                Probe::Ready => return Ok(()),
                Probe::Refused(detail) => return Err(refused_error(&addr, &detail)),
                Probe::NotReady => {}
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
        daemon_token::require_or_stop(&spec.version, spec.user_id, || self.stop(spec.user_id))
            .await?;
        if spec.daemon_token.is_none() {
            return Err(daemon_token::missing_token());
        }
        // A running pod keeps the mode it started with on a request-driven
        // start: the mode annotation sits on the pod template, so applying a
        // different one would roll a healthy workspace. Only an explicit
        // restart, or a start from scaled to zero, applies the requested mode.
        let live_json = if spec.apply_mode_to_running {
            None
        } else {
            self.run(
                &[
                    "get",
                    "deployment",
                    &object_name(spec.user_id),
                    "-o",
                    "json",
                ],
                None,
            )
            .await?
        };
        let live = live_json.as_deref().and_then(live_mode);
        let spec = &WorkspaceSpec {
            mode: live.unwrap_or(spec.mode),
            ..spec.clone()
        };
        // Declarative reconcile: apply always renders the desired running
        // state (replicas 1, current image), so first start, resume, and
        // version drift are all the same operation.
        let mut manifests = render_manifests(
            spec,
            &self.namespace,
            &self.volume_size,
            self.storage_class.as_deref(),
        );
        // Like the mode, a running pod keeps the bundle token it started with on a
        // request-driven start, so a rotation does not roll a healthy workspace.
        if let Some(fp) = live_json.as_deref().and_then(live_bundle_fingerprint) {
            pin_bundle_fingerprint(&mut manifests, fp);
        }
        self.run(&["apply", "-f", "-"], Some(manifests.to_string()))
            .await?;
        // `apply` only reconciles the objects it is handed, so dropping the
        // Secret from the manifest does not remove it. A user who deletes their
        // last credential would otherwise leave it sitting in a live Secret,
        // unreferenced by any pod and invisible in the UI. Deleting it here is
        // what keeps the object from outliving the values it holds.
        if env_values(spec).is_empty() {
            self.run(
                &[
                    "delete",
                    "secret",
                    &secret_name(spec.user_id),
                    "--ignore-not-found=true",
                ],
                None,
            )
            .await?;
        }
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
                "deployment,service,pvc,secret",
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

    async fn running_mode(&self, user_id: i32) -> Result<Option<WorkspaceMode>, OrchestratorError> {
        let out = self
            .run(
                &["get", "deployment", &object_name(user_id), "-o", "json"],
                None,
            )
            .await?;
        let Some(json) = out else {
            return Ok(None);
        };
        let deployment: Deployment = serde_json::from_str(json.trim())
            .map_err(|e| OrchestratorError::Runtime(format!("unparseable deployment: {e}")))?;
        if deployment.status.ready_replicas.unwrap_or(0) < 1 {
            return Ok(None);
        }
        Ok(live_mode(&json))
    }

    /// One `kubectl get deployments` for the whole fleet, replacing a `get
    /// deployment` per user for status and another for the mode.
    async fn statuses(&self) -> Result<Option<Vec<WorkspaceRuntime>>, OrchestratorError> {
        let out = self
            .run(
                &[
                    "get",
                    "deployments",
                    "-l",
                    "cityhall.managed=true",
                    "-o",
                    "json",
                ],
                None,
            )
            .await?
            .unwrap_or_default();
        Ok(Some(parse_deployments(&out, |id| self.addr(id))))
    }
}

/// The runtime rows of a `kubectl get deployments -o json` list. A Deployment
/// with no parseable `cityhall.user_id` label is skipped, not fatal. `addr`
/// is filled only for a ready one, the same condition `status()` reports
/// `Running` under.
fn parse_deployments(json: &str, addr: impl Fn(i32) -> String) -> Vec<WorkspaceRuntime> {
    let Ok(list) = serde_json::from_str::<serde_json::Value>(json.trim()) else {
        return Vec::new();
    };
    let items = list["items"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    items
        .iter()
        .filter_map(|item| {
            let user_id: i32 = item["metadata"]["labels"]["cityhall.user_id"]
                .as_str()?
                .parse()
                .ok()?;
            let running = item["status"]["readyReplicas"].as_i64().unwrap_or(0) >= 1;
            Some(WorkspaceRuntime {
                user_id,
                running,
                version: item["spec"]["template"]["metadata"]["annotations"]
                    ["cityhall.workspace.version"]
                    .as_str()
                    .map(str::to_string),
                mode: running.then(|| live_mode(&item.to_string())).flatten(),
                addr: running.then(|| addr(user_id)),
            })
        })
        .collect()
}

/// The mode a Deployment is running in, or `None` when it is scaled to zero
/// (or unparseable), in which case the requested mode applies. The mode is
/// recorded only for Full, so an absent annotation on a live Deployment is
/// Composer.
fn live_mode(deployment_json: &str) -> Option<WorkspaceMode> {
    let v: serde_json::Value = serde_json::from_str(deployment_json.trim()).ok()?;
    if v["spec"]["replicas"].as_i64().unwrap_or(0) < 1 {
        return None;
    }
    Some(WorkspaceMode::from_stored(
        v["spec"]["template"]["metadata"]["annotations"][MODE_ANNOTATION].as_str(),
    ))
}

/// The bundle-token fingerprint a live Deployment's pods were created with
/// (`Some(None)` when it carries none), or `None` when it is scaled to zero or
/// unparseable, in which case the requested token applies.
fn live_bundle_fingerprint(deployment_json: &str) -> Option<Option<String>> {
    let v: serde_json::Value = serde_json::from_str(deployment_json.trim()).ok()?;
    if v["spec"]["replicas"].as_i64().unwrap_or(0) < 1 {
        return None;
    }
    Some(
        v["spec"]["template"]["metadata"]["annotations"][BUNDLE_TOKEN_ANNOTATION]
            .as_str()
            .map(str::to_string),
    )
}

/// Make the Deployment's template carry `live` (or no annotation) instead of
/// the requested fingerprint, so applying it does not roll the pod.
fn pin_bundle_fingerprint(manifests: &mut serde_json::Value, live: Option<String>) {
    let Some(items) = manifests["items"].as_array_mut() else {
        return;
    };
    for item in items.iter_mut().filter(|i| i["kind"] == "Deployment") {
        let annotations = &mut item["spec"]["template"]["metadata"]["annotations"];
        match &live {
            Some(fp) => annotations[BUNDLE_TOKEN_ANNOTATION] = json!(fp),
            None => {
                if let Some(map) = annotations.as_object_mut() {
                    map.remove(BUNDLE_TOKEN_ANNOTATION);
                }
            }
        }
    }
}

/// Every environment value the workspace needs: the bundle location and
/// token (when configured), every agent credential, and the daemon token. Empty when there is
/// nothing to inject, in which case the caller emits neither a Secret nor an
/// `envFrom` referencing one.
///
/// `BTreeMap` rather than `Vec` because this becomes a Secret's `stringData`
/// object, which has no order to preserve, and a stable key order keeps the
/// rendered manifest deterministic for tests and diffs.
fn env_values(spec: &WorkspaceSpec) -> std::collections::BTreeMap<String, String> {
    let mut values = std::collections::BTreeMap::new();
    if let Some(bundle) = &spec.bundle {
        values.insert("AOE_CITYHALL_BUNDLE_URL".to_string(), bundle.url.clone());
        values.insert(
            "AOE_CITYHALL_BUNDLE_TOKEN".to_string(),
            bundle.token.clone(),
        );
    }
    for (name, value) in spec.telemetry.env_pairs() {
        values.insert(name.to_string(), value.to_string());
    }
    for (name, value) in &spec.agent_env.pairs {
        values.insert(name.clone(), value.expose().to_string());
    }
    if let Some(token) = &spec.daemon_token {
        values.insert(
            daemon_token::ENV_VAR.to_string(),
            token.expose().to_string(),
        );
    }
    values
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

    let mut container = json!({
        "name": "aoe",
        "image": spec.image,
        // No `command`: that would override the image's entrypoint, which is
        // where the reference image seeds `$HOME` on the
        // volume. Passing the whole command as `args` instead matches
        // what the docker backend does, and an image with no entrypoint runs
        // these as the command anyway. The wrapper (never the entrypoint) writes
        // the daemon token, and for force-on asserts the telemetry policy, before
        // aoe serves.
        "args": daemon_token::wrap_command(spec.telemetry, vec![
            "aoe".to_string(),
            "serve".to_string(),
            "--host".to_string(), "0.0.0.0".to_string(),
            "--port".to_string(), AOE_PORT.to_string(),
            // aoe adopts the per-workspace token the wrapper
            // writes (see `daemon_token`); the proxy presents
            // it as the bearer.
            "--auth".to_string(), "token".to_string(),
            "--behind-proxy".to_string(),
            // The proxy forwards the public Host and
            // Origin, both of which aoe's DNS-rebinding
            // gate requires on the allowlist.
            "--allowed-host".to_string(), proxy_allowed_host(),
            "--allowed-origin".to_string(), proxy_allowed_origin(),
        ]
        .into_iter()
        // Composer mode is the locked-down end-user client (composer +
        // structured view only); Full mode omits the flag.
        .chain(spec.mode.cityhall_flag().map(String::from))
        .chain(plaintext_flag(&spec.version))
        .collect()),
        "ports": [{ "containerPort": AOE_PORT }],
        // No root and no sudo in a workspace: without this a setuid binary in a
        // derived image could still elevate. No read-only root filesystem, a
        // workspace writes outside $HOME (tmux sockets, /tmp, installs).
        "securityContext": {
            "allowPrivilegeEscalation": false,
            "capabilities": { "drop": ["ALL"] },
            "seccompProfile": { "type": "RuntimeDefault" },
        },
        // `subPath` keeps the volume's `lost+found` out of `$HOME`.
        "volumeMounts": [{ "name": "data", "mountPath": HOME_DIR, "subPath": "home" }],
    });

    // Which coding agents to install is not a secret, so it goes straight on the
    // container rather than into the per-user Secret the credentials use. Only
    // emitted when something is configured, so a pod template without the
    // feature in use is byte-identical to what it was before.
    if !spec.agents.is_empty() {
        container["env"] = json!([{ "name": crate::agents::ENV_VAR, "value": spec.agents }]);
    }

    let env_values = env_values(spec);
    let mut items = vec![
        json!({
            "apiVersion": "v1",
            "kind": "PersistentVolumeClaim",
            "metadata": { "name": pvc_name(spec.user_id), "namespace": namespace, "labels": labels },
            "spec": pvc_spec,
        }),
        json!({
            "apiVersion": "v1",
            "kind": "Service",
            "metadata": { "name": name, "namespace": namespace, "labels": labels },
            "spec": {
                "selector": { "cityhall.workspace": name },
                "ports": [{ "port": AOE_PORT, "targetPort": AOE_PORT }],
            },
        }),
    ];

    // A literal `env` array on the Deployment would put the bundle token and
    // every agent credential in plaintext anywhere `kubectl get deployment -o
    // yaml` reaches. Routing them through a dedicated Secret and `envFrom`
    // keeps the Deployment manifest itself free of the values; note that a
    // Kubernetes Secret is not encrypted at rest unless the cluster enables
    // that (e.g. a KMS-backed EncryptionConfiguration), so this is about
    // keeping literals out of the Deployment, not strong secrecy on its own.
    if !env_values.is_empty() {
        container["envFrom"] = json!([{ "secretRef": { "name": secret_name(spec.user_id) } }]);
        items.push(json!({
            "apiVersion": "v1",
            "kind": "Secret",
            "metadata": { "name": secret_name(spec.user_id), "namespace": namespace, "labels": labels },
            "type": "Opaque",
            "stringData": env_values,
        }));
    }

    let mut annotations = json!({
        // Version drives pod recreation on apply.
        "cityhall.workspace.version": spec.version,
        // Editing the Secret alone does not restart running
        // pods; changing this annotation changes the pod
        // template, which is what makes the Deployment roll
        // to pick up a new credential set.
        "cityhall.workspace/env-fingerprint": spec.agent_env.fingerprint,
    });
    // Same reason as the fingerprint: the policy changes the pod's environment
    // and args, so the template has to change for the Deployment to roll.
    // Written only when a policy is actually forced, so deploying this feature
    // leaves an existing workspace's template byte-identical and does not roll
    // every pod in the namespace for a setting nobody has touched yet.
    // Fingerprint only, never the token. The changed args already roll pods
    // once on upgrade; this makes a future token rotation roll them too, since
    // the Secret alone changing does not restart running pods.
    if let Some(token) = &spec.daemon_token {
        annotations["cityhall.workspace/daemon-token"] =
            json!(daemon_token::fingerprint(token.expose()));
    }
    if spec.bundle.is_some() {
        annotations[BUNDLE_TOKEN_ANNOTATION] = json!(spec.bundle_token_fingerprint());
    }
    if spec.telemetry != TelemetryPolicy::UserChoice {
        annotations["cityhall.workspace/telemetry-policy"] = json!(spec.telemetry.as_str());
    }
    // The agent set already changes the pod template through the container's
    // `env` above, so this is not what makes a change roll. It is here so the
    // applied set is readable on the Deployment without diffing container
    // environments, the way the version annotation is. Written only when a set is
    // configured, for the same reason the telemetry annotation is.
    if !spec.agents.is_empty() {
        annotations["cityhall.workspace/agents"] = json!(spec.agents);
    }

    // The mode changes the container's args, so the template has to change for
    // the Deployment to roll. Written only for Full, so an existing Composer
    // workspace's template stays byte-identical and an upgrade rolls nothing.
    if spec.mode == WorkspaceMode::Full {
        annotations[MODE_ANNOTATION] = json!(spec.mode.as_str());
    }

    items.push(json!({
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
                    "annotations": annotations,
                },
                "spec": {
                    // The workspace never talks to the Kubernetes API, so it
                    // gets no ServiceAccount token. The reference image runs as
                    // uid 1000; fsGroup makes the volume writable by it.
                    "automountServiceAccountToken": false,
                    "securityContext": {
                        "runAsNonRoot": true,
                        "runAsUser": 1000,
                        "fsGroup": 1000,
                        // The default re-applies group-write to every directory in
                        // the volume on each pod start, and aoe refuses a $HOME
                        // whose directories are writable by their group.
                        "fsGroupChangePolicy": "OnRootMismatch",
                    },
                    // The kubelet creates a `subPath` directory root-owned and
                    // world-writable, which fsGroup does not change, and aoe
                    // refuses to run under a $HOME like that. So the directory
                    // is created first, owned by uid 1000 and not writable by
                    // anyone else. Root only here, with just the two
                    // capabilities the chown and chmod need; the workspace
                    // container never runs as root. Overriding `command` skips
                    // the image entrypoint.
                    "initContainers": [{
                        "name": "init-home",
                        "image": spec.image,
                        "command": ["install", "-d", "-o", "1000", "-g", "1000", "-m", "0755", "/vol/home"],
                        "securityContext": {
                            "runAsUser": 0,
                            "runAsNonRoot": false,
                            "allowPrivilegeEscalation": false,
                            "readOnlyRootFilesystem": true,
                            "capabilities": { "drop": ["ALL"], "add": ["CHOWN", "FOWNER"] },
                        },
                        "volumeMounts": [{ "name": "data", "mountPath": "/vol" }],
                    }],
                    "containers": [container],
                    "volumes": [{
                        "name": "data",
                        "persistentVolumeClaim": { "claimName": pvc_name(spec.user_id) },
                    }],
                },
            },
        },
    }));

    json!({
        "apiVersion": "v1",
        "kind": "List",
        "items": items,
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
    use crate::orchestrator::PLAINTEXT_FLAG;

    fn bundle_spec(token: &str) -> WorkspaceSpec {
        WorkspaceSpec {
            bundle: Some(super::super::BundleAccess {
                url: "http://cityhall/api/workspace-bundle".to_string(),
                token: token.to_string(),
            }),
            ..spec()
        }
    }

    fn template_annotation(manifests: &serde_json::Value) -> Option<String> {
        manifests["items"]
            .as_array()?
            .iter()
            .find(|i| i["kind"] == "Deployment")?["spec"]["template"]["metadata"]["annotations"]
            [BUNDLE_TOKEN_ANNOTATION]
            .as_str()
            .map(str::to_string)
    }

    /// A rotation changes the pod template only where it is meant to: an
    /// explicit restart or a scaled-to-zero start renders the new fingerprint,
    /// a running pod is pinned to the one it has.
    #[test]
    fn a_rotated_bundle_token_rolls_the_pod_only_when_not_pinned() {
        let render = |token: &str| render_manifests(&bundle_spec(token), "ns", "1Gi", None);
        let old = render("old-token");
        let new = render("new-token");
        let (old_fp, new_fp) = (template_annotation(&old), template_annotation(&new));
        assert!(old_fp.is_some() && old_fp != new_fp);

        let live = |m: &serde_json::Value| {
            json!({ "spec": { "replicas": 1, "template": m["items"].as_array().unwrap()
                .iter().find(|i| i["kind"] == "Deployment").unwrap()["spec"]["template"].clone() } })
            .to_string()
        };
        let mut pinned = new.clone();
        pin_bundle_fingerprint(&mut pinned, live_bundle_fingerprint(&live(&old)).unwrap());
        assert_eq!(template_annotation(&pinned), old_fp);

        // A Deployment from before the annotation: pinned to "none", not rolled.
        let mut legacy = new.clone();
        pin_bundle_fingerprint(&mut legacy, None);
        assert_eq!(template_annotation(&legacy), None);

        // Scaled to zero: the requested token applies.
        let zero = json!({ "spec": { "replicas": 0 } }).to_string();
        assert_eq!(live_bundle_fingerprint(&zero), None);
    }

    fn spec() -> WorkspaceSpec {
        WorkspaceSpec {
            user_id: 42,
            image: "registry.example.com/aoe:v1.0.0".to_string(),
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

    /// A distinctive credential value: any assertion below that finds this
    /// substring outside the Secret item has found a leak into the
    /// Deployment.
    /// Distinctive bundle token: a plain substring check on it cannot be
    /// confused with the word `token` in a manifest.
    const BUNDLE_PROBE: &str = "bundle-probe-7f3a";
    const SECRET_VALUE: &str = "sk-super-secret-value";

    fn spec_with_bundle_and_agent_env() -> WorkspaceSpec {
        WorkspaceSpec {
            bundle: Some(super::super::BundleAccess {
                url: "http://cityhall.cityhall.svc:3000/api/workspace-bundle".to_string(),
                token: BUNDLE_PROBE.to_string(),
            }),
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

    #[test]
    fn env_values_collects_bundle_and_agent_pairs() {
        let values = env_values(&spec_with_bundle_and_agent_env());
        assert_eq!(
            values.get("AOE_CITYHALL_BUNDLE_URL").map(String::as_str),
            Some("http://cityhall.cityhall.svc:3000/api/workspace-bundle")
        );
        assert_eq!(
            values.get("AOE_CITYHALL_BUNDLE_TOKEN").map(String::as_str),
            Some(BUNDLE_PROBE)
        );
        assert_eq!(
            values.get("ANTHROPIC_API_KEY").map(String::as_str),
            Some(SECRET_VALUE)
        );
    }

    /// Also the condition `ensure_started` deletes the Secret on: emptiness here
    /// is what says the object has nothing left to hold. A user who deletes their
    /// last credential lands back on exactly this spec, since `AgentEnv` with no
    /// pairs fingerprints empty, so it is indistinguishable from never having had
    /// one.
    #[test]
    fn env_values_is_empty_with_nothing_to_inject() {
        assert!(env_values(&spec()).is_empty());
    }

    /// The other side of that branch. With the config bundle configured, its URL
    /// and token alone keep the Secret populated, so reconciling a user who has
    /// no agent credentials must leave it in place. Getting this wrong would
    /// delete the bundle token on every reconcile rather than in the one case the
    /// deletion is for.
    #[test]
    fn a_bundle_with_no_agent_credentials_still_needs_a_secret() {
        let spec = WorkspaceSpec {
            bundle: Some(super::super::BundleAccess {
                url: "http://cityhall.cityhall.svc:3000/api/workspace-bundle".to_string(),
                token: BUNDLE_PROBE.to_string(),
            }),
            ..spec()
        };
        let values = env_values(&spec);
        assert!(!values.is_empty(), "the bundle alone must keep the Secret");
        assert!(!values.contains_key("ANTHROPIC_API_KEY"));

        let m = render_manifests(&spec, "cityhall", "5Gi", None);
        let items = m["items"].as_array().unwrap();
        assert!(items.iter().any(|i| i["kind"] == "Secret"));
        let dep = items.iter().find(|i| i["kind"] == "Deployment").unwrap();
        assert_eq!(
            dep["spec"]["template"]["spec"]["containers"][0]["envFrom"][0]["secretRef"]["name"],
            "cityhall-workspace-u42-env"
        );
    }

    #[test]
    fn names_are_deterministic() {
        assert_eq!(object_name(42), "cityhall-workspace-u42");
        assert_eq!(pvc_name(42), "cityhall-workspace-u42-data");
        assert_eq!(secret_name(42), "cityhall-workspace-u42-env");
    }

    #[test]
    fn manifests_render_pvc_service_and_recreate_deployment() {
        let m = render_manifests(&spec(), "cityhall", "5Gi", None);
        let items = m["items"].as_array().unwrap();
        // No credentials configured: PVC + Service + Deployment only, no Secret.
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
        // No credentials: an empty fingerprint annotation, not a missing key,
        // so the rendered manifest shape stays stable.
        assert_eq!(
            dep["spec"]["template"]["metadata"]["annotations"]
                ["cityhall.workspace/env-fingerprint"],
            ""
        );
        let pod = &dep["spec"]["template"]["spec"];
        assert_eq!(pod["automountServiceAccountToken"], false);
        assert_eq!(pod["securityContext"]["runAsNonRoot"], true);
        assert_eq!(pod["securityContext"]["runAsUser"], 1000);
        assert_eq!(pod["securityContext"]["fsGroup"], 1000);
        assert_eq!(
            pod["securityContext"]["fsGroupChangePolicy"],
            "OnRootMismatch"
        );
        // The only root container creates `home` for uid 1000; the workspace
        // container sets no user of its own, so it keeps the pod's non-root one.
        let init = &pod["initContainers"][0];
        assert_eq!(init["securityContext"]["runAsUser"], 0);
        assert_eq!(init["securityContext"]["runAsNonRoot"], false);
        assert_eq!(init["securityContext"]["allowPrivilegeEscalation"], false);
        assert_eq!(init["securityContext"]["readOnlyRootFilesystem"], true);
        assert_eq!(
            init["securityContext"]["capabilities"]["drop"],
            json!(["ALL"])
        );
        assert_eq!(
            init["securityContext"]["capabilities"]["add"],
            json!(["CHOWN", "FOWNER"])
        );
        assert_eq!(
            init["command"],
            json!([
                "install",
                "-d",
                "-o",
                "1000",
                "-g",
                "1000",
                "-m",
                "0755",
                "/vol/home"
            ])
        );
        assert_eq!(init["volumeMounts"][0]["mountPath"], "/vol");
        // The workspace container keeps the pod's non-root user and is hardened
        // without a read-only root filesystem.
        let main_sc = &pod["containers"][0]["securityContext"];
        assert_eq!(main_sc["allowPrivilegeEscalation"], false);
        assert_eq!(main_sc["capabilities"]["drop"], json!(["ALL"]));
        assert_eq!(main_sc["seccompProfile"]["type"], "RuntimeDefault");
        assert!(main_sc.get("readOnlyRootFilesystem").is_none());
        assert!(main_sc.get("runAsUser").is_none());
        let container = &dep["spec"]["template"]["spec"]["containers"][0];
        assert_eq!(container["image"], "registry.example.com/aoe:v1.0.0");
        // The whole command travels in `args`, and `command` stays absent so the
        // image entrypoint still runs. Setting `command` would override it, and
        // the reference image seeds `$HOME` from there, so a new workspace
        // would start without its shell defaults.
        assert!(container.get("command").is_none());
        // The wrapper is always present; aoe's own argv follows the script.
        assert_eq!(container["args"][0], "sh");
        assert_eq!(container["args"][3], "aoe");
        assert_eq!(container["args"][4], "serve");
        // Nothing to inject: no envFrom referencing a Secret that does not exist.
        assert!(container.get("envFrom").is_none());
        // No agents configured: neither the env nor the annotation is written, so
        // a pod template for an install not using the feature is byte-identical
        // to what it was before the feature existed and no pod rolls on upgrade.
        assert!(container.get("env").is_none());
        assert!(dep["spec"]["template"]["metadata"]["annotations"]
            .get("cityhall.workspace/agents")
            .is_none());
        assert_eq!(container["volumeMounts"][0]["mountPath"], "/home/aoe");
        assert_eq!(container["volumeMounts"][0]["subPath"], "home");
        // Workspaces are locked-down end-user clients, never full dashboards.
        let args = container["args"].as_array().unwrap();
        assert!(args.iter().any(|a| a == "--cityhall"));
        // --behind-proxy without this makes `aoe serve` refuse to start.
        assert!(args.iter().any(|a| a == "--allowed-host"));
    }

    /// The Secret carries the values; the Deployment references it through
    /// `envFrom` and must not contain any literal secret value itself.
    #[test]
    fn manifests_route_credentials_through_a_secret_not_the_deployment() {
        let spec = spec_with_bundle_and_agent_env();
        let m = render_manifests(&spec, "cityhall", "5Gi", None);
        let items = m["items"].as_array().unwrap();
        assert_eq!(items.len(), 4, "expected PVC, Service, Secret, Deployment");

        let secret = items
            .iter()
            .find(|i| i["kind"] == "Secret")
            .expect("a Secret must be emitted when there is something to inject");
        assert_eq!(secret["metadata"]["name"], "cityhall-workspace-u42-env");
        assert_eq!(secret["metadata"]["labels"]["cityhall.user_id"], "42");
        assert_eq!(secret["stringData"]["ANTHROPIC_API_KEY"], SECRET_VALUE);
        assert_eq!(
            secret["stringData"]["AOE_CITYHALL_BUNDLE_TOKEN"],
            BUNDLE_PROBE
        );

        let dep = items.iter().find(|i| i["kind"] == "Deployment").unwrap();
        let container = &dep["spec"]["template"]["spec"]["containers"][0];
        assert_eq!(
            container["envFrom"][0]["secretRef"]["name"],
            "cityhall-workspace-u42-env"
        );
        assert!(container.get("env").is_none());

        // The Deployment's own JSON must never carry the literal value, only
        // the Secret does.
        let dep_text = dep.to_string();
        assert!(
            !dep_text.contains(SECRET_VALUE),
            "credential value leaked into the Deployment: {dep_text}"
        );
        assert!(
            !dep_text.contains(BUNDLE_PROBE),
            "bundle token leaked into the Deployment: {dep_text}"
        );

        // Changing the fingerprint changes the pod template, which is what
        // makes the Deployment roll on apply (editing the Secret alone
        // does not restart running pods).
        assert_eq!(
            dep["spec"]["template"]["metadata"]["annotations"]
                ["cityhall.workspace/env-fingerprint"],
            "abc123fingerprint"
        );
    }

    fn spec_with_telemetry(policy: TelemetryPolicy) -> WorkspaceSpec {
        WorkspaceSpec {
            telemetry: policy,
            ..spec()
        }
    }

    fn deployment(policy: TelemetryPolicy) -> serde_json::Value {
        let m = render_manifests(&spec_with_telemetry(policy), "cityhall", "5Gi", None);
        m["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["kind"] == "Deployment")
            .unwrap()
            .clone()
    }

    fn deployment_for(spec: &WorkspaceSpec) -> serde_json::Value {
        let m = render_manifests(spec, "cityhall", "5Gi", None);
        m["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["kind"] == "Deployment")
            .unwrap()
            .clone()
    }

    #[test]
    fn container_args_omit_cityhall_only_in_full_mode() {
        let args_of = |spec: &WorkspaceSpec| {
            deployment_for(spec)["spec"]["template"]["spec"]["containers"][0]["args"]
                .as_array()
                .unwrap()
                .clone()
        };
        assert!(args_of(&spec()).iter().any(|a| a == "--cityhall"));
        let full = args_of(&WorkspaceSpec {
            mode: WorkspaceMode::Full,
            ..spec()
        });
        assert!(!full.iter().any(|a| a == "--cityhall"));
        assert!(full.iter().any(|a| a == "--auth"));
    }

    #[test]
    fn a_live_deployment_reports_its_recorded_mode_and_a_stopped_one_none() {
        let dep = |replicas: i64, annotation: Option<&str>| {
            let annotations = annotation
                .map(|m| json!({ MODE_ANNOTATION: m }))
                .unwrap_or_else(|| json!({}));
            json!({"spec": {"replicas": replicas,
                "template": {"metadata": {"annotations": annotations}}}})
            .to_string()
        };
        assert_eq!(live_mode(&dep(1, None)), Some(WorkspaceMode::Composer));
        assert_eq!(live_mode(&dep(1, Some("full"))), Some(WorkspaceMode::Full));
        // Scaled to zero: the requested mode applies.
        assert_eq!(live_mode(&dep(0, Some("full"))), None);
        assert_eq!(live_mode("not json"), None);
    }

    /// Only Full is annotated, so a Composer pod's template is unchanged by
    /// this feature (an upgrade rolls nothing) and a Full pod's differs (a mode
    /// change rolls the Deployment).
    #[test]
    fn the_mode_annotation_is_written_only_for_full() {
        let composer = deployment_for(&spec());
        assert!(composer["spec"]["template"]["metadata"]["annotations"]
            .get("cityhall.workspace/mode")
            .is_none());
        let full = deployment_for(&WorkspaceSpec {
            mode: WorkspaceMode::Full,
            ..spec()
        });
        assert_eq!(
            full["spec"]["template"]["metadata"]["annotations"]["cityhall.workspace/mode"],
            "full"
        );
        assert_ne!(composer["spec"]["template"], full["spec"]["template"]);
    }

    /// The annotation is what makes the Deployment roll, so it must appear for a
    /// forced policy and must NOT appear for `user_choice`: writing it there too
    /// would change every existing workspace's pod template on upgrade and roll
    /// every pod in the namespace for a setting nobody has touched.
    #[test]
    fn only_a_forced_telemetry_policy_annotates_the_pod_template() {
        let annotations =
            |policy| deployment(policy)["spec"]["template"]["metadata"]["annotations"].clone();
        assert!(annotations(TelemetryPolicy::UserChoice)
            .get("cityhall.workspace/telemetry-policy")
            .is_none());
        assert_eq!(
            annotations(TelemetryPolicy::ForceOn)["cityhall.workspace/telemetry-policy"],
            "force_on"
        );
    }

    #[test]
    fn container_args_pass_the_plaintext_flag_only_for_aoe_1_22_and_later() {
        for (version, want) in [("v1.21.0", false), ("v1.22.0", true), ("v1.26.0", true)] {
            let spec = WorkspaceSpec {
                version: version.to_string(),
                ..spec()
            };
            let m = render_manifests(&spec, "cityhall", "5Gi", None);
            let dep = m["items"]
                .as_array()
                .unwrap()
                .iter()
                .find(|i| i["kind"] == "Deployment")
                .unwrap();
            let args = dep["spec"]["template"]["spec"]["containers"][0]["args"]
                .as_array()
                .unwrap();
            assert_eq!(args.iter().any(|a| a == PLAINTEXT_FLAG), want, "{version}");
        }
    }

    /// Force-on wraps the container's args, never its `command`, which would
    /// take the image's ENTRYPOINT out of the picture. Every serve argument
    /// stays its own list element.
    #[test]
    fn force_on_wraps_the_container_args() {
        let dep = deployment(TelemetryPolicy::ForceOn);
        let container = &dep["spec"]["template"]["spec"]["containers"][0];
        let args = container["args"].as_array().unwrap();
        assert_eq!(args[0], "sh");
        assert_eq!(args[1], "-c");
        assert!(args[2].as_str().unwrap().contains("aoe telemetry enable"));
        assert_eq!(args[3], "aoe");
        assert_eq!(args[4], "serve");
        assert!(container.get("command").is_none());
    }

    /// Force-off travels as `DO_NOT_TRACK`, so a deployment with no credentials
    /// and no bundle still needs the Secret that policy rides in.
    #[test]
    fn force_off_injects_do_not_track_through_the_secret() {
        let m = render_manifests(
            &spec_with_telemetry(TelemetryPolicy::ForceOff),
            "cityhall",
            "5Gi",
            None,
        );
        let items = m["items"].as_array().unwrap();
        let secret = items
            .iter()
            .find(|i| i["kind"] == "Secret")
            .expect("force-off has a variable to inject, so a Secret is emitted");
        assert_eq!(secret["stringData"]["DO_NOT_TRACK"], "1");
    }

    /// The agent set is not a credential: it belongs on the container, not in
    /// the per-user Secret. It still has to change the pod template, or an
    /// admin's change would apply to nothing that already exists.
    #[test]
    fn the_agent_set_is_plain_pod_environment_and_rolls_the_deployment() {
        let spec = WorkspaceSpec {
            agents: "claude,codex".to_string(),
            ..spec()
        };
        let m = render_manifests(&spec, "cityhall", "5Gi", None);
        let items = m["items"].as_array().unwrap();
        // Agents alone are not a reason to emit a Secret.
        assert!(!items.iter().any(|i| i["kind"] == "Secret"));

        let dep = items.iter().find(|i| i["kind"] == "Deployment").unwrap();
        let container = &dep["spec"]["template"]["spec"]["containers"][0];
        assert_eq!(container["env"][0]["name"], "CITYHALL_AGENTS");
        assert_eq!(container["env"][0]["value"], "claude,codex");
        assert!(container.get("envFrom").is_none());
        assert_eq!(
            dep["spec"]["template"]["metadata"]["annotations"]["cityhall.workspace/agents"],
            "claude,codex"
        );
    }

    const TEST_TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn spec_with_token() -> WorkspaceSpec {
        WorkspaceSpec {
            daemon_token: Some(crate::crypto::Secret::new(TEST_TOKEN.to_string())),
            ..spec()
        }
    }

    /// The token rides in the per-user Secret, only its fingerprint is on the
    /// pod template, and aoe is launched under `--auth token`.
    #[test]
    fn the_daemon_token_rides_the_secret_and_only_its_fingerprint_rolls_pods() {
        let m = render_manifests(&spec_with_token(), "cityhall", "5Gi", None);
        let items = m["items"].as_array().unwrap();
        let secret = items.iter().find(|i| i["kind"] == "Secret").unwrap();
        assert_eq!(secret["stringData"]["CITYHALL_DAEMON_TOKEN"], TEST_TOKEN);

        let dep = items.iter().find(|i| i["kind"] == "Deployment").unwrap();
        let dep_text = dep.to_string();
        assert!(!dep_text.contains(TEST_TOKEN), "{dep_text}");
        let fp = daemon_token::fingerprint(TEST_TOKEN);
        assert_eq!(
            dep["spec"]["template"]["metadata"]["annotations"]["cityhall.workspace/daemon-token"],
            fp.as_str()
        );

        let args = dep["spec"]["template"]["spec"]["containers"][0]["args"]
            .as_array()
            .unwrap();
        let at = args.iter().position(|a| a == "--auth").unwrap();
        assert_eq!(args[at + 1], "token");
        assert!(!args.iter().any(|a| a == "none"), "{args:?}");

        // A different token changes the template, so the Deployment rolls.
        let other = WorkspaceSpec {
            daemon_token: Some(crate::crypto::Secret::new("f".repeat(64))),
            ..spec()
        };
        let m2 = render_manifests(&other, "cityhall", "5Gi", None);
        let dep2 = m2["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["kind"] == "Deployment")
            .unwrap();
        assert_ne!(
            dep["spec"]["template"]["metadata"]["annotations"],
            dep2["spec"]["template"]["metadata"]["annotations"]
        );
    }

    #[tokio::test]
    async fn ensure_started_refuses_old_aoe_and_a_missing_token() {
        let orch = KubectlOrchestrator::from_env();
        let old = WorkspaceSpec {
            version: "v1.17.0".to_string(),
            ..spec_with_token()
        };
        let err = orch.ensure_started(&old).await.unwrap_err().to_string();
        assert!(err.contains("v1.17.0"), "{err}");
        let no_token = WorkspaceSpec {
            version: "v1.26.0".to_string(),
            ..spec()
        };
        let err = orch
            .ensure_started(&no_token)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("daemon token"), "{err}");
    }

    #[test]
    fn manifests_pin_storage_class_when_configured() {
        let m = render_manifests(&spec(), "cityhall", "10Gi", Some("fast-ssd"));
        assert_eq!(m["items"][0]["spec"]["storageClassName"], "fast-ssd");
    }

    /// A `kubectl get deployments` list item.
    fn dep_item(
        user: &str,
        ready: Option<i64>,
        replicas: i64,
        mode: Option<&str>,
    ) -> serde_json::Value {
        let annotations = match mode {
            Some(m) => json!({ MODE_ANNOTATION: m, "cityhall.workspace.version": "v1" }),
            None => json!({ "cityhall.workspace.version": "v1" }),
        };
        let mut status = json!({});
        if let Some(r) = ready {
            status["readyReplicas"] = json!(r);
        }
        json!({
            "metadata": { "labels": { "cityhall.user_id": user } },
            "spec": { "replicas": replicas,
                "template": { "metadata": { "annotations": annotations } } },
            "status": status,
        })
    }

    #[test]
    fn running_mode_needs_a_ready_pod_and_a_live_template() {
        // The same rule `running_mode()` applies to one `get deployment`.
        let mode = |item: serde_json::Value| {
            parse_deployments(&json!({ "items": [item] }).to_string(), |_| String::new())[0].mode
        };
        assert_eq!(
            mode(dep_item("1", Some(1), 1, Some("full"))),
            Some(WorkspaceMode::Full)
        );
        assert_eq!(
            mode(dep_item("1", Some(1), 1, None)),
            Some(WorkspaceMode::Composer)
        );
        // Not ready (scheduling or pulling) and scaled to zero: no mode.
        assert_eq!(mode(dep_item("1", None, 1, Some("full"))), None);
        assert_eq!(mode(dep_item("1", Some(0), 0, Some("full"))), None);
    }

    #[test]
    fn one_deployment_list_answers_status_mode_and_address_for_all() {
        let list = json!({ "items": [
            dep_item("1", Some(1), 1, Some("full")),
            dep_item("2", Some(1), 1, None),
            dep_item("3", None, 0, None),
            // No usable identity: skipped, not fatal.
            dep_item("not-a-number", Some(1), 1, None),
        ]})
        .to_string();
        let rows = parse_deployments(&list, |id| format!("w{id}:{AOE_PORT}"));
        assert_eq!(
            rows,
            vec![
                WorkspaceRuntime {
                    user_id: 1,
                    running: true,
                    version: Some("v1".to_string()),
                    mode: Some(WorkspaceMode::Full),
                    addr: Some(format!("w1:{AOE_PORT}")),
                },
                WorkspaceRuntime {
                    user_id: 2,
                    running: true,
                    version: Some("v1".to_string()),
                    mode: Some(WorkspaceMode::Composer),
                    addr: Some(format!("w2:{AOE_PORT}")),
                },
                WorkspaceRuntime {
                    user_id: 3,
                    running: false,
                    version: Some("v1".to_string()),
                    mode: None,
                    addr: None,
                },
            ]
        );
        assert!(parse_deployments("", |_| String::new()).is_empty());
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
