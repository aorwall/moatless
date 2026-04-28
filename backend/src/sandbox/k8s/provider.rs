//! Kubernetes implementation of the SandboxProvider trait.

use async_trait::async_trait;
use axum::response::sse::Event;
use futures::stream::BoxStream;
use k8s_openapi::api::core::v1::Pod;
use kube::api::{Api, ListParams};
use std::collections::BTreeMap;
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
}

#[async_trait]
impl SandboxProvider for K8sSandboxProvider {
    async fn provision(
        &self,
        task_id: &str,
        request: &ProvisionRequest,
    ) -> Result<ProvisionResponse, SandboxError> {
        info!(task_id = %task_id, user_id = %request.user_id, "Provisioning sandbox");

        let plan = &request.plan;

        // 1. Check for existing pod — never destroy, only reuse or wait
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
            ExistingPodResult::NotFound => {} // Continue provisioning
        }

        // 2. Generate JWT using user_id directly (no DB lookup)
        let task_token_secret = self
            .config
            .sandbox
            .task_token_secret
            .as_ref()
            .ok_or_else(|| SandboxError::ConfigMissing("TASK_TOKEN_SECRET".to_string()))?
            .clone();

        let user_auth_token =
            jwt::generate_user_auth_token(&request.user_id, &request.user_id, &task_token_secret)
                .map_err(|e| SandboxError::JwtError(e.to_string()))?;

        // 3. Create PVC
        let pvc_name = match self.create_or_find_pvc(task_id).await {
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
        info!(task_id = %task_id, "Restarting sandbox (deleting pods, Deployment will recreate)");

        // Ensure Deployment is scaled to 1 (in case it was stopped)
        if let Ok((desired, _)) = deployment::get_deployment_replicas(&self.k8s, task_id).await {
            if desired == 0 {
                deployment::scale_deployment(&self.k8s, task_id, 1)
                    .await
                    .map_err(SandboxError::KubeError)?;
            }
        }

        // Delete pods — Deployment controller will recreate them
        deployment::restart_deployment(&self.k8s, task_id)
            .await
            .map_err(SandboxError::KubeError)?;

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
}

// =============================================================================
// Private helper methods
// =============================================================================

/// Result of checking for an existing pod.
enum ExistingPodResult {
    /// Running and ready — can be reused immediately.
    Ready,
    /// Exists but not yet ready (pending, starting, scaling up).
    Initializing,
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

    /// Create or find existing PVC.
    async fn create_or_find_pvc(&self, task_id: &str) -> Result<String, String> {
        if let Some(name) = pvc::find_task_pvc(&self.k8s, task_id).await {
            info!(task_id, pvc_name = %name, "Found existing PVC, reusing");
            return Ok(name);
        }

        pvc::create_task_pvc(&self.k8s, &self.config, task_id)
            .await
            .map_err(|e| e.to_string())
    }

    /// Check for existing pod — returns richer result for idempotent provisioning.
    ///
    /// - `Ready`: Running pod with sandbox container ready
    /// - `Initializing`: Deployment exists but pod not ready yet (including scaled-to-0 that we scale back up)
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

        let _ = dep; // Deployment exists

        // If Deployment is scaled to 0, scale it back up
        if let Ok((desired, _)) = deployment::get_deployment_replicas(&self.k8s, task_id).await {
            if desired == 0 {
                if let Err(e) = deployment::scale_deployment(&self.k8s, task_id, 1).await {
                    tracing::warn!(task_id, error = %e, "Failed to scale deployment back up");
                }
                return ExistingPodResult::Initializing;
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
