//! Kubernetes implementation of the SandboxProvider trait.

use async_trait::async_trait;
use axum::response::sse::Event;
use futures::stream::BoxStream;
use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::core::v1::Pod;
use kube::api::{Api, ListParams};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use tracing::{info, warn};

use super::client::K8sClient;
use super::deployment;
use super::ingress;
use super::naming::generate_service_name;
use super::pod_builder;
use super::pod_reader;
use super::pod_status::is_sandbox_container_ready;
use super::pvc;
use super::service;
use super::status_resolver::StatusResolver;
use super::storage::StorageBackend;
use crate::config::Config;
use crate::events::EventPublisher;
use crate::sandbox::client::SandboxClient;
use crate::sandbox::error::SandboxError;
use crate::sandbox::jwt;
use crate::sandbox::models::*;
use crate::sandbox::provider::SandboxProvider;
use crate::sandbox::schemas::*;

/// Kubernetes-based sandbox provider.
///
/// Pure infrastructure provider — no database dependencies.
/// DB concerns (server configs, auto-provisioning) are handled by callers.
pub struct K8sSandboxProvider {
    pub config: Arc<Config>,
    pub k8s: K8sClient,
    pub status_resolver: StatusResolver,
    /// Optional event publisher for environment lifecycle events.
    pub event_publisher: Option<Arc<dyn EventPublisher>>,
    /// Optional S3 client used to delete a task's transcript archive on cleanup.
    /// The sandbox uploads transcripts via backend-proxied gRPC (no pod creds);
    /// the backend owns the credentials, so cleanup of `<prefix>/<task_id>/` lives
    /// here. `None` when S3 isn't configured — cleanup then skips the delete.
    pub s3: Option<Arc<aws_sdk_s3::Client>>,
}

