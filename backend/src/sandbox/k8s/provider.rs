//! Kubernetes implementation of the SandboxProvider trait.

use async_trait::async_trait;
use axum::response::sse::Event;
use futures::stream::BoxStream;
use k8s_openapi::api::core::v1::Pod;
use kube::api::{Api, ListParams};
use std::collections::BTreeMap;
use std::sync::Arc;
use tracing::{info, warn};

use super::auto_provision;
use super::client::K8sClient;
use super::deployment;
use super::health;
use super::ingress;
use super::naming::generate_service_name;
use super::pod_builder;
use super::pod_status::is_sandbox_container_ready;
use super::pvc;
use super::secret;
use super::service;
use super::status_resolver::StatusResolver;
use crate::auth::provider_token_dao::ProviderTokenRepository;
use crate::auth::users::UserRepository;
use crate::config::Config;
use crate::events::EventPublisher;
use crate::repository::dao::RepoRepository;
use crate::sandbox::error::SandboxError;
use crate::sandbox::jwt;
use crate::sandbox::provider::SandboxProvider;
use crate::sandbox::models::*;
use crate::sandbox::schemas::*;
use crate::task::dao::TaskRepository;

/// Kubernetes-based sandbox provider.
pub struct K8sSandboxProvider {
    pub config: Arc<Config>,
    pub k8s: K8sClient,
    pub status_resolver: StatusResolver,
    /// Optional user repository — enables user login resolution on `/provision`.
    pub users: Option<Arc<dyn UserRepository>>,
    /// Optional task repository — enables auto-provisioning on `/start`.
    pub tasks: Option<Arc<dyn TaskRepository>>,
    /// Optional repo repository — enables auto-provisioning on `/start`.
    pub repos: Option<Arc<dyn RepoRepository>>,
    /// Optional provider token repository — enables credential resolution.
    pub provider_tokens: Option<Arc<dyn ProviderTokenRepository>>,
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
        info!(task_id = %task_id, user_id = %request.user_id, "Provisioning workspace");

        let plan = &request.plan;

        // 1. Check for existing healthy pod
        match self.check_existing_pod(task_id).await {
            Ok(true) => {
                info!(task_id = %task_id, "Reusing existing healthy pod");
                return Ok(ProvisionResponse {
                    status: EnvironmentStatus::Ready,
                });
            }
            Ok(false) => {} // No existing pod, continue provisioning
            Err(e) => {
                warn!(task_id = %task_id, error = %e, "Error checking existing pod");
            }
        }

        // 2. Resolve user login for JWT generation
        let task_token_secret = self
            .config
            .sandbox.task_token_secret
            .as_ref()
            .ok_or_else(|| SandboxError::ConfigMissing("TASK_TOKEN_SECRET".to_string()))?
            .clone();

        let user_login = if let Some(ref users) = self.users {
            match users.get_user(&request.user_id).await {
                Ok(Some(user)) => user.login,
                _ => request.user_id.clone(), // Fallback to user_id as login
            }
        } else {
            request.user_id.clone()
        };

        // 3. Check existing user secret for a valid token, reuse or regenerate
        let existing_creds = secret::read_user_credentials(&self.k8s, &request.user_id).await;
        let user_auth_token = match existing_creds
            .as_ref()
            .and_then(|c| c.user_auth_token.as_deref())
        {
            Some(token) if !jwt::is_token_near_expiry(token, &task_token_secret, 7 * 24 * 3600) => {
                // Token still valid and not near expiry — reuse it
                token.to_string()
            }
            _ => {
                // No token or near expiry — generate a fresh one
                jwt::generate_user_auth_token(&request.user_id, &user_login, &task_token_secret)
                    .map_err(|e| SandboxError::JwtError(e.to_string()))?
            }
        };

        // 4. Create PVC
        let pvc_name = match self.create_or_find_pvc(task_id).await {
            Ok(name) if !name.is_empty() => Some(name),
            Ok(_) => None,
            Err(e) => {
                warn!(task_id = %task_id, error = %e, "Failed to create PVC, proceeding without persistence");
                None
            }
        };

        // 5. Build pod spec
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
        let _deployment_name = deployment::create_workspace_deployment(
            &self.k8s,
            task_id,
            pod,
        )
        .await?;

