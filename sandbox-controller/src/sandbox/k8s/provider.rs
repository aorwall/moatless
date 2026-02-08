//! Kubernetes implementation of the SandboxProvider trait.

use async_trait::async_trait;
use k8s_openapi::api::core::v1::Pod;
use kube::api::{Api, ListParams};
use std::collections::BTreeMap;
use std::sync::Arc;
use tracing::{info, warn};

use super::auto_provision;
use super::readiness::BackgroundReadinessChecker;
use super::client::K8sClient;
use super::deployment;
use super::health;
use super::ingress;
use super::naming::generate_service_name;
use super::pod_builder;
use super::pvc;
use super::secret;
use super::service;
use super::status_cache::{CachedPodStatus, StatusCache};
use crate::config::Config;
use crate::db::DbPool;
use crate::events::RedisEventPublisher;
use crate::sandbox::error::SandboxError;
use crate::sandbox::jwt;
use crate::sandbox::provider::SandboxProvider;
use crate::sandbox::types::*;

/// Kubernetes-based sandbox provider.
pub struct K8sSandboxProvider {
    pub config: Arc<Config>,
    pub k8s: K8sClient,
    pub status_cache: StatusCache,
    pub pod_template: Arc<String>,
    /// Optional DB pool — enables auto-provisioning on `/start`.
    pub db_pool: Option<DbPool>,
    /// Optional Redis event publisher for environment lifecycle events.
    pub event_publisher: Option<RedisEventPublisher>,
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
        match self.check_existing_pod(task_id, plan).await {
            Ok(Some(target)) => {
                info!(task_id = %task_id, "Reusing existing healthy pod");
                return Ok(ProvisionResponse {
                    execution_target: target,
                });
            }
            Ok(None) => {} // No existing pod, continue provisioning
            Err(e) => {
                warn!(task_id = %task_id, error = %e, "Error checking existing pod");
            }
        }

        // 2. Generate task auth JWT
        let task_token_secret = self
            .config
            .task_token_secret
            .as_ref()
            .ok_or_else(|| SandboxError::ConfigMissing("TASK_TOKEN_SECRET".to_string()))?
            .clone();

        let task_auth_token =
            jwt::generate_task_auth_token(task_id, &request.user_id, &task_token_secret)
                .map_err(|e| SandboxError::JwtError(e.to_string()))?;

        // 3. Create PVC + Secret in parallel
        let credentials = TaskCredentials {
            github_token: plan.git_provider_token.clone(),
            anthropic_api_key: plan.anthropic_api_key.clone(),
            claude_oauth_token: plan.claude_oauth_token.clone(),
            task_auth_token: Some(task_auth_token.clone()),
        };

        let (pvc_result, secret_result) = tokio::join!(
            self.create_or_find_pvc(task_id),
            self.create_or_find_secret(task_id, &credentials),
        );

        let pvc_name = match pvc_result {
            Ok(name) if !name.is_empty() => Some(name),
            Ok(_) => None,
            Err(e) => {
                warn!(task_id = %task_id, error = %e, "Failed to create PVC, proceeding without persistence");
                None
            }
        };

        let secret_name = match secret_result {
            Ok(name) => Some(name),
            Err(e) => {
                warn!(task_id = %task_id, error = %e, "Failed to create secret, using inline env vars");
                None
            }
        };

        // 4. Build pod from template (used as Deployment pod template)
        let pod = pod_builder::build_pod_from_template(
            &self.pod_template,
            &self.config,
            task_id,
            plan,
            pvc_name.as_deref(),
            secret_name.as_deref(),
            &task_auth_token,
        )
        .map_err(SandboxError::PodBuildError)?;

        // Build servers map for service port exposure
        let servers_map = pod_builder::build_servers_map(plan);
        let server_ports: BTreeMap<String, u16> =
            servers_map.iter().map(|(k, v)| (k.clone(), *v)).collect();

        // 5. Create Deployment (wraps pod spec) + Service
        let _deployment_name = deployment::create_workspace_deployment(
            &self.k8s,
            task_id,
            pod,
        )
        .await?;

        let service_name =
            service::create_sandbox_service(&self.k8s, task_id, &server_ports).await?;

        // 5b. Create Traefik IngressRoutes (if proxy_domain is configured)
        if self.config.proxy_domain.is_some() {
            if let Err(e) =
                ingress::create_task_ingress_routes(&self.k8s, task_id, &server_ports, &self.config)
                    .await
            {
                warn!(task_id = %task_id, error = %e, "Failed to create IngressRoutes");
            }
        }