#[async_trait]
impl SandboxProvider for K8sSandboxProvider {
    async fn provision(
        &self,
        task_id: &str,
        request: &ProvisionRequest,
    ) -> Result<ProvisionResponse, SandboxError> {
        // Time the whole provision so the `ebs` PVC path (attach) and the
        // node-local backends can be compared. The body runs in an inner async
        // block so every early return is captured at a single measurement point.
        let backend =
            StorageBackend::from_plan(&request.plan, &self.config.sandbox.default_storage_mode);
        let started = std::time::Instant::now();
        let result: Result<ProvisionResponse, SandboxError> = async {
        info!(task_id = %task_id, user_id = %request.user_id, "Provisioning sandbox");

        let plan = &request.plan;

        // 1. Check for existing pod — never destroy, only reuse, wait, or rebuild on tag change
        match self.check_existing_pod(task_id).await {
            ExistingPodResult::Ready => {
                info!(task_id = %task_id, "Reusing existing healthy pod");
                return Ok(ProvisionResponse {
                    status: SandboxStatus::Ready,
                });
            }
            ExistingPodResult::Initializing => {
                info!(task_id = %task_id, "Pod exists but not ready yet, returning Initializing");
                return Ok(ProvisionResponse {
                    status: SandboxStatus::Initializing,
                });
            }
            ExistingPodResult::Stopped { current_image } => {
                let expected = pod_builder::resolve_sandbox_image(&self.config, plan);
                if current_image.as_deref() == Some(expected.as_str()) {
                    // Tag unchanged — fast path: scale back up.
                    info!(task_id = %task_id, image = %expected, "Starting stopped sandbox (image unchanged), scaling up");
                    // A stop→start collision (e.g. the idle reaper scales to 0
                    // and the user immediately reopens the task) can leave the
                    // old pod still Terminating while it holds the ReadWriteOnce
                    // PVC. Scaling straight to 1 would schedule a new pod that
                    // races that attachment → "Multi-Attach" stall. Wait for the
                    // old pod to fully terminate first (no-op if already gone).
                    deployment::wait_for_pods_deleted(&self.k8s, task_id).await;
                    deployment::scale_deployment(&self.k8s, task_id, 1)
                        .await
                        .map_err(SandboxError::KubeError)?;
                } else {
                    // Tag changed — rebuild the Deployment from the fresh plan.
                    info!(
                        task_id = %task_id,
                        old_image = ?current_image,
                        new_image = %expected,
                        "Sandbox image changed since stop, rebuilding deployment"
                    );
                    let token = self.generate_user_auth_token(&request.user_id)?;
                    deployment::redeploy_with_plan(
                        &self.k8s,
                        &self.config,
                        task_id,
                        request,
                        &token,
                    )
                    .await?;
                }
                self.publish_initializing(task_id).await;
                return Ok(ProvisionResponse {
                    status: SandboxStatus::Initializing,
                });
            }
            ExistingPodResult::NotFound => {} // Continue provisioning
        }

        // 2. Generate JWT using user_id directly (no DB lookup)
        let user_auth_token = self.generate_user_auth_token(&request.user_id)?;

        // 3. Create PVC
        let pvc_name = match self.create_or_find_pvc(task_id, plan).await {
            Ok(name) if !name.is_empty() => Some(name),
            Ok(_) => None,
            Err(e) => {
                warn!(task_id = %task_id, error = %e, "Failed to create PVC, proceeding without persistence");
                None
            }
        };

        // 4. Build pod spec. Secrets flow via gRPC `GetCredentials` and are
        //    materialized to `/opt/moatless/data/server.env` by the sandbox init.
        let pod = pod_builder::build_pod(
            &self.config,
            task_id,
            plan,
            pvc_name.as_deref(),
            Some(&user_auth_token),
        )
        .map_err(SandboxError::PodBuildError)?;

        // Build servers map for service port exposure
        let server_ports: BTreeMap<String, u16> =
            pod_builder::build_servers_map(plan).into_iter().collect();

        // 5. Create Deployment (wraps pod spec) + Service
        let _deployment_name =
            deployment::create_sandbox_deployment(&self.k8s, task_id, pod).await?;

        let service_name = service::create_sandbox_service(
            &self.k8s,
            task_id,
            &server_ports,
            self.config.sandbox.grpc_port,
        )
        .await?;

        // 5b. Create Traefik IngressRoutes (if proxy_domain is configured)
        if self.config.proxy.domain.is_some() {
            if let Err(e) =
                ingress::create_task_ingress_routes(&self.k8s, task_id, &server_ports, &self.config)
                    .await
            {
                warn!(task_id = %task_id, error = %e, "Failed to create IngressRoutes");
            }
        }

        info!(
            task_id = %task_id,
            service_name = %service_name,
            server_count = server_ports.len(),
            "Sandbox provisioned, returning immediately (pod starting in background)"
        );

        // Publish initializing event (pod_watcher handles the rest)
        self.publish_initializing(task_id).await;

        Ok(ProvisionResponse {
            status: SandboxStatus::Initializing,
        })
        }
        .await;

        crate::sandbox::metrics::record_provision(
            backend,
            if result.is_ok() { "ok" } else { "error" },
            started.elapsed(),
        );
        result
    }

    async fn sandbox_status(
        &self,
        task_id: &str,
        server_configs: &[ServerConfig],
    ) -> Result<SandboxStatusFullResponse, SandboxError> {
        Ok(self
            .status_resolver
            .resolve_sandbox_status(task_id, server_configs)
            .await)
    }

    async fn stop(&self, task_id: &str) -> Result<(), SandboxError> {
        info!(task_id = %task_id, "Stopping sandbox (scaling Deployment to 0)");

        // Scale Deployment to 0 — keeps Service and IngressRoutes alive
        match deployment::find_deployment(&self.k8s, task_id).await {
            Ok(Some(_)) => {
                deployment::scale_deployment(&self.k8s, task_id, 0)
                    .await
                    .map_err(SandboxError::KubeError)?;
            }
            Ok(None) => {
                info!(task_id = %task_id, "No deployment found, nothing to stop");
            }
            Err(e) => {
                return Err(SandboxError::KubeError(e));
            }
        }

        if let Some(ref publisher) = self.event_publisher {
            publisher
                .publish_sandbox_event(task_id, moatless_events::SandboxStatus::Stopped, "Stopped")
                .await;
        }

        Ok(())
    }