        let service_name =
            service::create_sandbox_service(&self.k8s, task_id, &server_ports, self.config.sandbox.grpc_port).await?;

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
            "Workspace provisioned, returning immediately (pod starting in background)"
        );

        // Publish initializing event (pod_watcher handles the rest)
        self.publish_initializing(task_id).await;

        Ok(ProvisionResponse {
            status: EnvironmentStatus::Initializing,
        })
    }

    async fn status(&self, task_id: &str) -> Result<StatusResponse, SandboxError> {
        Ok(self.status_resolver.resolve_status(task_id).await)
    }

    async fn sandbox_status(
        &self,
        task_id: &str,
    ) -> Result<WorkspaceStatusFullResponse, SandboxError> {
        Ok(self.status_resolver.resolve_sandbox_status(task_id).await)
    }

    async fn health(&self, task_id: &str) -> HealthResponse {
        self.status_resolver.resolve_health(task_id).await
    }

    async fn stop(&self, task_id: &str) -> Result<(), SandboxError> {
        info!(task_id = %task_id, "Stopping workspace (scaling Deployment to 0)");

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
                .publish_sandbox_event(task_id, moatless_events::SandboxSubType::Stopped, None)
                .await;
        }

        Ok(())
    }

    async fn start(&self, task_id: &str) -> Result<(), SandboxError> {
        info!(task_id = %task_id, "Starting workspace");

        // Check if a Deployment exists
        match deployment::find_deployment(&self.k8s, task_id).await {
            Ok(Some(_)) => {
                info!(task_id = %task_id, "Deployment found, scaling to 1");
                deployment::scale_deployment(&self.k8s, task_id, 1)
                    .await
                    .map_err(SandboxError::KubeError)?;

                // Publish initializing event (pod_watcher handles readiness)
                self.publish_initializing(task_id).await;
            }
            Ok(None) => {
                // No Deployment — try auto-provisioning from DB
                info!(task_id = %task_id, "No Deployment found, attempting auto-provision from DB");
                self.auto_provision(task_id).await?;
            }
            Err(e) => {
                tracing::error!(task_id = %task_id, error = %e, "Failed to check Deployment");
                return Err(SandboxError::KubeError(e));
            }
        }

        info!(task_id = %task_id, "Workspace started successfully");
        Ok(())
    }

    async fn restart(&self, task_id: &str) -> Result<(), SandboxError> {
        info!(task_id = %task_id, "Restarting workspace (deleting pods, Deployment will recreate)");

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

    async fn cleanup(&self, task_id: &str) -> Result<(), SandboxError> {
        info!(task_id = %task_id, "Cleaning up workspace (deleting all resources)");

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
        // Delete legacy task secret if it exists (migration cleanup).
        // User secrets are NOT deleted — they are shared across tasks.
        if let Err(e) = secret::delete_task_secret(&self.k8s, task_id).await {
            warn!(task_id = %task_id, error = %e, "Failed to delete legacy task secret");
        }

        Ok(())
    }

    async fn server_status(
        &self,
        task_id: &str,
    ) -> Result<ServerStatusResponse, SandboxError> {
        let container_statuses =
            health::fetch_server_container_statuses(&self.k8s, task_id).await;

        let server_configs = match (&self.tasks, &self.repos) {
            (Some(ref tasks), Some(ref repos)) => {
                match auto_provision::get_server_configs(tasks.as_ref(), repos.as_ref(), task_id).await {
                    Ok(configs) => configs,
                    Err(e) => {
                        warn!(task_id = %task_id, error = %e, "Failed to get server configs for server_status");
                        vec![]
                    }
                }
            }
            _ => vec![],
        };

        let servers: Vec<ServerSnapshot> = server_configs
            .iter()
            .map(|config| {
                let container = container_statuses.iter().find(|cs| {
                    let name = cs.name.strip_suffix("-server").unwrap_or(&cs.name);
                    name == config.name
                });
                let status = container.map_or_else(|| "starting".to_string(), |cs| cs.to_server_status().to_string());
                let mut snapshot = ServerSnapshot::from_config(config, &status);
                snapshot.error = container.and_then(|cs| cs.waiting_reason.clone());
                snapshot
            })
            .collect();

        Ok(ServerStatusResponse { servers })
    }

    async fn server_logs(
        &self,
        task_id: &str,
        server_name: &str,
    ) -> Result<BoxStream<'static, Result<Event, SandboxError>>, SandboxError> {
        let container_name = format!("{server_name}-server");

        let stream = health::stream_container_logs_k8s(
            &self.k8s,
            task_id,
            &container_name,
            500,
        )
        .await
        .map_err(SandboxError::Internal)?;

        Ok(stream)
    }

    async fn search_files(
        &self,
        task_id: &str,
        query: &str,
        limit: u32,
    ) -> Result<Vec<FileSearchResult>, SandboxError> {
        let service_name = generate_service_name(task_id);
        let results =
            health::search_sandbox_files(&service_name, self.k8s.namespace(), self.config.sandbox.grpc_port, query, limit)
                .await
                .map_err(SandboxError::Internal)?;

        Ok(results)
    }
}

