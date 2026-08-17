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
use super::edge;
use super::ingress;
use super::naming::generate_service_name;
use super::networkpolicy;
use super::node_archive;
use super::owner_ref;
use super::pod_builder;
use super::pod_reader;
use super::pod_status::{is_sandbox_container_ready, pick_best_pod};
use super::pvc;
use super::rescue_pod;
use super::service;
use super::status_resolver::StatusResolver;
use moatless_backend_core::sandbox as core_sandbox;
use moatless_core::config::Config;
use moatless_sandbox_control::client::SandboxClient;
use moatless_sandbox_control::error::SandboxError;
use moatless_sandbox_control::lifecycle::outcome::{DegradationWarning, Diagnostics};
use moatless_sandbox_control::lifecycle::runtime::{
    RuntimeOperationContext, RuntimeOutcome, SandboxRuntime, map_sandbox_error,
};
use moatless_sandbox_control::lifecycle::{
    ArchiveRetention, ObservedStatus, OperationGeneration, ProvisionDeps, RuntimeEffect,
};
use moatless_sandbox_control::models::*;
use moatless_sandbox_control::schemas::*;
use moatless_sandbox_control::status_reader::SandboxStatusReader;
use moatless_sandbox_control::storage::StorageBackend;

/// Kubernetes adapter for lifecycle mutation, observation, and connection.
pub struct K8sSandboxAdapter {
    pub config: Arc<Config>,
    pub k8s: K8sClient,
    pub status_resolver: StatusResolver,
    // `pub` (not `pub(crate)`) is the carve's declared debt: the backend
    // composition root (`startup.rs::build_k8s_sandbox_adapter`) builds this
    // adapter with a struct literal across the crate boundary, and a cross-crate
    // struct literal requires every field it sets to be `pub`. Visibility follows
    // the real consumer, which is now in another crate.
    pub plan: Option<ProvisionDeps>,
    /// Optional S3 client used to delete a task's transcript archive on cleanup.
    /// The sandbox uploads transcripts via backend-proxied gRPC (no pod creds);
    /// the backend owns the credentials, so cleanup of `<prefix>/<task_id>/` lives
    /// here. `None` when S3 isn't configured — cleanup then skips the delete.
    pub s3: Option<Arc<aws_sdk_s3::Client>>,
}

async fn wait_absent<F>(
    resource: &str,
    wait: F,
) -> Result<(), moatless_sandbox_control::lifecycle::RuntimeError>
where
    F: std::future::Future<Output = Result<(), SandboxError>>,
{
    wait.await.map_err(|error| {
        moatless_sandbox_control::lifecycle::RuntimeError::NotAbsent(format!("{resource}: {error}"))
    })
}