    async fn restart(&self, task_id: &str) -> Result<(), SandboxError> {
        info!(task_id = %task_id, "Restarting sandbox (scaling to 0, then back to 1 after termination)");

        // Time the restart so the `ebs` path (which pays a pod-drain wait to
        // release the RWO PVC before the new pod can attach) can be compared to
        // the node-local backends that don't. The backend is read from the
        // running Deployment's env rather than a plan (restart has no plan).
        let backend = self.deployment_storage_backend(task_id).await;
        let started = std::time::Instant::now();

        // Scale to 0, wait for the pod to terminate (releasing the RWO PVC),
        // then scale back to 1. This also covers the "was stopped" case since
        // restart_deployment always ends at 1 replica.
        let outcome = deployment::restart_deployment(&self.k8s, task_id)
            .await
            .map_err(SandboxError::KubeError);

        crate::sandbox::metrics::record_restart(backend, started.elapsed());
        outcome?;

        // Publish initializing event (pod_watcher handles readiness)
        self.publish_initializing(task_id).await;

        Ok(())
    }

    async fn redeploy(
        &self,
        task_id: &str,
        request: &ProvisionRequest,
    ) -> Result<ProvisionResponse, SandboxError> {
        info!(task_id = %task_id, "Redeploying sandbox (deleting Deployment/Service/IngressRoutes, preserving PVC)");

        // 1. Delete Deployment (cascades to pods + ReplicaSets)
        if let Err(e) = deployment::delete_deployment(&self.k8s, task_id).await {
            warn!(task_id = %task_id, error = %e, "Failed to delete Deployment during redeploy");
        }
        // 1b. Wait for the old pod to fully terminate before reprovisioning, so
        //     the new pod doesn't race the ReadWriteOnce PVC still attached to
        //     the terminating pod's node ("Multi-Attach" stall).
        deployment::wait_for_pods_deleted(&self.k8s, task_id).await;
        // 2. Delete Service + IngressRoutes (will recreate with new config)
        if let Err(e) = service::delete_sandbox_service(&self.k8s, task_id).await {
            warn!(task_id = %task_id, error = %e, "Failed to delete Service during redeploy");
        }
        if let Err(e) = ingress::delete_task_ingress_routes(&self.k8s, task_id).await {
            warn!(task_id = %task_id, error = %e, "Failed to delete IngressRoutes during redeploy");
        }
        // 3. PVC is intentionally NOT deleted — workspace data is preserved

        // 4. Reprovision with fresh config (provision handles PVC reuse)
        self.provision(task_id, request).await
    }

    async fn cleanup(&self, task_id: &str) -> Result<(), SandboxError> {
        info!(task_id = %task_id, "Cleaning up sandbox (deleting all resources)");

        // Delete Deployment (cascades to pods and ReplicaSets)
        if let Err(e) = deployment::delete_deployment(&self.k8s, task_id).await {
            warn!(task_id = %task_id, error = %e, "Failed to delete Deployment");
        }
        if let Err(e) = service::delete_sandbox_service(&self.k8s, task_id).await {
            warn!(task_id = %task_id, error = %e, "Failed to delete service");
        }
        if let Err(e) = ingress::delete_task_ingress_routes(&self.k8s, task_id).await {
            warn!(task_id = %task_id, error = %e, "Failed to delete IngressRoutes");
        }
        if let Err(e) = pvc::delete_task_pvc(&self.k8s, task_id).await {
            warn!(task_id = %task_id, error = %e, "Failed to delete PVC");
        }

        // Delete this task's transcript archive from S3 (`<prefix>/<task_id>/...`).
        // The per-task PVC delete above doesn't cover it — transcripts live in the
        // uploads bucket, synced there by the sandbox via backend-proxied gRPC.
        // Best-effort: a leftover object is harmless and the bucket lifecycle rule
        // eventually expires it. No-op when S3 isn't configured.
        self.delete_task_transcripts(task_id).await;

        // Delete this task's `s3`-backend workspace archive too (`<prefix>/<task_id>/`).
        // No-op for `ebs`/`ephemeral` tasks (nothing under the prefix).
        self.delete_task_workspace(task_id).await;

        if let Some(ref publisher) = self.event_publisher {
            publisher
                .publish_sandbox_event(
                    task_id,
                    moatless_events::SandboxStatus::Removed,
                    "Resources removed",
                )
                .await;
        }

        Ok(())
    }

    async fn container_statuses(
        &self,
        task_id: &str,
    ) -> Result<Vec<ContainerObservedStatus>, SandboxError> {
        Ok(pod_reader::fetch_server_container_statuses(&self.k8s, task_id).await)
    }

