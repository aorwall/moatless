//! Kubernetes implementation of the SandboxProvider trait.

use async_trait::async_trait;
use axum::response::sse::Event;
use futures::stream::BoxStream;
use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::core::v1::Pod;
use kube::api::{Api, ListParams};
use moatless_events::SandboxEventPayload;
use std::collections::BTreeMap;
use std::sync::Arc;
use tracing::{info, warn};

use super::client::K8sClient;
use super::deployment;
use super::ingress;
use super::naming::generate_service_name;
use super::owner_ref;
use super::pod_builder;
use super::pod_reader;
use super::pod_status::{is_sandbox_container_ready, pick_best_pod};
use super::pvc;
use super::service;
use super::status_resolver::StatusResolver;
use super::storage::StorageBackend;
use crate::config::Config;
use crate::events::EventPublisher;
use crate::sandbox::client::SandboxClient;
use crate::sandbox::error::SandboxError;
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
        let existing = self.check_existing_pod(task_id).await;
        if !matches!(existing, ExistingPodResult::NotFound) {
            // A live Deployment can have lost its Service/IngressRoutes: they
            // are owner-referenced to the Deployment, and the old redeploy path
            // deleted + recreated it, cascading their deletion without ever
            // recreating them. Without the Service the backend can't reach the
            // sandbox over gRPC, so heal before taking any fast path.
            self.heal_network_resources(task_id, plan).await;
        }
        match existing {
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
                    // A timeout here surfaces as an environment error rather than
                    // scheduling a new pod straight into the stall.
                    deployment::wait_for_pods_deleted(&self.k8s, task_id).await?;
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
                    deployment::redeploy_with_plan(&self.k8s, &self.config, task_id, request)
                        .await?;
                }
                self.publish_initializing(task_id).await;
                return Ok(ProvisionResponse {
                    status: SandboxStatus::Initializing,
                });
            }
            ExistingPodResult::NotFound => {} // Continue provisioning
        }

        // 2. Mint the pod's auth tokens (short-lived bootstrap + optional legacy
        //    user token during rollout) from user_id directly (no DB lookup). The
        //    long-lived user token is delivered over the credential channel.
        let minted_tokens = pod_builder::mint_sandbox_tokens(&self.config, &request.user_id)?;

        // 3. Create PVC. A 409 is handled inside create_task_pvc (reuse the
        //    existing PVC), so an Err here is a genuine failure — degrading to
        //    ephemeral storage silently drops workspace persistence, so surface
        //    a visible warning event instead of only logging.
        let pvc_name = match self.create_or_find_pvc(task_id, plan).await {
            Ok(name) if !name.is_empty() => Some(name),
            Ok(_) => None,
            Err(e) => {
                warn!(task_id = %task_id, error = %e, "Failed to create PVC, proceeding without persistence");
                if let Some(ref publisher) = self.event_publisher {
                    publisher
                        .publish_sandbox_event_with_payload(
                            task_id,
                            SandboxStatus::Initializing,
                            &SandboxEventPayload {
                                message:
                                    "Persistent storage unavailable — running with ephemeral \
                                     storage; workspace changes will not survive a restart."
                                        .to_string(),
                                lifecycle_event: Some("storage_degraded".to_string()),
                                error: Some(e),
                                ..Default::default()
                            },
                        )
                        .await;
                }
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
            Some(pod_builder::SandboxAuthTokens::from_minted(&minted_tokens)),
        )
        .map_err(SandboxError::PodBuildError)?;

        // Build servers map for service port exposure
        let server_ports: BTreeMap<String, u16> =
            pod_builder::build_servers_map(plan).into_iter().collect();

        // 5. Create Deployment (wraps pod spec) first so its UID can own the
        //    Service and IngressRoutes — K8s garbage collection then cascades
        //    their deletion when the Deployment is deleted, so a transient
        //    failure mid-cleanup can't orphan them.
        let deployment = deployment::create_sandbox_deployment(&self.k8s, task_id, pod).await?;
        let owner = deployment
            .uid
            .as_deref()
            .map(|uid| owner_ref::deployment_owner_reference(&deployment.name, uid));
        if owner.is_none() {
            warn!(task_id = %task_id, "Deployment UID unavailable; Service/IngressRoutes will not be owner-referenced");
        }

        let service_name = service::create_sandbox_service(
            &self.k8s,
            task_id,
            &server_ports,
            self.config.sandbox.grpc_port,
            owner.as_ref(),
        )
        .await?;

        // 5b. Create Traefik IngressRoutes (if proxy_domain is configured)
        if self.config.proxy.domain.is_some() {
            if let Err(e) = ingress::create_task_ingress_routes(
                &self.k8s,
                task_id,
                &server_ports,
                &self.config,
                owner.as_ref(),
            )
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
        task_status: crate::task::models::TaskStatus,
        desired_state: crate::task::models::SandboxDesiredState,
    ) -> Result<SandboxStatusFullResponse, SandboxError> {
        Ok(self
            .status_resolver
            .resolve_sandbox_status(task_id, server_configs, task_status, desired_state)
            .await)
    }

    async fn live_status(
        &self,
        task_id: &str,
        task_status: crate::task::models::TaskStatus,
        desired_state: crate::task::models::SandboxDesiredState,
    ) -> Result<crate::sandbox::live_status::LiveStatus, SandboxError> {
        Ok(self
            .status_resolver
            .resolve_live_status(task_id, task_status, desired_state)
            .await)
    }

    fn expected_sandbox_image(&self, repo_docker_image: Option<&str>) -> String {
        repo_docker_image
            .filter(|s| !s.is_empty())
            .map_or_else(|| self.config.sandbox.image.clone(), str::to_string)
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
        let outcome = deployment::restart_deployment(&self.k8s, task_id).await;

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

        // 1. Delete Deployment (cascades to pods + ReplicaSets). Fail hard on
        //    error: if the delete fails, the reprovision below hits 409 and
        //    "reuses" the existing Deployment, so the old pod spec survives and
        //    the redeploy is silently defeated. `delete_deployment` already
        //    treats 404 as success, so this only fails on a genuine API error.
        deployment::delete_deployment(&self.k8s, task_id)
            .await
            .map_err(SandboxError::KubeError)?;
        // 1b. Wait for the old pod to fully terminate before reprovisioning, so
        //     the new pod doesn't race the ReadWriteOnce PVC still attached to
        //     the terminating pod's node ("Multi-Attach" stall).
        deployment::wait_for_pods_deleted(&self.k8s, task_id).await?;
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

    async fn cleanup(&self, task_id: &str, retain_storage: bool) -> Result<(), SandboxError> {
        info!(task_id = %task_id, retain_storage, "Cleaning up sandbox (deleting all resources)");

        // Delete the Deployment first. Its ownerReferences make the Service and
        // IngressRoutes children, so K8s garbage collection cascades their
        // deletion once the Deployment is gone. The explicit Service/IngressRoute
        // deletes below are best-effort cleanup for resources created before the
        // ownership chain existed — their failures don't fail cleanup.
        //
        // Every load-bearing step feeds `first_err`: a failure returns an error
        // so the caller retries and the `Removed` completion is withheld. Removal
        // is strict — it reports success only once every resource is observed
        // absent (pods drained, PVC gone) and every owed S3 archive is deleted.
        let mut first_err: Option<SandboxError> = None;

        if let Err(e) = deployment::delete_deployment(&self.k8s, task_id).await {
            warn!(task_id = %task_id, error = %e, "Failed to delete Deployment");
            first_err.get_or_insert(SandboxError::KubeError(e));
        }
        // Best-effort direct deletes (pre-ownership resources). GC covers the
        // rest via the Deployment's ownerReferences.
        if let Err(e) = service::delete_sandbox_service(&self.k8s, task_id).await {
            warn!(task_id = %task_id, error = %e, "Failed to delete service (best-effort; GC cascades)");
        }
        if let Err(e) = ingress::delete_task_ingress_routes(&self.k8s, task_id).await {
            warn!(task_id = %task_id, error = %e, "Failed to delete IngressRoutes (best-effort; GC cascades)");
        }

        // Await pod termination before deleting the PVC: a still-mounted PVC
        // won't delete. Skip the wait if the Deployment delete already failed —
        // the caller will retry the whole cleanup.
        if first_err.is_none() {
            if let Err(e) = deployment::wait_for_pods_deleted(&self.k8s, task_id).await {
                warn!(task_id = %task_id, error = %e, "Pods did not terminate before PVC delete");
                first_err.get_or_insert(e);
            }
        }
        if let Err(e) = pvc::delete_task_pvc(&self.k8s, task_id).await {
            warn!(task_id = %task_id, error = %e, "Failed to delete PVC");
            first_err.get_or_insert(SandboxError::KubeError(e));
        }
        // Strict absence: a delete request returns before reclaim completes, so
        // await the PVC actually vanishing before reporting success.
        if first_err.is_none() {
            if let Err(e) = pvc::wait_for_pvc_deleted(&self.k8s, task_id).await {
                warn!(task_id = %task_id, error = %e, "PVC did not vanish after delete");
                first_err.get_or_insert(e);
            }
        }

        if retain_storage {
            // Soft close: keep the S3 archives so the task can be revived. The
            // sandbox's restore-on-start (workspace) and download-on-start
            // (transcript) are keyed purely by task_id, so a later reprovision
            // picks them back up transparently. K8s resources (incl. PVC) are
            // still gone — only the S3-backed state survives.
            info!(task_id = %task_id, "Retaining S3 archives on close (revivable)");
        } else {
            // Delete this task's transcript and `s3`-backend workspace archives
            // (`<prefix>/<task_id>/...`). These live in the uploads bucket, not
            // the PVC, so the per-task PVC delete above doesn't cover them. The
            // deletes are load-bearing: a "successful" remove must not leave an
            // archive behind. No-op when S3 isn't configured or the prefix is
            // empty (`ebs`/`ephemeral` tasks).
            if let Err(e) = self.delete_task_transcripts(task_id).await {
                warn!(task_id = %task_id, error = %e, "Failed to delete task transcripts from S3");
                first_err.get_or_insert(e);
            }
            if let Err(e) = self.delete_task_sandbox(task_id).await {
                warn!(task_id = %task_id, error = %e, "Failed to delete task workspace from S3");
                first_err.get_or_insert(e);
            }
        }

        // Surface any load-bearing failure to the caller: a propagated error
        // withholds lifecycle's `Removed` completion (its sole publisher), so
        // resources or archives that linger get another reconciler pass.
        if let Some(e) = first_err {
            return Err(e);
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
            Err(
                pod_reader::LogStreamError::NoPod(msg)
                | pod_reader::LogStreamError::NotStarted(msg),
            ) => Err(SandboxError::NotReady(msg)),
            Err(pod_reader::LogStreamError::NoPreviousLogs(msg)) => {
                Err(SandboxError::NotFound(msg))
            }
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
                            e.name == "WORKSPACE_SYNC_ENABLED" && e.value.as_deref() == Some("true")
                        })
                    })
            })
        })
}