impl K8sSandboxAdapter {
    fn require_plan(
        &self,
    ) -> Result<&ProvisionDeps, moatless_sandbox_control::lifecycle::RuntimeError> {
        self.plan.as_ref().ok_or_else(|| {
            moatless_sandbox_control::lifecycle::RuntimeError::Infrastructure(
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
        // Time the whole provision so the PVC path (attach) and the node-local
        // emptyDir path can be compared. The body runs in an inner async block
        // so every early return is captured at a single measurement point.
        let backend = StorageBackend::from_plan(
            &request.plan,
            self.config.sandbox.default_pvc_enabled,
            self.config.sandbox.default_s3_enabled,
        );
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
                    .map_err(|e| SandboxError::KubeError(Box::new(e)))?
                    .ok_or_else(|| {
                        SandboxError::Internal(
                            "stopped sandbox Deployment vanished before start".to_string(),
                        )
                    })?;
                let uses_node_archives = self.config.sandbox.node_archive_enabled
                    && node_archive::deployment_uses_node_archives(&existing_deployment);
                let retained_node = if uses_node_archives {
                    node_archive::record_current_pod_nodes(&self.k8s, task_id)
                        .await
                        .map_err(SandboxError::Runtime)?;
                    node_archive::retained_node(&self.k8s, task_id)
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
                        deployment::set_node_placement(&self.k8s, task_id, retained_node.as_deref())
                            .await
                        .map_err(|e| SandboxError::KubeError(Box::new(e)))?;
                    }
                    deployment::stamp_operation_generation(&self.k8s, task_id, ctx)
                        .await
                        .map_err(|e| SandboxError::KubeError(Box::new(e)))?;
                    deployment::scale_deployment(&self.k8s, task_id, 1)
                        .await
                        .map_err(|e| SandboxError::KubeError(Box::new(e)))?;
                } else {
                    // Tag changed — rebuild the Deployment from the fresh plan.
                    info!(
                        task_id = %task_id,
                        old_image = ?current_image,
                        new_image = %expected,
                        "Sandbox image changed since stop, rebuilding deployment"
                    );
                    // Resolve the PVC from the plan rather than the running
                    // Deployment: a repo that turned `pvcEnabled` on since the
                    // last start gets one created here, and one that turned it
                    // off stops carrying the stale claim forward.
                    let pvc_name = self
                        .create_or_find_pvc(task_id, plan)
                        .await
                        .ok()
                        .filter(|name| !name.is_empty());
                    deployment::redeploy_with_plan(
                        &self.k8s,
                        &self.config,
                        task_id,
                        request,
                        ctx,
                        retained_node.as_deref(),
                        pvc_name.as_deref(),
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
            if let Some(node_name) = node_archive::retained_node(&self.k8s, task_id)
                .await
                .map_err(SandboxError::Runtime)?
            {
                pod_builder::pin_pod_to_node(&mut pod, &node_name)?;
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
        .await
        .map_err(|e| SandboxError::KubeError(Box::new(e)))?;

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

        moatless_sandbox_control::metrics::record_provision(
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
    ) -> Result<RuntimeOutcome, moatless_sandbox_control::lifecycle::RuntimeError> {
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
    ) -> Result<RuntimeOutcome, moatless_sandbox_control::lifecycle::RuntimeError> {
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
                    .map_err(|error| map_sandbox_error(SandboxError::KubeError(Box::new(error))))?;
                RuntimeEffect::Applied
            }
            Ok(None) => {
                info!(task_id = %task_id, "No deployment found, nothing to stop");
                RuntimeEffect::NoOp
            }
            Err(e) => {
                return Err(map_sandbox_error(SandboxError::KubeError(Box::new(e))));
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
    ) -> Result<RuntimeOutcome, moatless_sandbox_control::lifecycle::RuntimeError> {
        info!(task_id = %task_id, "Restarting sandbox (scaling to 0, then back to 1 after termination)");

        // Time the restart so the `ebs` path (which pays a pod-drain wait to
        // release the RWO PVC before the new pod can attach) can be compared to
        // the node-local backends that don't. The backend is read from the
        // running Deployment's env rather than a plan (restart has no plan).
        let backend = self.deployment_storage_backend(task_id).await;
        let started = std::time::Instant::now();
        let existing_deployment = deployment::find_deployment(&self.k8s, task_id)
            .await
            .map_err(|error| map_sandbox_error(SandboxError::KubeError(Box::new(error))))?;
        if self.config.sandbox.node_archive_enabled
            && existing_deployment
                .as_ref()
                .is_some_and(node_archive::deployment_uses_node_archives)
        {
            node_archive::record_current_pod_nodes(&self.k8s, task_id).await?;
            let retained_node = node_archive::retained_node(&self.k8s, task_id).await?;
            deployment::set_node_placement(&self.k8s, task_id, retained_node.as_deref())
                .await
                .map_err(|error| map_sandbox_error(SandboxError::KubeError(Box::new(error))))?;
        }

        // Scale to 0, wait for the pod to terminate (releasing the RWO PVC),
        // then scale back to 1. This also covers the "was stopped" case since
        // restart_deployment always ends at 1 replica.
        let outcome = deployment::restart_deployment(&self.k8s, task_id, ctx).await;

        moatless_sandbox_control::metrics::record_restart(backend, started.elapsed());
        outcome.map_err(map_sandbox_error)?;

        Ok(RuntimeOutcome::applied(ObservedStatus::Starting))
    }

    #[tracing::instrument(skip_all, fields(task_id = %task_id))]
    async fn redeploy(
        &self,
        task_id: &str,
        ctx: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, moatless_sandbox_control::lifecycle::RuntimeError> {
        info!(task_id = %task_id, "Redeploying sandbox (deleting Deployment/Service/IngressRoutes, preserving PVC)");
        let plan = self.require_plan()?;
        let request = plan.build_request(task_id).await?;
        let existing_deployment = deployment::find_deployment(&self.k8s, task_id)
            .await
            .map_err(|error| map_sandbox_error(SandboxError::KubeError(Box::new(error))))?;
        if self.config.sandbox.node_archive_enabled
            && existing_deployment
                .as_ref()
                .is_some_and(node_archive::deployment_uses_node_archives)
        {
            // Record the current placement before the teardown below removes the
            // only pod that can name the owning node, so the replacement can
            // still prefer it.
            node_archive::record_current_pod_nodes(&self.k8s, task_id).await?;
        }

        // 1. Delete Deployment (cascades to pods + ReplicaSets). Fail hard on
        //    error: if the delete fails, reprovisioning hits 409 and reuses the
        //    existing Deployment, so the stale pod spec survives and the redeploy
        //    is silently defeated. `delete_deployment` already treats 404 as
        //    success, so this only fails on a genuine API error.
        deployment::delete_deployment(&self.k8s, task_id)
            .await
            .map_err(|error| map_sandbox_error(SandboxError::KubeError(Box::new(error))))?;
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
            .map_err(|error| map_sandbox_error(SandboxError::KubeError(Box::new(error))))?;
        ingress::delete_task_ingress_routes(&self.k8s, task_id)
            .await
            .map_err(|error| map_sandbox_error(SandboxError::KubeError(Box::new(error))))?;
        networkpolicy::delete_task_preview_network_policy(&self.k8s, task_id)
            .await
            .map_err(|error| map_sandbox_error(SandboxError::KubeError(Box::new(error))))?;
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
    ) -> Result<RuntimeOutcome, moatless_sandbox_control::lifecycle::RuntimeError> {
        let retain_storage = retention.retains();
        let existing_deployment = deployment::find_deployment(&self.k8s, task_id)
            .await
            .map_err(|error| map_sandbox_error(SandboxError::KubeError(Box::new(error))))?;
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
        let mut first_err: Option<moatless_sandbox_control::lifecycle::RuntimeError> = None;

        if let Err(e) = deployment::delete_deployment(&self.k8s, task_id).await {
            warn!(task_id = %task_id, error = %e, "Failed to delete Deployment");
            first_err.get_or_insert(map_sandbox_error(SandboxError::KubeError(Box::new(e))));
        }
        if let Err(e) = service::delete_sandbox_service(&self.k8s, task_id).await {
            warn!(task_id = %task_id, error = %e, "Failed to delete Service");
            first_err.get_or_insert(map_sandbox_error(SandboxError::KubeError(Box::new(e))));
        }
        if let Err(e) = ingress::delete_task_ingress_routes(&self.k8s, task_id).await {
            warn!(task_id = %task_id, error = %e, "Failed to delete IngressRoutes");
            first_err.get_or_insert(map_sandbox_error(SandboxError::KubeError(Box::new(e))));
        }
        if let Err(e) = networkpolicy::delete_task_preview_network_policy(&self.k8s, task_id).await
        {
            warn!(task_id = %task_id, error = %e, "Failed to delete preview NetworkPolicy");
            first_err.get_or_insert(map_sandbox_error(SandboxError::KubeError(Box::new(e))));
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
            first_err.get_or_insert(map_sandbox_error(SandboxError::KubeError(Box::new(e))));
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

    #[tracing::instrument(skip_all, fields(task_id = %task_id))]
    async fn reclaim_storage(
        &self,
        task_id: &str,
        rescue_upload_url: Option<&str>,
        _ctx: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, moatless_sandbox_control::lifecycle::RuntimeError> {
        let deployment = deployment::find_deployment(&self.k8s, task_id)
            .await
            .map_err(|error| map_sandbox_error(SandboxError::KubeError(Box::new(error))))?;
        let claim = pvc::find_task_pvc(&self.k8s, task_id).await;
        if deployment.is_none() && claim.is_none() {
            // Already cold. The reaper reruns every cycle until the task is
            // started or closed, so this is the common call, not the rare one.
            return Ok(RuntimeOutcome {
                observed: ObservedStatus::Stopped,
                effect: RuntimeEffect::NoOp,
                diagnostics: Diagnostics::default(),
            });
        }

        info!(
            task_id = %task_id,
            has_deployment = deployment.is_some(),
            has_pvc = claim.is_some(),
            "Reclaiming stopped sandbox storage (deleting Deployment and PVC)"
        );

        // The Deployment goes first and its pods are awaited: deleting a claim
        // that a live pod template still names leaves the next start Pending on
        // a volume that no longer exists, and a pod still holding the RWO claim
        // blocks its deletion.
        //
        // The rescue also needs the claim detached, so it runs after this and
        // before the delete — the one window where the volume is both readable
        // and still ours.
        if deployment.is_some() {
            deployment::delete_deployment(&self.k8s, task_id)
                .await
                .map_err(|error| map_sandbox_error(SandboxError::KubeError(Box::new(error))))?;
            wait_absent(
                "Deployment",
                deployment::wait_for_deployment_deleted(&self.k8s, task_id),
            )
            .await?;
            wait_absent(
                "pods",
                deployment::wait_for_all_pods_deleted(&self.k8s, task_id),
            )
            .await?;
        }

        if let Some(claim_name) = claim {
            if let Some(put_url) = rescue_upload_url {
                // Load-bearing, unlike the pre-stop push: this is the last read
                // of a volume that is about to stop existing, and lifecycle only
                // asks for it when it could not prove the archive is current.
                // A failure here leaves the claim in place for the next cycle.
                rescue_pod::rescue_workspace(
                    &self.k8s,
                    &self.config,
                    task_id,
                    &claim_name,
                    put_url,
                )
                .await?;
            }
            pvc::delete_task_pvc(&self.k8s, task_id)
                .await
                .map_err(|error| map_sandbox_error(SandboxError::KubeError(Box::new(error))))?;
            wait_absent("PVC", pvc::wait_for_pvc_deleted(&self.k8s, task_id)).await?;
        }

        // Service, IngressRoutes and the preview NetworkPolicy survive: they
        // carry the task's stable URLs, and only `remove` owns them.
        Ok(RuntimeOutcome {
            observed: ObservedStatus::Stopped,
            effect: RuntimeEffect::Applied,
            diagnostics: Diagnostics::default(),
        })
    }
}

/// Outcome of an attach-failure storage fallback attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageFallbackOutcome {
    /// The Deployment was rebuilt on an emptyDir; the caller should surface a
    /// `storage_degraded` degradation warning.
    Rebuilt,
    /// Nothing to swap — the task is not a pvc+s3 tier, or no plan is available.
    /// The Pending pod and its environment error are left in place.
    Skipped,
    /// The rebuild was attempted but failed.
    Failed,
}

/// The Kubernetes runtime's attach-failure remediation, factored behind a trait
/// so the pod watcher can drive it without naming the concrete adapter.
///
/// A pvc+s3 task whose PVC will not attach (`Multi-Attach`, `FailedAttachVolume`)
/// would otherwise sit Pending forever. Because its workspace is snapshotted to
/// S3, it can instead come back up on an emptyDir and restore — a visible
/// downgrade rather than a stall.
#[async_trait]
pub trait StorageFallback: Send + Sync {
    /// Rebuild the task's Deployment on ephemeral storage, keeping the operation
    /// generation so the replacement is the current pod. Best-effort and
    /// idempotent from the caller's side; the caller guards it to once per
    /// generation.
    async fn rebuild_without_pvc(
        &self,
        task_id: &str,
        generation: OperationGeneration,
    ) -> StorageFallbackOutcome;
}

#[async_trait]
impl StorageFallback for K8sSandboxAdapter {
    async fn rebuild_without_pvc(
        &self,
        task_id: &str,
        generation: OperationGeneration,
    ) -> StorageFallbackOutcome {
        let plan = match self.require_plan() {
            Ok(plan) => plan,
            Err(e) => {
                warn!(task_id, error = %e, "attach-failure fallback: no plan available");
                return StorageFallbackOutcome::Skipped;
            }
        };
        let request = match plan.build_request(task_id).await {
            Ok(request) => request,
            Err(e) => {
                warn!(task_id, error = %e, "attach-failure fallback: could not build request");
                return StorageFallbackOutcome::Failed;
            }
        };

        // Only a pvc+s3 tier can be rescued: without S3 there is nothing to
        // restore an emptyDir from, so the environment error stays. Without a PVC
        // there was no attach to fail, so there is nothing to swap.
        let backend = StorageBackend::from_plan(
            &request.plan,
            self.config.sandbox.default_pvc_enabled,
            self.config.sandbox.default_s3_enabled,
        );
        if !(backend.uses_pvc() && backend.syncs_sandbox()) {
            info!(
                task_id,
                "attach-failure fallback: not a pvc+s3 tier, leaving pod Pending"
            );
            return StorageFallbackOutcome::Skipped;
        }

        // Capture the claim's fate before rebuilding: a bound PVC (the ordinary
        // Multi-Attach case) is kept so the next clean start can retry it, an
        // unbound one is an orphan to clear.
        let pvc_bound = pvc::task_pvc_is_bound(&self.k8s, task_id).await;

        let ctx = RuntimeOperationContext::new(generation);
        info!(task_id, %generation, "attach-failure fallback: rebuilding Deployment on emptyDir");
        match deployment::redeploy_with_plan(
            &self.k8s,
            &self.config,
            task_id,
            &request,
            ctx,
            None, // retained_node — an emptyDir has no node to prefer
            None, // pvc_name None → emptyDir; the sandbox restores from S3
        )
        .await
        {
            Ok(()) => {
                if !pvc_bound {
                    if let Err(e) = pvc::delete_task_pvc(&self.k8s, task_id).await {
                        warn!(task_id, error = %e, "attach-failure fallback: failed to delete orphaned unbound PVC");
                    }
                }
                info!(
                    task_id,
                    "attach-failure fallback: Deployment rebuilt on emptyDir; workspace will restore from S3"
                );
                StorageFallbackOutcome::Rebuilt
            }
            Err(e) => {
                warn!(task_id, error = %e, "attach-failure fallback: rebuild failed");
                StorageFallbackOutcome::Failed
            }
        }
    }
}

#[async_trait]
impl SandboxStatusReader for K8sSandboxAdapter {
    async fn sandbox_status(
        &self,
        task_id: &str,
        task_status: moatless_backend_core::models::TaskStatus,
        desired_state: moatless_backend_core::models::SandboxDesiredState,
    ) -> Result<SandboxStatusFullResponse, SandboxError> {
        let sandbox_id = self.sandbox_id_for_task(task_id).await?;
        let server_configs = if let Some(plan) = &self.plan {
            plan.server_configs(task_id).await
        } else {
            warn!(task_id, "Sandbox plan repositories unavailable for status");
            Vec::new()
        };
        Ok(self
            .status_resolver
            .resolve_sandbox_status(
                task_id,
                &sandbox_id,
                &server_configs,
                task_status,
                desired_state,
                self,
            )
            .await)
    }

    async fn live_status(
        &self,
        task_id: &str,
        task_status: moatless_backend_core::models::TaskStatus,
        desired_state: moatless_backend_core::models::SandboxDesiredState,
    ) -> Result<moatless_sandbox_control::live_status::LiveStatus, SandboxError> {
        let sandbox_id = self.sandbox_id_for_task(task_id).await?;
        Ok(self
            .status_resolver
            .resolve_live_status(task_id, &sandbox_id, task_status, desired_state, self)
            .await)
    }

    async fn server_logs(
        &self,
        task_id: &str,
        server_name: &str,
        options: moatless_sandbox_control::access::LogOptions,
    ) -> Result<BoxStream<'static, Result<String, SandboxError>>, SandboxError> {
        let container_name = if server_name == "sandbox" {
            "sandbox".to_string()
        } else {
            format!("{server_name}-server")
        };
        let sandbox_id = self.sandbox_id_for_task(task_id).await?;

        match pod_reader::stream_container_logs_k8s(
            &self.k8s,
            &sandbox_id,
            &container_name,
            options,
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
}

#[async_trait]
impl core_sandbox::SandboxConnector for K8sSandboxAdapter {
    async fn sandbox_client(
        &self,
        task_id: &str,
    ) -> Result<SandboxClient, core_sandbox::SandboxError> {
        let sandbox_id = self
            .sandbox_id_for_task(task_id)
            .await
            .map_err(SandboxError::into_contract)?;
        self.require_sandbox_ready(&sandbox_id)
            .await
            .map_err(SandboxError::into_contract)?;
        let service_name = generate_service_name(&sandbox_id);
        SandboxClient::for_service(
            &service_name,
            self.k8s.namespace(),
            self.config.sandbox.grpc_port,
        )
        .map_err(|e| core_sandbox::SandboxError::GrpcClient(e.to_string()))
    }
}

#[async_trait]
impl core_sandbox::SandboxPortPublisher for K8sSandboxAdapter {
    /// Publish one port on a running sandbox — the Script path into the same
    /// edge provisioning builds for declared preview servers.
    ///
    /// Refuses a reserved infrastructure port before touching the cluster, for
    /// the same reason `admitted_server_ports` filters the declared ones:
    /// routing the authenticated preview edge onto the control plane, the gRPC
    /// channel or the Docker socket would be a sandbox escape.
    async fn publish_port(
        &self,
        task_id: &str,
        name: &str,
        port: u16,
    ) -> Result<Option<String>, core_sandbox::SandboxError> {
        if core_sandbox::is_reserved_sandbox_port(port, self.config.sandbox.grpc_port) {
            return Err(core_sandbox::SandboxError::BadRequest(format!(
                "Port {port} is reserved by the sandbox and cannot be exposed"
            )));
        }

        let sandbox_id = self
            .sandbox_id_for_task(task_id)
            .await
            .map_err(SandboxError::into_contract)?;

        self.require_sandbox_ready(&sandbox_id)
            .await
            .map_err(SandboxError::into_contract)?;

        edge::publish_task_port(&self.k8s, &sandbox_id, name, port, &self.config)
            .await
            .map_err(|e| core_sandbox::SandboxError::Other(e.to_string()))
    }
}

impl K8sSandboxAdapter {
    async fn sandbox_id_for_task(&self, task_id: &str) -> Result<String, SandboxError> {
        match &self.plan {
            Some(plan) => plan
                .sandbox_id_for_task(task_id)
                .await
                .map_err(SandboxError::Runtime),
            None => Ok(task_id.to_string()),
        }
    }
}

#[async_trait]
impl core_sandbox::ClusterDiagnostics for K8sSandboxAdapter {
    async fn list_sandbox_diagnostics(
        &self,
    ) -> Result<Vec<core_sandbox::SandboxDiagnosticsEntry>, core_sandbox::SandboxError> {
        crate::pod_reader::fetch_fleet_diagnostics(&self.k8s)
            .await
            .map_err(|e| core_sandbox::SandboxError::Other(e.to_string()))
    }
}

// =============================================================================
// Private helper methods
// =============================================================================

/// Inspect a running Deployment's sandbox container for the
/// `WORKSPACE_SYNC_ENABLED=true` env that marks the S3 switch as on.
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
                    Err(e) => Err(SandboxError::KubeError(Box::new(e))),
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

        // The plan only knows the *declared* ports. A Script may have published
        // one against the live sandbox, and its IngressRoute — a separate object
        // that survived the missing Service — is the only record of it. Union
        // those back into the Service, or the rebuilt Service drops the port and
        // the route that outlived it starts pointing at nothing.
        //
        // Only into the Service and the policy: those routes still exist, and
        // recreating them under a second name would give Traefik two rules for
        // one hostname.
        let declared: BTreeSet<u16> = server_ports.values().copied().collect();
        let reserved = pod_builder::reserved_sandbox_ports(&self.config);
        let mut service_ports = server_ports.clone();
        for port in edge::published_task_ports(&self.k8s, task_id).await {
            if declared.contains(&port) || reserved.contains(&port) {
                continue;
            }
            info!(task_id = %task_id, port, "Preserving dynamically published port across network heal");
            service_ports.insert(edge::script_port_name(port), port);
        }

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
            &service_ports,
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
            let preview_ports: BTreeSet<u16> = service_ports.values().copied().collect();
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

    /// Resolve both storage switches for a *running* task from cluster state
    /// (used where there's no `ProvisionPlan`, e.g. restart). Each is read from
    /// the resource that realizes it: an existing PVC is the `pvc` switch, and
    /// the `WORKSPACE_SYNC_ENABLED` env on the sandbox container is the `s3`
    /// switch.
    async fn deployment_storage_backend(&self, task_id: &str) -> StorageBackend {
        StorageBackend {
            pvc: pvc::find_task_pvc(&self.k8s, task_id).await.is_some(),
            s3: matches!(
                deployment::find_deployment(&self.k8s, task_id).await,
                Ok(Some(dep)) if deployment_has_workspace_sync(&dep)
            ),
        }
    }

    /// Create or find existing PVC. A per-repo `resources.storage` override
    /// (if set) sizes a freshly created PVC; existing PVCs are reused as-is.
    ///
    /// Returns an empty string (→ no PVC) when the repo's `resources.pvcEnabled`
    /// switch is off: `build_pod` then mounts a node-local emptyDir at
    /// `/opt/moatless` instead. The check runs *before* `find_task_pvc` so a repo
    /// that flipped the switch off stops reusing any stale PVC.
    async fn create_or_find_pvc(
        &self,
        task_id: &str,
        plan: &ProvisionPlan,
    ) -> Result<String, String> {
        if !StorageBackend::from_plan(
            plan,
            self.config.sandbox.default_pvc_enabled,
            self.config.sandbox.default_s3_enabled,
        )
        .uses_pvc()
        {
            info!(task_id, "PVC switch off — skipping PVC creation");
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
        let deleted =
            moatless_core::file_storage::s3::delete_objects_by_prefix(s3, bucket, &prefix)
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
        let deleted =
            moatless_core::file_storage::s3::delete_objects_by_prefix(s3, bucket, &prefix)
                .await
                .map_err(|e| {
                    SandboxError::Internal(format!("delete workspace for {task_id}: {e}"))
                })?;
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