    async fn server_logs(
        &self,
        task_id: &str,
        server_name: &str,
        previous: bool,
    ) -> Result<BoxStream<'static, Result<Event, SandboxError>>, SandboxError> {
        let container_name = if server_name == "sandbox" {
            "sandbox".to_string()
        } else {
            format!("{server_name}-server")
        };

        match pod_reader::stream_container_logs_k8s(
            &self.k8s,
            task_id,
            &container_name,
            500,
            previous,
        )
        .await
        {
            Ok(stream) => Ok(stream),
            Err(pod_reader::LogStreamError::NoPod(msg)) => Err(SandboxError::NotReady(msg)),
            Err(pod_reader::LogStreamError::Other(msg)) => Err(SandboxError::Internal(msg)),
        }
    }

    async fn sandbox_client(&self, task_id: &str) -> Result<SandboxClient, SandboxError> {
        self.require_sandbox_ready(task_id).await?;
        let service_name = generate_service_name(task_id);
        SandboxClient::for_service(
            &service_name,
            self.k8s.namespace(),
            self.config.sandbox.grpc_port,
        )
        .map_err(|e| SandboxError::Internal(format!("Failed to create gRPC client: {e}")))
    }

    async fn bulk_environment_status(
        &self,
        task_ids: &[String],
    ) -> HashMap<String, SandboxStatus> {
        if task_ids.is_empty() {
            return HashMap::new();
        }
        let wanted: std::collections::HashSet<&str> =
            task_ids.iter().map(String::as_str).collect();

        // One cluster-wide LIST of managed Deployments; the Deployment status
        // (desired vs ready replicas) tells us stopped/starting/ready without
        // touching individual pods or the sandbox gRPC endpoint.
        let deployments: Api<Deployment> =
            Api::namespaced(self.k8s.client.clone(), self.k8s.namespace());
        let lp = ListParams::default().labels("app.kubernetes.io/managed-by=moatless");
        let list = match deployments.list(&lp).await {
            Ok(list) => list,
            Err(e) => {
                warn!(error = %e, "Failed to list deployments for bulk status");
                return HashMap::new();
            }
        };

        let mut out = HashMap::new();
        for dep in list.items {
            let task_id = dep
                .metadata
                .labels
                .as_ref()
                .and_then(|labels| labels.get("moatless.io/task-id"))
                .cloned();
            let Some(task_id) = task_id else { continue };
            if !wanted.contains(task_id.as_str()) {
                continue;
            }

            let desired = dep.spec.as_ref().and_then(|s| s.replicas).unwrap_or(0);
            let ready = dep
                .status
                .as_ref()
                .and_then(|s| s.ready_replicas)
                .unwrap_or(0);
            let status = if desired == 0 {
                SandboxStatus::Stopped
            } else if ready > 0 {
                SandboxStatus::Ready
            } else {
                SandboxStatus::Initializing
            };
            out.insert(task_id, status);
        }
        out
    }
}

// =============================================================================
// Private helper methods
// =============================================================================

/// Inspect a running Deployment's sandbox container for the
/// `WORKSPACE_SYNC_ENABLED=true` env that marks the `s3` storage backend.
fn deployment_has_workspace_sync(dep: &Deployment) -> bool {
    dep.spec
        .as_ref()
        .and_then(|s| s.template.spec.as_ref())
        .is_some_and(|pod| {
            pod.containers.iter().any(|c| {
                c.name == "sandbox"
                    && c.env.as_ref().is_some_and(|env| {
                        env.iter().any(|e| {
                            e.name == "WORKSPACE_SYNC_ENABLED"
                                && e.value.as_deref() == Some("true")
                        })
                    })
            })
        })
}

/// Result of checking for an existing pod.
enum ExistingPodResult {
    /// Running and ready — can be reused immediately.
    Ready,
    /// Exists but not yet ready (pending, starting, scaling up).
    Initializing,
    /// Deployment exists but is scaled to 0. Carries the image currently baked
    /// into the `sandbox` container so the start path can decide whether to
    /// scale up (tag unchanged) or rebuild (tag changed).
    Stopped { current_image: Option<String> },
    /// No deployment/pod found.
    NotFound,
}