        // 6. Build response immediately (don't block on pod readiness)
        let target = ExecutionTarget {
            mode: "kubernetes".to_string(),
            pod_name: None,
            pod_namespace: self.k8s.namespace().to_string(),
            service_name: service_name.clone(),
            pod_phase: Some("Pending".to_string()),
            status: EnvironmentStatus::Initializing,
            repository_clone_url: plan.git_clone_url.clone(),
            repository_branch: plan.git_branch.clone(),
            sandbox_port: 8001,
            preview_host_port: 3000,
            servers: servers_map,
        };

        // Invalidate status cache
        self.status_cache.invalidate(task_id).await;

        info!(
            task_id = %task_id,
            service_name = %target.service_name,
            server_count = target.servers.len(),
            "Workspace provisioned, returning immediately (pod starting in background)"
        );

        // Publish initializing event + spawn background readiness checker
        self.publish_and_watch_readiness(task_id).await;

        Ok(ProvisionResponse {
            execution_target: target,
        })
    }

    async fn status(&self, task_id: &str) -> Result<StatusResponse, SandboxError> {
        // Check cache first
        if let Some(cached) = self.status_cache.get(task_id).await {
            return Ok(StatusResponse {
                status: cached.phase.to_environment_status(),
                pod_name: cached.pod_name,
                last_error: cached.last_error,
            });
        }

        // Check Deployment replicas first (for Stopped detection)
        let phase = self.resolve_pod_phase(task_id).await;
        let pod_name = if phase == PodPhase::Running || phase == PodPhase::Pending {
            self.get_task_pod_name(task_id).await
        } else {
            None
        };

        // Cache the result
        self.status_cache
            .insert(
                task_id,
                CachedPodStatus {
                    phase: phase.clone(),
                    pod_name: pod_name.clone(),
                    last_error: None,
                },
            )
            .await;

        Ok(StatusResponse {
            status: phase.to_environment_status(),
            pod_name,
            last_error: None,
        })
    }

    async fn workspace_status(
        &self,
        task_id: &str,
    ) -> Result<WorkspaceStatusFullResponse, SandboxError> {
        let service_name = generate_service_name(task_id);

        // 1. Resolve pod phase (checks Deployment replicas for Stopped detection)
        let phase = self.resolve_pod_phase(task_id).await;
        let environment_status = phase.to_environment_status();
        let is_running = phase == PodPhase::Running;

        // 2. If running, fetch sandbox snapshot for agent + server info
        if is_running {
            if let Some(snapshot) =
                health::fetch_sandbox_status(&service_name, self.k8s.namespace(), 8001).await
            {
                let servers = snapshot
                    .servers
                    .iter()
                    .map(|s| {
                        let url = self.build_proxy_url(task_id, s.port);
                        ServerEntry {
                            name: s.name.clone(),
                            label: s.label.clone(),
                            port: s.port,
                            status: ServerStatus::from(s.status.as_str()),
                            url,
                            error: s.error.clone(),
                            default: s.name == "preview",
                            capabilities: get_server_capabilities(&s.name),
                        }
                    })
                    .collect();

                let agent_state = AgentState::from(snapshot.agent.state.as_str());
                let agent_running = matches!(agent_state, AgentState::Running);

                return Ok(WorkspaceStatusFullResponse {
                    workspace_state: WorkspaceState::Exists,
                    environment_status,
                    environment_error: None,
                    agent_state,
                    servers,
                    is_running: agent_running,
                    workspace_ready: true,
                    can_execute: true,
                    resources: snapshot.resources,
                });
            }

            // Sandbox /status failed but pod is running — application not yet serving
            return Ok(WorkspaceStatusFullResponse {
                workspace_state: WorkspaceState::Exists,
                environment_status: EnvironmentStatus::Initializing,
                environment_error: None,
                agent_state: AgentState::None,
                servers: vec![],
                is_running: false,
                workspace_ready: false,
                can_execute: false,
                resources: None,
            });
        }

        // 3. Pod is not running — check if workspace exists
        let workspace_state = match phase {
            PodPhase::NotFound => WorkspaceState::Missing,
            _ => WorkspaceState::Exists,
        };

        // Stopped workspaces can be started, so can_execute = true
        let can_execute = phase == PodPhase::Stopped;

        Ok(WorkspaceStatusFullResponse {
            workspace_state,
            environment_status,
            environment_error: None,
            agent_state: AgentState::None,
            servers: vec![],
            is_running: false,
            workspace_ready: false,
            can_execute,
            resources: None,
        })
    }

    async fn health(&self, task_id: &str) -> HealthResponse {
        let service_name = generate_service_name(task_id);
        let healthy =
            health::check_service_health(&service_name, self.k8s.namespace(), 8001).await;
        HealthResponse { healthy }
    }

    async fn stop(&self, task_id: &str) -> Result<(), SandboxError> {
        info!(task_id = %task_id, "Stopping workspace (scaling Deployment to 0)");

        // Scale Deployment to 0 — keeps Service and IngressRoutes alive
        deployment::scale_deployment(&self.k8s, task_id, 0)
            .await
            .map_err(SandboxError::KubeError)?;

        self.status_cache.invalidate(task_id).await;

        if let Some(ref publisher) = self.event_publisher {
            publisher
                .publish_environment_event(task_id, "stopped", None)
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

                // Publish initializing event + spawn background readiness checker
                self.publish_and_watch_readiness(task_id).await;
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

        self.status_cache.invalidate(task_id).await;
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

        self.status_cache.invalidate(task_id).await;

        // Publish initializing event + watch for pod readiness
        self.publish_and_watch_readiness(task_id).await;

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
        if let Err(e) = secret::delete_task_secret(&self.k8s, task_id).await {
            warn!(task_id = %task_id, error = %e, "Failed to delete secret");
        }

        self.status_cache.invalidate(task_id).await;
        Ok(())
    }
}