/// Inspect a Deployment's pod template for a `workspace-data` hostPath volume
/// that marks the best-effort node-local storage backend.
fn deployment_has_sandbox_host_path(dep: &Deployment) -> bool {
    dep.spec
        .as_ref()
        .and_then(|s| s.template.spec.as_ref())
        .and_then(|pod| pod.volumes.as_ref())
        .is_some_and(|volumes| {
            volumes
                .iter()
                .any(|v| v.name == "workspace-data" && v.host_path.is_some())
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

    /// Recreate the sandbox Service (and IngressRoutes) if they went missing
    /// while the Deployment lives on — e.g. cascade-deleted through their
    /// Deployment `ownerReference` by the old delete+recreate redeploy path.
    /// Best-effort: failures are logged, not returned, so provisioning still
    /// proceeds (the notify path surfaces unreachability on its own).
    async fn heal_network_resources(&self, task_id: &str, plan: &ProvisionPlan) {
        match service::sandbox_service_exists(&self.k8s, task_id).await {
            Ok(true) => return,
            Ok(false) => {}
            Err(e) => {
                warn!(task_id = %task_id, error = %e, "Failed to check sandbox Service, skipping network heal");
                return;
            }
        }
        warn!(task_id = %task_id, "Sandbox Service missing for existing Deployment, recreating");

        let server_ports: BTreeMap<String, u16> =
            pod_builder::build_servers_map(plan).into_iter().collect();
        let owner = match deployment::find_deployment(&self.k8s, task_id).await {
            Ok(Some(dep)) => dep
                .metadata
                .name
                .zip(dep.metadata.uid)
                .map(|(name, uid)| owner_ref::deployment_owner_reference(&name, &uid)),
            _ => None,
        };

        if let Err(e) = service::create_sandbox_service(
            &self.k8s,
            task_id,
            &server_ports,
            self.config.sandbox.grpc_port,
            owner.as_ref(),
        )
        .await
        {
            warn!(task_id = %task_id, error = %e, "Failed to recreate sandbox Service");
            return;
        }

        if self.config.proxy.domain.is_some() {
            if let Err(e) = ingress::create_task_ingress_routes(
                &self.k8s,
                task_id,
                &server_ports,
                &self.config,
                owner.as_ref(),
            )
            .await
            {
                warn!(task_id = %task_id, error = %e, "Failed to recreate IngressRoutes");
            }
        }
        info!(task_id = %task_id, "Recreated missing sandbox Service");
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

    /// Resolve the storage backend of a *running* task from cluster state
    /// (used where there's no `ProvisionPlan`, e.g. restart). A PVC means `ebs`;
    /// otherwise `workspace-data` as a hostPath means `node`, and the
    /// `WORKSPACE_SYNC_ENABLED` env on the sandbox container distinguishes `s3`
    /// from plain `ephemeral`.
    async fn deployment_storage_backend(&self, task_id: &str) -> StorageBackend {
        if pvc::find_task_pvc(&self.k8s, task_id).await.is_some() {
            return StorageBackend::Ebs;
        }
        match deployment::find_deployment(&self.k8s, task_id).await {
            Ok(Some(dep)) if deployment_has_sandbox_host_path(&dep) => StorageBackend::Node,
            Ok(Some(dep)) if deployment_has_workspace_sync(&dep) => StorageBackend::S3,
            _ => StorageBackend::Ephemeral,
        }
    }

    /// Create or find existing PVC. A per-repo `resources.storage` override
    /// (if set) sizes a freshly created PVC; existing PVCs are reused as-is.
    ///
    /// Returns an empty string (→ no PVC) for non-EBS backends (`ephemeral`,
    /// `s3`, and `node`, via `resources.storage_mode`): `build_pod` then mounts
    /// node-local storage at `/opt/moatless` instead. The check runs *before*
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
    /// bucket. No-op when S3 or the bucket isn't configured. On a hard delete the
    /// removal is load-bearing: a failure propagates so `cleanup` can withhold
    /// the `Removed` completion and leave the archive obligation for a retry.
    async fn delete_task_transcripts(&self, task_id: &str) -> Result<(), SandboxError> {
        let (Some(s3), Some(bucket)) = (self.s3.as_ref(), self.config.s3.bucket_name.as_deref())
        else {
            return Ok(());
        };
        let prefix = format!(
            "{}/{}/",
            self.config.s3.transcript_prefix.trim_end_matches('/'),
            task_id
        );
        let deleted = crate::file_storage::s3::delete_objects_by_prefix(s3, bucket, &prefix)
            .await
            .map_err(|e| {
                SandboxError::Internal(format!("delete transcripts for {task_id}: {e}"))
            })?;
        if deleted > 0 {
            info!(task_id, deleted, prefix = %prefix, "Deleted task transcripts from S3");
        }
        Ok(())
    }

    /// Delete of a task's `s3`-backend workspace archive from S3.
    ///
    /// Removes every object under `<workspace_prefix>/<task_id>/` in the uploads
    /// bucket — the counterpart to the sandbox's presigned uploads. No-op when
    /// S3 or the bucket isn't configured, or for tasks that never used the `s3`
    /// backend (the prefix is simply empty). Load-bearing like
    /// [`Self::delete_task_transcripts`]: a failure propagates.
    async fn delete_task_sandbox(&self, task_id: &str) -> Result<(), SandboxError> {
        let (Some(s3), Some(bucket)) = (self.s3.as_ref(), self.config.s3.bucket_name.as_deref())
        else {
            return Ok(());
        };
        let prefix = format!(
            "{}/{}/",
            self.config.s3.workspace_prefix.trim_end_matches('/'),
            task_id
        );
        let deleted = crate::file_storage::s3::delete_objects_by_prefix(s3, bucket, &prefix)
            .await
            .map_err(|e| SandboxError::Internal(format!("delete workspace for {task_id}: {e}")))?;
        if deleted > 0 {
            info!(task_id, deleted, prefix = %prefix, "Deleted task workspace from S3");
        }
        Ok(())
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

        // During a rolling update (old terminating pod + new starting pod both
        // present) `list.items[0]` is arbitrary — pick the best pod so we report
        // the replacement's status, not the dying pod's. Mirrors status_resolver.
        let Some(pod) = pick_best_pod(&list.items) else {
            return ExistingPodResult::Initializing; // Deployment exists but no pods yet
        };
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