impl K8sSandboxProvider {
    /// Fast readiness gate — checks deployment replicas before attempting gRPC.
    async fn require_sandbox_ready(&self, task_id: &str) -> Result<(), SandboxError> {
        match deployment::get_deployment_replicas(&self.k8s, task_id).await {
            Ok((0, _)) => Err(SandboxError::NotReady("Sandbox is stopped".into())),
            Ok(_) => Ok(()),
            Err(_) => {
                // Deployment might not exist at all
                match deployment::find_deployment(&self.k8s, task_id).await {
                    Ok(None) => Err(SandboxError::NotReady("Sandbox not provisioned".into())),
                    Ok(Some(_)) => Ok(()), // exists but replicas check failed — try gRPC anyway
                    Err(e) => Err(SandboxError::KubeError(e)),
                }
            }
        }
    }

    /// Publish an "initializing" event (fire-and-forget).
    ///
    /// The pod_watcher handles all subsequent readiness tracking and server
    /// container monitoring. No background readiness checker is spawned.
    async fn publish_initializing(&self, task_id: &str) {
        if let Some(ref publisher) = self.event_publisher {
            publisher
                .publish_sandbox_event(
                    task_id,
                    moatless_events::SandboxStatus::Initializing,
                    "Starting",
                )
                .await;
        }
    }

    /// Generate the user auth JWT a sandbox pod uses to call back to the backend.
    fn generate_user_auth_token(&self, user_id: &str) -> Result<String, SandboxError> {
        let task_token_secret = self
            .config
            .sandbox
            .task_token_secret
            .as_ref()
            .ok_or_else(|| SandboxError::ConfigMissing("TASK_TOKEN_SECRET".to_string()))?;

        jwt::generate_user_auth_token(user_id, user_id, task_token_secret)
            .map_err(|e| SandboxError::JwtError(e.to_string()))
    }

    /// Resolve the storage backend of a *running* task from cluster state
    /// (used where there's no `ProvisionPlan`, e.g. restart). A PVC means `ebs`;
    /// otherwise the `WORKSPACE_SYNC_ENABLED` env on the sandbox container
    /// distinguishes `s3` from plain `ephemeral`.
    async fn deployment_storage_backend(&self, task_id: &str) -> StorageBackend {
        if pvc::find_task_pvc(&self.k8s, task_id).await.is_some() {
            return StorageBackend::Ebs;
        }
        match deployment::find_deployment(&self.k8s, task_id).await {
            Ok(Some(dep)) if deployment_has_workspace_sync(&dep) => StorageBackend::S3,
            _ => StorageBackend::Ephemeral,
        }
    }

    /// Create or find existing PVC. A per-repo `resources.storage` override
    /// (if set) sizes a freshly created PVC; existing PVCs are reused as-is.
    ///
    /// Returns an empty string (→ no PVC) for the non-EBS backends (`ephemeral`
    /// and `s3`, via `resources.storage_mode`): `build_pod` then mounts a
    /// node-local emptyDir at `/opt/moatless` instead. The check runs *before*
    /// `find_task_pvc` so a repo flipped off `ebs` stops reusing any stale PVC.
    async fn create_or_find_pvc(
        &self,
        task_id: &str,
        plan: &ProvisionPlan,
    ) -> Result<String, String> {
        if !StorageBackend::from_plan(plan, &self.config.sandbox.default_storage_mode).uses_pvc() {
            info!(task_id, "Non-EBS storage backend — skipping PVC creation");
            return Ok(String::new());
        }

        if let Some(name) = pvc::find_task_pvc(&self.k8s, task_id).await {
            info!(task_id, pvc_name = %name, "Found existing PVC, reusing");
            return Ok(name);
        }

        let storage_override = plan
            .repo_config
            .resources
            .as_ref()
            .and_then(|r| r.storage.as_deref());

        pvc::create_task_pvc(&self.k8s, &self.config, task_id, storage_override)
            .await
            .map_err(|e| e.to_string())
    }