// =============================================================================
// Private helper methods
// =============================================================================

impl K8sSandboxProvider {
    /// Publish an "initializing" event and spawn a background readiness checker.
    ///
    /// Used by `provision()`, `start()`, and `restart()` to give the frontend
    /// real-time feedback while a pod is starting up.
    async fn publish_and_watch_readiness(&self, task_id: &str) {
        if let Some(ref publisher) = self.event_publisher {
            publisher
                .publish_environment_event(task_id, "initializing", None)
                .await;
        }

        let service_name = generate_service_name(task_id);
        let bg_k8s = self.k8s.clone();
        let bg_task_id = task_id.to_string();
        let bg_timeout = self.config.k8s_pod_startup_timeout_secs;
        let bg_cache = self.status_cache.clone();
        let bg_publisher = self.event_publisher.clone();
        tokio::spawn(async move {
            let checker = BackgroundReadinessChecker {
                k8s: bg_k8s,
                task_id: bg_task_id,
                service_name,
                timeout_secs: bg_timeout,
                status_cache: bg_cache,
                event_publisher: bg_publisher,
            };
            checker.run().await;
        });
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

    /// Create or find existing secret (updating credentials if found).
    async fn create_or_find_secret(
        &self,
        task_id: &str,
        credentials: &TaskCredentials,
    ) -> Result<String, String> {
        if let Some(name) = secret::find_task_secret(&self.k8s, task_id).await {
            secret::update_task_secret(&self.k8s, task_id, credentials)
                .await
                .map_err(|e| e.to_string())?;
            info!(task_id, secret_name = %name, "Found existing secret, updated credentials");
            return Ok(name);
        }

        secret::create_task_secret(&self.k8s, task_id, credentials)
            .await
            .map_err(|e| e.to_string())
    }

    /// Resolve the effective pod phase, checking Deployment replicas first.
    ///
    /// If a Deployment exists with replicas=0, returns `PodPhase::Stopped`.
    /// Otherwise falls back to checking actual pod status.
    async fn resolve_pod_phase(&self, task_id: &str) -> PodPhase {
        // Check Deployment replicas first
        if let Ok(Some(_dep)) = deployment::find_deployment(&self.k8s, task_id).await {
            if let Ok((desired, _ready)) =
                deployment::get_deployment_replicas(&self.k8s, task_id).await
            {
                if desired == 0 {
                    return PodPhase::Stopped;
                }
            }
        } else {
            // No Deployment found — workspace doesn't exist
            return PodPhase::NotFound;
        }

        // Deployment exists with replicas > 0, check actual pod status
        let pods_api: Api<Pod> =
            Api::namespaced(self.k8s.client.clone(), self.k8s.namespace());
        let lp =
            ListParams::default().labels(&format!("moatless.io/task-id={task_id}"));

        match pods_api.list(&lp).await {
            Ok(list) => {
                if let Some(pod) = list.items.first() {
                    let is_terminating = pod.metadata.deletion_timestamp.is_some();
                    let raw_phase = pod
                        .status
                        .as_ref()
                        .and_then(|s| s.phase.as_deref())
                        .unwrap_or("Unknown");
                    if is_terminating {
                        PodPhase::Terminating
                    } else {
                        PodPhase::from(raw_phase)
                    }
                } else {
                    // Deployment exists but no pods yet (scaling up)
                    PodPhase::Pending
                }
            }
            Err(e) => {
                warn!(task_id = %task_id, error = %e, "Failed to get pod status");
                PodPhase::Unknown
            }
        }
    }

    /// Get the name of the first pod matching a task ID.
    async fn get_task_pod_name(&self, task_id: &str) -> Option<String> {
        let pods_api: Api<Pod> =
            Api::namespaced(self.k8s.client.clone(), self.k8s.namespace());
        let lp =
            ListParams::default().labels(&format!("moatless.io/task-id={task_id}"));

        if let Ok(list) = pods_api.list(&lp).await {
            list.items.first().and_then(|p| p.metadata.name.clone())
        } else {
            None
        }
    }

    /// Check for existing healthy pod that can be reused.
    async fn check_existing_pod(
        &self,
        task_id: &str,
        plan: &ProvisionPlan,
    ) -> Result<Option<ExecutionTarget>, String> {
        // Check if a Deployment already exists
        let dep = deployment::find_deployment(&self.k8s, task_id)
            .await
            .map_err(|e| e.to_string())?;

        if dep.is_none() {
            return Ok(None);
        }

        // If Deployment is scaled to 0, scale it back up
        if let Ok((desired, _)) = deployment::get_deployment_replicas(&self.k8s, task_id).await {
            if desired == 0 {
                let _ = deployment::scale_deployment(&self.k8s, task_id, 1).await;
                return Ok(None); // Will need to wait for pod to come up
            }
        }

        // Check actual pod status
        let pods_api: Api<Pod> =
            Api::namespaced(self.k8s.client.clone(), self.k8s.namespace());
        let lp =
            ListParams::default().labels(&format!("moatless.io/task-id={task_id}"));

        let list = pods_api.list(&lp).await.map_err(|e| e.to_string())?;

        if list.items.is_empty() {
            return Ok(None);
        }

        let pod = &list.items[0];
        let is_terminating = pod.metadata.deletion_timestamp.is_some();
        let raw_phase = pod
            .status
            .as_ref()
            .and_then(|s| s.phase.as_deref())
            .unwrap_or("Unknown");

        if is_terminating {
            return Ok(None);
        }

        let phase = PodPhase::from(raw_phase);

        match phase {
            PodPhase::Running => {
                let service_name = generate_service_name(task_id);
                let healthy =
                    health::check_service_health(&service_name, self.k8s.namespace(), 8001)
                        .await;

                if healthy {
                    let servers_map = pod_builder::build_servers_map(plan);
                    let target = ExecutionTarget {
                        mode: "kubernetes".to_string(),
                        pod_name: pod.metadata.name.clone(),
                        pod_namespace: self.k8s.namespace().to_string(),
                        service_name,
                        pod_phase: Some("Running".to_string()),
                        status: EnvironmentStatus::Ready,
                        repository_clone_url: plan.git_clone_url.clone(),
                        repository_branch: plan.git_branch.clone(),
                        sandbox_port: 8001,
                        preview_host_port: 3000,
                        servers: servers_map,
                    };
                    return Ok(Some(target));
                }
                Ok(None)
            }
            PodPhase::Pending => Ok(None),
            _ => Ok(None),
        }
    }


    /// Auto-provision a workspace from DB state when no Deployment exists.
    ///
    /// Reads task + repository from the database, resolves credentials from
    /// existing K8s Secret or environment variables, and calls `provision()`.
    async fn auto_provision(&self, task_id: &str) -> Result<(), SandboxError> {
        let pool = self.db_pool.as_ref().ok_or_else(|| {
            SandboxError::NotReady(
                "Workspace not provisioned and DATABASE_URL not configured for auto-provisioning. \
                 Use /provision first."
                    .to_string(),
            )
        })?;

        let request =
            auto_provision::build_provision_request(pool, &self.k8s, task_id).await?;

        info!(
            task_id = %task_id,
            repo_name = %request.plan.repo_config.name,
            "Auto-provisioning workspace from DB"
        );

        self.provision(task_id, &request).await?;

        info!(task_id = %task_id, "Auto-provision succeeded");
        Ok(())
    }

    /// Build a proxy URL for a server port using the configured proxy domain.
    /// Returns `None` if no proxy domain is configured.
    fn build_proxy_url(&self, task_id: &str, port: u16) -> Option<String> {
        self.config.proxy_domain.as_ref().map(|domain| {
            format!(
                "{}://{}--{}.{}",
                self.config.proxy_protocol, task_id, port, domain
            )
        })
    }
}