// =============================================================================
// Private helper methods
// =============================================================================

impl K8sSandboxProvider {
    /// Publish an "initializing" event (fire-and-forget).
    ///
    /// The pod_watcher handles all subsequent readiness tracking and server
    /// container monitoring. No background readiness checker is spawned.
    async fn publish_initializing(&self, task_id: &str) {
        if let Some(ref publisher) = self.event_publisher {
            publisher
                .publish_sandbox_event(task_id, moatless_events::SandboxSubType::Initializing, None)
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

    /// Create or find existing user secret (updating credentials if found).
    #[allow(dead_code)] // Infrastructure method for future use
    async fn create_or_find_user_secret(
        &self,
        user_id: &str,
        credentials: &UserCredentials,
    ) -> Result<String, String> {
        if let Some(name) = secret::find_user_secret(&self.k8s, user_id).await {
            secret::update_user_secret(&self.k8s, user_id, credentials)
                .await
                .map_err(|e| e.to_string())?;
            info!(user_id, secret_name = %name, "Found existing user secret, updated credentials");
            return Ok(name);
        }

        secret::create_user_secret(&self.k8s, user_id, credentials)
            .await
            .map_err(|e| e.to_string())
    }

    /// Check for existing healthy pod that can be reused.
    ///
    /// Returns `true` if a running, ready pod already exists for this task.
    async fn check_existing_pod(
        &self,
        task_id: &str,
    ) -> Result<bool, String> {
        // Check if a Deployment already exists
        let dep = deployment::find_deployment(&self.k8s, task_id)
            .await
            .map_err(|e| e.to_string())?;

        if dep.is_none() {
            return Ok(false);
        }

        // If Deployment is scaled to 0, scale it back up
        if let Ok((desired, _)) = deployment::get_deployment_replicas(&self.k8s, task_id).await {
            if desired == 0 {
                if let Err(e) = deployment::scale_deployment(&self.k8s, task_id, 1).await {
                    tracing::warn!(task_id, error = %e, "Failed to scale deployment back up");
                }
                return Ok(false); // Will need to wait for pod to come up
            }
        }

        // Check actual pod status
        let pods_api: Api<Pod> =
            Api::namespaced(self.k8s.client.clone(), self.k8s.namespace());
        let lp =
            ListParams::default().labels(&format!("moatless.io/task-id={task_id}"));

        let list = pods_api.list(&lp).await.map_err(|e| e.to_string())?;

        if list.items.is_empty() {
            return Ok(false);
        }

        let pod = &list.items[0];
        let is_terminating = pod.metadata.deletion_timestamp.is_some();
        let raw_phase = pod
            .status
            .as_ref()
            .and_then(|s| s.phase.as_deref())
            .unwrap_or("Unknown");

        if is_terminating {
            return Ok(false);
        }

        let phase = PodPhase::from(raw_phase);

        match phase {
            PodPhase::Running => Ok(is_sandbox_container_ready(pod)),
            _ => Ok(false),
        }
    }


    /// Auto-provision a workspace from DB state when no Deployment exists.
    ///
    /// Reads task + repository from the database, resolves credentials from
    /// existing K8s Secret or environment variables, and calls `provision()`.
    async fn auto_provision(&self, task_id: &str) -> Result<(), SandboxError> {
        let tasks = self.tasks.as_ref().ok_or_else(|| {
            SandboxError::NotReady(
                "Workspace not provisioned and DATABASE_URL not configured for auto-provisioning. \
                 Use /provision first."
                    .to_string(),
            )
        })?;
        let repos = self.repos.as_ref().ok_or_else(|| {
            SandboxError::NotReady(
                "Repository access not configured for auto-provisioning.".to_string(),
            )
        })?;
        let provider_tokens = self.provider_tokens.as_ref().ok_or_else(|| {
            SandboxError::NotReady(
                "Provider tokens not configured for auto-provisioning.".to_string(),
            )
        })?;

        let request = auto_provision::build_provision_request(
            tasks.as_ref(),
            repos.as_ref(),
            provider_tokens.as_ref(),
            &self.k8s,
            task_id,
        )
        .await?;

        info!(
            task_id = %task_id,
            repo_name = %request.plan.repo_config.name,
            "Auto-provisioning workspace from DB"
        );

        self.provision(task_id, &request).await?;

        info!(task_id = %task_id, "Auto-provision succeeded");
        Ok(())
    }
}
