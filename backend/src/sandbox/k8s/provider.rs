//! Kubernetes Sandbox runtime, observer, and connector adapter.

use async_trait::async_trait;
use futures::stream::BoxStream;
use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::core::v1::Pod;
use kube::api::{Api, ListParams};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use tracing::{info, warn};

use super::client::K8sClient;
use super::deployment;
use super::ingress;
use super::naming::generate_service_name;
use super::networkpolicy;
use super::node_archive;
use super::owner_ref;
use super::pod_builder;
use super::pod_reader;
use super::pod_status::{is_sandbox_container_ready, pick_best_pod};
use super::pvc;
use super::service;
use super::status_resolver::StatusResolver;
use super::storage::StorageBackend;
use crate::config::Config;
use crate::sandbox::access::{SandboxConnector, SandboxObserver};
use crate::sandbox::client::SandboxClient;
use crate::sandbox::error::SandboxError;
use crate::sandbox::lifecycle::outcome::{DegradationWarning, Diagnostics};
use crate::sandbox::lifecycle::runtime::{
    map_sandbox_error, RuntimeOperationContext, RuntimeOutcome, SandboxRuntime,
};
use crate::sandbox::lifecycle::{ArchiveRetention, ObservedStatus, ProvisionDeps, RuntimeEffect};
use crate::sandbox::models::*;
use crate::sandbox::schemas::*;

/// Kubernetes adapter for lifecycle mutation, observation, and connection.
pub struct K8sSandboxAdapter {
    pub config: Arc<Config>,
    pub k8s: K8sClient,
    pub status_resolver: StatusResolver,
    pub(crate) plan: Option<ProvisionDeps>,
    /// Optional S3 client used to delete a task's transcript archive on cleanup.
    /// The sandbox uploads transcripts via backend-proxied gRPC (no pod creds);
    /// the backend owns the credentials, so cleanup of `<prefix>/<task_id>/` lives
    /// here. `None` when S3 isn't configured — cleanup then skips the delete.
    pub s3: Option<Arc<aws_sdk_s3::Client>>,
}

async fn wait_absent<F>(
    resource: &str,
    wait: F,
) -> Result<(), crate::sandbox::lifecycle::RuntimeError>
where
    F: std::future::Future<Output = Result<(), SandboxError>>,
{
    wait.await.map_err(|error| {
        crate::sandbox::lifecycle::RuntimeError::NotAbsent(format!("{resource}: {error}"))
    })
}

impl K8sSandboxAdapter {
    fn require_plan(&self) -> Result<&ProvisionDeps, crate::sandbox::lifecycle::RuntimeError> {
        self.plan.as_ref().ok_or_else(|| {
            crate::sandbox::lifecycle::RuntimeError::Infrastructure(
                "sandbox plan repositories unavailable".to_string(),
            )
        })
    }