    /// Best-effort delete of a task's transcript archive from S3.
    ///
    /// Removes every object under `<transcript_prefix>/<task_id>/` in the uploads
    /// bucket. No-op when S3 or the bucket isn't configured. Errors are logged,
    /// never propagated — cleanup must not fail on a stray transcript object.
    async fn delete_task_transcripts(&self, task_id: &str) {
        let (Some(s3), Some(bucket)) = (self.s3.as_ref(), self.config.s3.bucket_name.as_deref())
        else {
            return;
        };
        let prefix = format!(
            "{}/{}/",
            self.config.s3.transcript_prefix.trim_end_matches('/'),
            task_id
        );
        match crate::file_storage::s3::delete_objects_by_prefix(s3, bucket, &prefix).await {
            Ok(0) => {}
            Ok(n) => info!(task_id, deleted = n, prefix = %prefix, "Deleted task transcripts from S3"),
            Err(e) => {
                warn!(task_id, error = %e, prefix = %prefix, "Failed to delete task transcripts from S3");
            }
        }
    }

    /// Best-effort delete of a task's `s3`-backend workspace archive from S3.
    ///
    /// Removes every object under `<workspace_prefix>/<task_id>/` in the uploads
    /// bucket — the counterpart to the sandbox's presigned uploads. No-op when
    /// S3 or the bucket isn't configured, or for tasks that never used the `s3`
    /// backend (the prefix is simply empty). Errors are logged, never propagated.
    async fn delete_task_workspace(&self, task_id: &str) {
        let (Some(s3), Some(bucket)) = (self.s3.as_ref(), self.config.s3.bucket_name.as_deref())
        else {
            return;
        };
        let prefix = format!(
            "{}/{}/",
            self.config.s3.workspace_prefix.trim_end_matches('/'),
            task_id
        );
        match crate::file_storage::s3::delete_objects_by_prefix(s3, bucket, &prefix).await {
            Ok(0) => {}
            Ok(n) => info!(task_id, deleted = n, prefix = %prefix, "Deleted task workspace from S3"),
            Err(e) => {
                warn!(task_id, error = %e, prefix = %prefix, "Failed to delete task workspace from S3");
            }
        }
    }

    /// Check for existing pod — returns richer result for idempotent provisioning.
    ///
    /// - `Ready`: Running pod with sandbox container ready
    /// - `Initializing`: Deployment exists but pod not ready yet
    /// - `Stopped`: Deployment scaled to 0 (carries current sandbox image)
    /// - `NotFound`: No Deployment exists
    async fn check_existing_pod(&self, task_id: &str) -> ExistingPodResult {
        // Check if a Deployment already exists
        let dep = match deployment::find_deployment(&self.k8s, task_id).await {
            Ok(Some(d)) => d,
            Ok(None) => return ExistingPodResult::NotFound,
            Err(e) => {
                warn!(task_id = %task_id, error = %e, "Error checking existing deployment");
                return ExistingPodResult::NotFound;
            }
        };

        // If Deployment is scaled to 0, report it as Stopped along with the
        // image currently baked into the pod template. The caller decides
        // whether to scale up (tag unchanged) or rebuild (tag changed).
        if let Ok((desired, _)) = deployment::get_deployment_replicas(&self.k8s, task_id).await {
            if desired == 0 {
                return ExistingPodResult::Stopped {
                    current_image: deployment::deployment_sandbox_image(&dep),
                };
            }
        }

        // Check actual pod status
        let pods_api: Api<Pod> = Api::namespaced(self.k8s.client.clone(), self.k8s.namespace());
        let lp = ListParams::default().labels(&format!("moatless.io/task-id={task_id}"));

        let list = match pods_api.list(&lp).await {
            Ok(list) => list,
            Err(e) => {
                warn!(task_id = %task_id, error = %e, "Error listing pods");
                return ExistingPodResult::Initializing; // Deployment exists, assume initializing
            }
        };

        if list.items.is_empty() {
            return ExistingPodResult::Initializing; // Deployment exists but no pods yet
        }

        let pod = &list.items[0];
        let is_terminating = pod.metadata.deletion_timestamp.is_some();
        let raw_phase = pod
            .status
            .as_ref()
            .and_then(|s| s.phase.as_deref())
            .unwrap_or("Unknown");

        if is_terminating {
            return ExistingPodResult::Initializing;
        }

        let phase = PodPhase::from(raw_phase);

        match phase {
            PodPhase::Running if is_sandbox_container_ready(pod) => ExistingPodResult::Ready,
            _ => ExistingPodResult::Initializing,
        }
    }
}