    async fn ensure_running_with_request(
        &self,
        task_id: &str,
        request: &ProvisionRequest,
        ctx: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, SandboxError> {
        // Time the whole provision so the `ebs` PVC path (attach) and the
        // node-local backends can be compared. The body runs in an inner async
        // block so every early return is captured at a single measurement point.
        let backend =
            StorageBackend::from_plan(&request.plan, &self.config.sandbox.default_storage_mode);
        let started = std::time::Instant::now();
        let result: Result<RuntimeOutcome, SandboxError> = async {
        let mut diagnostics = Diagnostics::default();
        info!(task_id = %task_id, user_id = %request.user_id, "Provisioning sandbox");

        let plan = &request.plan;

        // 1. Check for existing pod — never destroy, only reuse, wait, or rebuild on tag change
        let existing = self.check_existing_pod(task_id).await;
        if !matches!(existing, ExistingPodResult::NotFound) {
            // A live Deployment can have lost its Service/IngressRoutes. Without
            // the Service the backend can't reach the sandbox over gRPC, so heal
            // before taking any fast path.
            self.heal_network_resources(task_id, plan).await;
        }
        match existing {
            ExistingPodResult::Ready => {
                info!(task_id = %task_id, "Reusing existing healthy pod");
                return Ok(RuntimeOutcome {
                    observed: ObservedStatus::Ready,
                    effect: RuntimeEffect::NoOp,
                    diagnostics,
                });
            }
            ExistingPodResult::Initializing => {
                info!(task_id = %task_id, "Pod exists but not ready yet, returning Initializing");
                return Ok(RuntimeOutcome {
                    observed: ObservedStatus::Starting,
                    effect: RuntimeEffect::NoOp,
                    diagnostics,
                });
            }
            ExistingPodResult::Stopped { current_image } => {
                let existing_deployment = deployment::find_deployment(&self.k8s, task_id)
                    .await
                    .map_err(SandboxError::KubeError)?
                    .ok_or_else(|| {
                        SandboxError::Internal(
                            "stopped sandbox Deployment vanished before start".to_string(),
                        )
                    })?;
                let require_node =
                    node_archive::deployment_requires_retained_node(&existing_deployment);
                let uses_node_archives = self.config.sandbox.node_archive_enabled
                    && node_archive::deployment_uses_node_archives(&existing_deployment);
                let retained_node = if uses_node_archives {
                    node_archive::record_current_pod_nodes(&self.k8s, task_id)
                        .await
                        .map_err(SandboxError::Runtime)?;
                    node_archive::retained_node(&self.k8s, task_id, require_node)
                        .await
                        .map_err(SandboxError::Runtime)?
                } else {
                    None
                };
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
                    if uses_node_archives {
                        deployment::set_node_placement(
                            &self.k8s,
                            task_id,
                            retained_node.as_deref(),
                            require_node,
                        )
                        .await
                        .map_err(SandboxError::KubeError)?;
                    }
                    deployment::stamp_operation_generation(&self.k8s, task_id, ctx)
                        .await
                        .map_err(SandboxError::KubeError)?;
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
                    deployment::redeploy_with_plan(
                        &self.k8s,
                        &self.config,
                        task_id,
                        request,
                        ctx,
                        retained_node.as_deref(),
                        require_node,
                    )
                    .await?;
                }
                return Ok(RuntimeOutcome {
                    observed: ObservedStatus::Starting,
                    effect: RuntimeEffect::Applied,
                    diagnostics,
                });
            }
            ExistingPodResult::NotFound => {} // Continue provisioning
        }

        // 2. Mint the pod's auth tokens from user_id directly (no DB lookup). The
        //    long-lived user token is delivered over the credential channel.
        let minted_tokens =
            pod_builder::mint_sandbox_tokens(&self.config, task_id, &request.user_id)?;

        // 3. Create PVC. A 409 is handled inside create_task_pvc (reuse the
        //    existing PVC), so an Err here is a genuine failure — degrading to
        //    ephemeral storage silently drops workspace persistence, so surface
        //    a visible warning event instead of only logging.
        let pvc_name = match self.create_or_find_pvc(task_id, plan).await {
            Ok(name) if !name.is_empty() => Some(name),
            Ok(_) => None,
            Err(e) => {
                warn!(task_id = %task_id, error = %e, "Failed to create PVC, proceeding without persistence");
                diagnostics.degradations.push(DegradationWarning {
                    code: "storage_degraded".to_string(),
                    message: format!(
                        "Persistent storage unavailable; workspace changes will not survive a restart: {e}"
                    ),
                });
                None
            }
        };

        // 4. Build pod spec. Secrets flow via gRPC `GetCredentials` and are
        //    materialized to `/opt/moatless/data/server.env` by the sandbox init.
        let mut pod = pod_builder::build_pod(
            &self.config,
            task_id,
            plan,
            pvc_name.as_deref(),
            Some(pod_builder::SandboxAuthTokens::from_minted(&minted_tokens)),
        )
        .map_err(SandboxError::PodBuildError)?;
        if self.config.sandbox.node_archive_enabled
            && node_archive::pod_uses_node_archives(&pod)
        {
            node_archive::record_current_pod_nodes(&self.k8s, task_id)
                .await
                .map_err(SandboxError::Runtime)?;
            if let Some(node_name) = node_archive::retained_node(&self.k8s, task_id, false)
                .await
                .map_err(SandboxError::Runtime)?
            {
                let require_node = node_archive::pod_requires_retained_node(&pod);
                pod_builder::pin_pod_to_node(&mut pod, &node_name, require_node)?;
            }
        }
        pod_builder::stamp_operation_generation(&mut pod, ctx);

        // Build servers map for service port exposure, dropping any port that
        // collides with a reserved sandbox infrastructure port so it is never
        // exposed via the Service, an IngressRoute, or the preview NetworkPolicy.
        let server_ports: BTreeMap<String, u16> = pod_builder::admitted_server_ports(
            pod_builder::build_servers_map(plan).into_iter().collect(),
            &self.config,
            task_id,
        );

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

        // 5b. Create Traefik IngressRoutes + the preview NetworkPolicy admitting
        //     Traefik to those same ports (if proxy_domain is configured).
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
            let preview_ports: BTreeSet<u16> = server_ports.values().copied().collect();
            if let Err(e) = networkpolicy::create_task_preview_network_policy(
                &self.k8s,
                task_id,
                &preview_ports,
                &self.config,
                owner.as_ref(),
            )
            .await
            {
                warn!(task_id = %task_id, error = %e, "Failed to create preview NetworkPolicy");
            }
        }

        info!(
            task_id = %task_id,
            service_name = %service_name,
            server_count = server_ports.len(),
            "Sandbox provisioned, returning immediately (pod starting in background)"
        );

        Ok(RuntimeOutcome {
            observed: ObservedStatus::Starting,
            effect: RuntimeEffect::Applied,
            diagnostics,
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
}

#[async_trait]
impl SandboxRuntime for K8sSandboxAdapter {
    #[tracing::instrument(skip_all, fields(task_id = %task_id))]
    async fn ensure_running(
        &self,
        task_id: &str,
        ctx: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, crate::sandbox::lifecycle::RuntimeError> {
        let plan = self.require_plan()?;
        let request = plan.build_request(task_id).await?;
        Box::pin(self.ensure_running_with_request(task_id, &request, ctx))
            .await
            .map_err(map_sandbox_error)
    }

    #[tracing::instrument(skip_all, fields(task_id = %task_id))]
    async fn stop(
        &self,
        task_id: &str,
        _ctx: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, crate::sandbox::lifecycle::RuntimeError> {
        info!(task_id = %task_id, "Stopping sandbox (scaling Deployment to 0)");

        // Scale Deployment to 0 — keeps Service and IngressRoutes alive
        let effect = match deployment::find_deployment(&self.k8s, task_id).await {
            Ok(Some(deployment))
                if deployment.spec.as_ref().and_then(|spec| spec.replicas) == Some(0) =>
            {
                RuntimeEffect::NoOp
            }
            Ok(Some(existing)) => {
                if self.config.sandbox.node_archive_enabled
                    && node_archive::deployment_uses_node_archives(&existing)
                {
                    // Capture the scheduled location before scale-to-zero
                    // removes the only pod that can name the owning node.
                    node_archive::record_current_pod_nodes(&self.k8s, task_id).await?;
                }
                deployment::scale_deployment(&self.k8s, task_id, 0)
                    .await
                    .map_err(|error| map_sandbox_error(SandboxError::KubeError(error)))?;
                RuntimeEffect::Applied
            }
            Ok(None) => {
                info!(task_id = %task_id, "No deployment found, nothing to stop");
                RuntimeEffect::NoOp
            }
            Err(e) => {
                return Err(map_sandbox_error(SandboxError::KubeError(e)));
            }
        };

        Ok(RuntimeOutcome {
            observed: ObservedStatus::Stopped,
            effect,
            diagnostics: Diagnostics::default(),
        })
    }

    #[tracing::instrument(skip_all, fields(task_id = %task_id))]
    async fn restart(
        &self,
        task_id: &str,
        ctx: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, crate::sandbox::lifecycle::RuntimeError> {
        info!(task_id = %task_id, "Restarting sandbox (scaling to 0, then back to 1 after termination)");

        // Time the restart so the `ebs` path (which pays a pod-drain wait to
        // release the RWO PVC before the new pod can attach) can be compared to
        // the node-local backends that don't. The backend is read from the
        // running Deployment's env rather than a plan (restart has no plan).
        let backend = self.deployment_storage_backend(task_id).await;
        let started = std::time::Instant::now();
        let existing_deployment = deployment::find_deployment(&self.k8s, task_id)
            .await
            .map_err(|error| map_sandbox_error(SandboxError::KubeError(error)))?;
        if self.config.sandbox.node_archive_enabled
            && existing_deployment
                .as_ref()
                .is_some_and(node_archive::deployment_uses_node_archives)
        {
            node_archive::record_current_pod_nodes(&self.k8s, task_id).await?;
            let require_node = existing_deployment
                .as_ref()
                .is_some_and(node_archive::deployment_requires_retained_node);
            let retained_node =
                node_archive::retained_node(&self.k8s, task_id, require_node).await?;
            deployment::set_node_placement(
                &self.k8s,
                task_id,
                retained_node.as_deref(),
                require_node,
            )
            .await
            .map_err(|error| map_sandbox_error(SandboxError::KubeError(error)))?;
        }

        // Scale to 0, wait for the pod to terminate (releasing the RWO PVC),
        // then scale back to 1. This also covers the "was stopped" case since
        // restart_deployment always ends at 1 replica.
        let outcome = deployment::restart_deployment(&self.k8s, task_id, ctx).await;

        crate::sandbox::metrics::record_restart(backend, started.elapsed());
        outcome.map_err(map_sandbox_error)?;

        Ok(RuntimeOutcome::applied(ObservedStatus::Starting))
    }

    #[tracing::instrument(skip_all, fields(task_id = %task_id))]
    async fn redeploy(
        &self,
        task_id: &str,
        ctx: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, crate::sandbox::lifecycle::RuntimeError> {
        info!(task_id = %task_id, "Redeploying sandbox (deleting Deployment/Service/IngressRoutes, preserving PVC)");
        let plan = self.require_plan()?;
        let request = plan.build_request(task_id).await?;
        let existing_deployment = deployment::find_deployment(&self.k8s, task_id)
            .await
            .map_err(|error| map_sandbox_error(SandboxError::KubeError(error)))?;
        if self.config.sandbox.node_archive_enabled
            && existing_deployment
                .as_ref()
                .is_some_and(node_archive::deployment_uses_node_archives)
        {
            // Pre-flight: reprovisioning after the teardown below schedules
            // unpinned, so refuse to tear down a pin that cannot be honoured.
            node_archive::record_current_pod_nodes(&self.k8s, task_id).await?;
            let require_node = existing_deployment
                .as_ref()
                .is_some_and(node_archive::deployment_requires_retained_node);
            node_archive::retained_node(&self.k8s, task_id, require_node).await?;
        }

        // 1. Delete Deployment (cascades to pods + ReplicaSets). Fail hard on
        //    error: if the delete fails, reprovisioning hits 409 and reuses the
        //    existing Deployment, so the stale pod spec survives and the redeploy
        //    is silently defeated. `delete_deployment` already treats 404 as
        //    success, so this only fails on a genuine API error.
        deployment::delete_deployment(&self.k8s, task_id)
            .await
            .map_err(|error| map_sandbox_error(SandboxError::KubeError(error)))?;
        // 1b. Wait for the old pod to fully terminate before reprovisioning, so
        //     the new pod doesn't race the ReadWriteOnce PVC still attached to
        //     the terminating pod's node ("Multi-Attach" stall).
        wait_absent(
            "pods",
            deployment::wait_for_all_pods_deleted(&self.k8s, task_id),
        )
        .await?;
        wait_absent(
            "Deployment",
            deployment::wait_for_deployment_deleted(&self.k8s, task_id),
        )
        .await?;
        // 2. Delete Service + IngressRoutes and observe absence before
        // rebuilding, so 409 reuse cannot preserve stale routing configuration.
        service::delete_sandbox_service(&self.k8s, task_id)
            .await
            .map_err(|error| map_sandbox_error(SandboxError::KubeError(error)))?;
        ingress::delete_task_ingress_routes(&self.k8s, task_id)
            .await
            .map_err(|error| map_sandbox_error(SandboxError::KubeError(error)))?;
        networkpolicy::delete_task_preview_network_policy(&self.k8s, task_id)
            .await
            .map_err(|error| map_sandbox_error(SandboxError::KubeError(error)))?;
        wait_absent(
            "Service",
            service::wait_for_sandbox_service_deleted(&self.k8s, task_id),
        )
        .await?;
        wait_absent(
            "IngressRoutes",
            ingress::wait_for_task_ingress_routes_deleted(&self.k8s, task_id),
        )
        .await?;
        // Await absence before reprovision: a create racing the old policy's
        // deletion would 409-reuse it, then watch it finalize and leave the pod
        // with no preview policy at all.
        wait_absent(
            "NetworkPolicy",
            networkpolicy::wait_for_task_preview_network_policy_deleted(&self.k8s, task_id),
        )
        .await?;
        // 3. PVC is intentionally NOT deleted — workspace data is preserved

        // 4. Reprovision with fresh config (provision handles PVC reuse)
        Box::pin(self.ensure_running_with_request(task_id, &request, ctx))
            .await
            .map_err(map_sandbox_error)
    }

    #[tracing::instrument(skip_all, fields(task_id = %task_id))]
    async fn remove(
        &self,
        task_id: &str,
        retention: ArchiveRetention,
        _ctx: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, crate::sandbox::lifecycle::RuntimeError> {
        let retain_storage = retention.retains();
        let existing_deployment = deployment::find_deployment(&self.k8s, task_id)
            .await
            .map_err(|error| map_sandbox_error(SandboxError::KubeError(error)))?;
        // Capture the current scheduled node before deleting the Deployment.
        // This synchronous merge is load-bearing; the watcher's write is only a
        // latency optimization and may have failed.
        let node_removal = if self.config.sandbox.node_archive_enabled {
            Some(
                node_archive::prepare_remove(
                    &self.k8s,
                    &self.config,
                    task_id,
                    existing_deployment.as_ref(),
                )
                .await?,
            )
        } else {
            None
        };
        let had_runtime =
            existing_deployment.is_some() || pvc::find_task_pvc(&self.k8s, task_id).await.is_some();
        info!(task_id = %task_id, retain_storage, "Cleaning up sandbox (deleting all resources)");

        // Delete the Deployment first. Its ownerReferences make the Service and
        // IngressRoutes children, and explicit deletes make the absence check
        // strict for every network resource.
        //
        // Every load-bearing step feeds `first_err`: a failure returns an error
        // so the caller retries and the `Removed` completion is withheld. Removal
        // is strict — it reports success only once every resource is observed
        // absent (pods drained, PVC gone) and every owed archive is deleted.
        let mut first_err: Option<crate::sandbox::lifecycle::RuntimeError> = None;

        if let Err(e) = deployment::delete_deployment(&self.k8s, task_id).await {
            warn!(task_id = %task_id, error = %e, "Failed to delete Deployment");
            first_err.get_or_insert(map_sandbox_error(SandboxError::KubeError(e)));
        }
        if let Err(e) = service::delete_sandbox_service(&self.k8s, task_id).await {
            warn!(task_id = %task_id, error = %e, "Failed to delete Service");
            first_err.get_or_insert(map_sandbox_error(SandboxError::KubeError(e)));
        }
        if let Err(e) = ingress::delete_task_ingress_routes(&self.k8s, task_id).await {
            warn!(task_id = %task_id, error = %e, "Failed to delete IngressRoutes");
            first_err.get_or_insert(map_sandbox_error(SandboxError::KubeError(e)));
        }
        if let Err(e) = networkpolicy::delete_task_preview_network_policy(&self.k8s, task_id).await {
            warn!(task_id = %task_id, error = %e, "Failed to delete preview NetworkPolicy");
            first_err.get_or_insert(map_sandbox_error(SandboxError::KubeError(e)));
        }

        // Observe every runtime resource absent before deleting archives or
        // publishing Removed. Skip waits when a delete failed; the caller
        // retries the whole cleanup.
        if first_err.is_none() {
            if let Err(e) = wait_absent(
                "Deployment",
                deployment::wait_for_deployment_deleted(&self.k8s, task_id),
            )
            .await
            {
                warn!(task_id = %task_id, error = %e, "Deployment did not vanish after delete");
                first_err.get_or_insert(e);
            }
        }
        if first_err.is_none() {
            if let Err(e) = wait_absent(
                "pods",
                deployment::wait_for_all_pods_deleted(&self.k8s, task_id),
            )
            .await
            {
                warn!(task_id = %task_id, error = %e, "Pods did not terminate before PVC delete");
                first_err.get_or_insert(e);
            }
        }
        if first_err.is_none() {
            if let Err(e) = wait_absent(
                "Service",
                service::wait_for_sandbox_service_deleted(&self.k8s, task_id),
            )
            .await
            {
                warn!(task_id = %task_id, error = %e, "Service did not vanish after delete");
                first_err.get_or_insert(e);
            }
        }
        if first_err.is_none() {
            if let Err(e) = wait_absent(
                "IngressRoutes",
                ingress::wait_for_task_ingress_routes_deleted(&self.k8s, task_id),
            )
            .await
            {
                warn!(task_id = %task_id, error = %e, "IngressRoutes did not vanish after delete");
                first_err.get_or_insert(e);
            }
        }
        if first_err.is_none() {
            if let Err(e) = wait_absent(
                "NetworkPolicy",
                networkpolicy::wait_for_task_preview_network_policy_deleted(&self.k8s, task_id),
            )
            .await
            {
                warn!(task_id = %task_id, error = %e, "Preview NetworkPolicy did not vanish after delete");
                first_err.get_or_insert(e);
            }
        }
        if let Err(e) = pvc::delete_task_pvc(&self.k8s, task_id).await {
            warn!(task_id = %task_id, error = %e, "Failed to delete PVC");
            first_err.get_or_insert(map_sandbox_error(SandboxError::KubeError(e)));
        }
        // Strict absence: a delete request returns before reclaim completes, so
        // await the PVC actually vanishing before reporting success.
        if first_err.is_none() {
            if let Err(e) = wait_absent("PVC", pvc::wait_for_pvc_deleted(&self.k8s, task_id)).await
            {
                warn!(task_id = %task_id, error = %e, "PVC did not vanish after delete");
                first_err.get_or_insert(e);
            }
        }

        if retain_storage {
            if let Some(node_removal) = &node_removal {
                // Soft close with managed node archives: retain the metadata so
                // a later node-mode start pins to the newest surviving location.
                info!(
                    task_id = %task_id,
                    recorded_nodes = node_removal.nodes.len(),
                    "Retaining task archives on close (revivable)"
                );
            } else {
                // Without archive management the hostPath remains on disk, but
                // placement is best-effort and no retained-node guarantee exists.
                info!(
                    task_id = %task_id,
                    "Retaining S3 archives; node-local storage is unmanaged"
                );
            }
        } else if first_err.is_none() {
            // Delete node-local workspace and Docker archives on every recorded
            // node. Keep driving the other nodes after one failure so a retry
            // has the smallest remaining obligation.
            if let Some(node_removal) = &node_removal {
                for node_name in &node_removal.nodes {
                    if let Err(error) =
                        node_archive::cleanup_node(&self.k8s, &self.config, task_id, node_name)
                            .await
                    {
                        warn!(
                            task_id,
                            node_name,
                            error = %error,
                            "Failed to delete node-local task archives"
                        );
                        first_err.get_or_insert(error);
                    }
                }
            }
        }

        if !retain_storage && first_err.is_none() {
            // Delete this task's transcript and `s3`-backend workspace archives
            // (`<prefix>/<task_id>/...`). These live in the uploads bucket, not
            // the PVC, so the per-task PVC delete above doesn't cover them. The
            // deletes are load-bearing: a "successful" remove must not leave an
            // archive behind. No-op when S3 isn't configured or the prefix is
            // empty (`ebs`/`ephemeral` tasks).
            if let Err(e) = self.delete_task_transcripts(task_id).await {
                warn!(task_id = %task_id, error = %e, "Failed to delete task transcripts from S3");
                first_err.get_or_insert(map_sandbox_error(e));
            }
            if let Err(e) = self.delete_task_sandbox(task_id).await {
                warn!(task_id = %task_id, error = %e, "Failed to delete task workspace from S3");
                first_err.get_or_insert(map_sandbox_error(e));
            }
        }
        if self.config.sandbox.node_archive_enabled && !retain_storage && first_err.is_none() {
            // Metadata is the durable node-cleanup obligation. Delete it last,
            // only after every cleanup pod succeeded and disappeared and after
            // the S3 obligations completed.
            if let Err(error) = node_archive::delete_metadata(&self.k8s, task_id).await {
                warn!(task_id, error = %error, "Failed to delete node archive metadata");
                first_err.get_or_insert(error);
            }
        }

        // Surface any load-bearing failure to the caller: a propagated error
        // withholds lifecycle's `Removed` completion (its sole publisher), so
        // resources or archives that linger get another reconciler pass.
        if let Some(e) = first_err {
            return Err(e);
        }

        let effect = if had_runtime || !retention.retains() {
            RuntimeEffect::Applied
        } else {
            RuntimeEffect::NoOp
        };
        Ok(RuntimeOutcome {
            observed: ObservedStatus::Removed,
            effect,
            diagnostics: Diagnostics::default(),
        })
    }
}

#[async_trait]
impl SandboxObserver for K8sSandboxAdapter {
    async fn sandbox_status(
        &self,
        task_id: &str,
        task_status: crate::task::models::TaskStatus,
        desired_state: crate::task::models::SandboxDesiredState,
    ) -> Result<SandboxStatusFullResponse, SandboxError> {
        let server_configs = if let Some(plan) = &self.plan {
            plan.server_configs(task_id).await
        } else {
            warn!(task_id, "Sandbox plan repositories unavailable for status");
            Vec::new()
        };
        Ok(self
            .status_resolver
            .resolve_sandbox_status(task_id, &server_configs, task_status, desired_state, self)
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
            .resolve_live_status(task_id, task_status, desired_state, self)
            .await)
    }

    async fn server_logs(
        &self,
        task_id: &str,
        server_name: &str,
        options: crate::sandbox::access::LogOptions,
    ) -> Result<BoxStream<'static, Result<String, SandboxError>>, SandboxError> {
        let container_name = if server_name == "sandbox" {
            "sandbox".to_string()
        } else {
            format!("{server_name}-server")
        };

        match pod_reader::stream_container_logs_k8s(&self.k8s, task_id, &container_name, options)
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
}

#[async_trait]
impl SandboxConnector for K8sSandboxAdapter {
    async fn sandbox_client(&self, task_id: &str) -> Result<SandboxClient, SandboxError> {
        self.require_sandbox_ready(task_id).await?;
        let service_name = generate_service_name(task_id);
        SandboxClient::for_service(
            &service_name,
            self.k8s.namespace(),
            self.config.sandbox.grpc_port,
        )
        .map_err(SandboxError::GrpcClient)
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

impl K8sSandboxAdapter {
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

    /// Recreate the sandbox Service and IngressRoutes if they went missing while
    /// the Deployment lives on.
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

        let server_ports: BTreeMap<String, u16> = pod_builder::admitted_server_ports(
            pod_builder::build_servers_map(plan).into_iter().collect(),
            &self.config,
            task_id,
        );
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
            let preview_ports: BTreeSet<u16> = server_ports.values().copied().collect();
            if let Err(e) = networkpolicy::create_task_preview_network_policy(
                &self.k8s,
                task_id,
                &preview_ports,
                &self.config,
                owner.as_ref(),
            )
            .await
            {
                warn!(task_id = %task_id, error = %e, "Failed to recreate preview NetworkPolicy");
            }
        }
        info!(task_id = %task_id, "Recreated missing sandbox Service");
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
        match deployment::get_deployment_replicas(&self.k8s, task_id).await {
            Ok((0, _)) => {
                return ExistingPodResult::Stopped {
                    current_image: deployment::deployment_sandbox_image(&dep),
                };
            }
            Ok(_) => {}
            Err(e) => {
                warn!(
                    task_id,
                    error = %e,
                    "Failed to read Deployment replicas; falling back to pod status"
                );
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

        // Multiple pods can be present during replacement. `list.items[0]` is
        // arbitrary, so pick the best pod and mirror status_resolver.
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
