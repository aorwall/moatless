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
use crate::sandbox::sandbox_state::SandboxStateCache;
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
    /// Optional shared Redis sandbox state cache (L2, cross-replica).
    pub sandbox_state: Option<SandboxStateCache>,
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

        // 2. Resolve user login for JWT generation
        let task_token_secret = self
            .config
            .task_token_secret
            .as_ref()
            .ok_or_else(|| SandboxError::ConfigMissing("TASK_TOKEN_SECRET".to_string()))?
            .clone();

        let user_login = if let Some(ref pool) = self.db_pool {
            match crate::db::users::get_user(pool, &request.user_id).await {
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

        // 4. Create PVC (no more per-user secrets — credentials fetched via gRPC)
        let pvc_name = match self.create_or_find_pvc(task_id).await {
            Ok(name) if !name.is_empty() => Some(name),
            Ok(_) => None,
            Err(e) => {
                warn!(task_id = %task_id, error = %e, "Failed to create PVC, proceeding without persistence");
                None
            }
        };

        // 5. Build pod from template with USER_AUTH_TOKEN as plain env var
        let pod = pod_builder::build_pod_from_template(
            &self.pod_template,
            &self.config,
            task_id,
            plan,
            pvc_name.as_deref(),
            Some(&user_auth_token),
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
            service::create_sandbox_service(&self.k8s, task_id, &server_ports, self.config.sandbox_grpc_port).await?;

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
            sandbox_port: self.config.sandbox_grpc_port,
            preview_host_port: 3000,
            servers: servers_map,
        };

        // Invalidate caches
        self.status_cache.invalidate(task_id).await;
        if let Some(ref state_cache) = self.sandbox_state {
            state_cache.delete(task_id).await;
        }

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
        // L1: Check moka cache first
        if let Some(cached) = self.status_cache.get(task_id).await {
            return Ok(StatusResponse {
                status: cached.phase.to_environment_status(),
                pod_name: cached.pod_name,
                last_error: cached.last_error,
            });
        }

        // L2: Check Redis cache
        if let Some(ref state_cache) = self.sandbox_state {
            if let Some(cached) = state_cache.get_if_fresh(task_id).await {
                // Populate L1 from L2
                let phase = env_status_to_pod_phase(&cached.environment_status);
                self.status_cache
                    .insert(
                        task_id,
                        CachedPodStatus {
                            phase,
                            pod_name: cached.pod_name.clone(),
                            last_error: cached.environment_error.clone(),
                        },
                    )
                    .await;

                return Ok(StatusResponse {
                    status: cached.environment_status,
                    pod_name: cached.pod_name,
                    last_error: cached.environment_error,
                });
            }
        }

        // L3: Check Deployment replicas first (for Stopped detection)
        let phase = self.resolve_pod_phase(task_id).await;
        let pod_name = if phase == PodPhase::Running || phase == PodPhase::Pending {
            self.get_task_pod_name(task_id).await
        } else {
            None
        };

        // Populate L1
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
        // L2: Check Redis cache for fresh full state
        if let Some(ref state_cache) = self.sandbox_state {
            if let Some(cached) = state_cache.get_if_fresh(task_id).await {
                let servers = cached
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

                let workspace_ready = cached.environment_status == EnvironmentStatus::Ready;
                let can_execute = workspace_ready
                    || cached.environment_status == EnvironmentStatus::Stopped;

                return Ok(WorkspaceStatusFullResponse {
                    workspace_state: cached.workspace_state,
                    environment_status: cached.environment_status,
                    environment_error: cached.environment_error,
                    agent_state: cached.agent_state,
                    servers,
                    is_running: cached.is_running,
                    workspace_ready,
                    can_execute,
                    resources: cached.resources,
                });
            }
        }

        let service_name = generate_service_name(task_id);

        // 1. Resolve pod phase (checks Deployment replicas for Stopped detection)
        let phase = self.resolve_pod_phase(task_id).await;
        let environment_status = phase.to_environment_status();
        let is_running = phase == PodPhase::Running;

        // 2. If running, fetch sandbox snapshot for agent + server info
        if is_running {
            if let Some(snapshot) =
                health::fetch_sandbox_status(&service_name, self.k8s.namespace(), self.config.sandbox_grpc_port).await
            {
                let agent_state = AgentState::from(snapshot.agent.state.as_str());
                let agent_running = matches!(agent_state, AgentState::Running);

                // Write to Redis L2 cache
                if let Some(ref state_cache) = self.sandbox_state {
                    let pod_name = self.get_task_pod_name(task_id).await;
                    let cached = CachedSandboxState {
                        environment_status: environment_status.clone(),
                        environment_error: None,
                        workspace_state: WorkspaceState::Exists,
                        pod_name,
                        agent_state: agent_state.clone(),
                        servers: snapshot.servers.iter().map(|s| ServerSnapshot {
                            name: s.name.clone(),
                            label: s.label.clone(),
                            port: s.port,
                            builtin: s.builtin,
                            status: s.status.clone(),
                            error: s.error.clone(),
                            pid: s.pid,
                        }).collect(),
                        resources: snapshot.resources.clone(),
                        is_running: agent_running,
                        updated_at: chrono::Utc::now().timestamp_millis(),
                    };
                    state_cache.set(task_id, &cached).await;
                }

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

            // gRPC status call failed but pod is running — try stale cache as fallback
            if let Some(ref state_cache) = self.sandbox_state {
                if let Some(cached) = state_cache.get(task_id).await {
                    if !cached.servers.is_empty() {
                        let servers = cached
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

                        return Ok(WorkspaceStatusFullResponse {
                            workspace_state: WorkspaceState::Exists,
                            environment_status,
                            environment_error: None,
                            agent_state: cached.agent_state,
                            servers,
                            is_running: cached.is_running,
                            workspace_ready: true,
                            can_execute: true,
                            resources: cached.resources,
                        });
                    }
                }
            }

            // No cache or cache had empty servers — derive from repo config in DB
            let servers = self.servers_from_repo_config(task_id).await;
            return Ok(WorkspaceStatusFullResponse {
                workspace_state: WorkspaceState::Exists,
                environment_status,
                environment_error: None,
                agent_state: AgentState::None,
                servers,
                is_running: false,
                workspace_ready: true,
                can_execute: true,
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
            health::check_service_health(&service_name, self.k8s.namespace(), self.config.sandbox_grpc_port).await;
        HealthResponse { healthy }
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

        self.status_cache.invalidate(task_id).await;
        if let Some(ref state_cache) = self.sandbox_state {
            state_cache.delete(task_id).await;
        }

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
        if let Some(ref state_cache) = self.sandbox_state {
            state_cache.delete(task_id).await;
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

        self.status_cache.invalidate(task_id).await;
        if let Some(ref state_cache) = self.sandbox_state {
            state_cache.delete(task_id).await;
        }

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
        // Delete legacy task secret if it exists (migration cleanup).
        // User secrets are NOT deleted — they are shared across tasks.
        if let Err(e) = secret::delete_task_secret(&self.k8s, task_id).await {
            warn!(task_id = %task_id, error = %e, "Failed to delete legacy task secret");
        }

        self.status_cache.invalidate(task_id).await;
        if let Some(ref state_cache) = self.sandbox_state {
            state_cache.delete(task_id).await;
        }
        Ok(())
    }

    async fn server_status(
        &self,
        task_id: &str,
    ) -> Result<ServerStatusResponse, SandboxError> {
        let service_name = generate_service_name(task_id);
        let snapshot =
            health::fetch_sandbox_status(&service_name, self.k8s.namespace(), self.config.sandbox_grpc_port)
                .await
                .ok_or_else(|| {
                    SandboxError::NotReady(format!(
                        "Sandbox not reachable for task {task_id}"
                    ))
                })?;

        Ok(ServerStatusResponse {
            servers: snapshot.servers,
        })
    }

    async fn server_logs(
        &self,
        task_id: &str,
        server_name: &str,
    ) -> Result<BoxStream<'static, Result<Event, SandboxError>>, SandboxError> {
        let service_name = generate_service_name(task_id);
        let namespace = self.k8s.namespace().to_string();
        let server_name = server_name.to_string();

        let stream = health::stream_server_logs(&service_name, &namespace, self.config.sandbox_grpc_port, &server_name)
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
            health::search_sandbox_files(&service_name, self.k8s.namespace(), self.config.sandbox_grpc_port, query, limit)
                .await
                .map_err(SandboxError::Internal)?;

        Ok(results)
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
        let bg_grpc_port = self.config.sandbox_grpc_port;
        let bg_cache = self.status_cache.clone();
        let bg_publisher = self.event_publisher.clone();
        tokio::spawn(async move {
            let checker = BackgroundReadinessChecker {
                k8s: bg_k8s,
                task_id: bg_task_id,
                service_name,
                timeout_secs: bg_timeout,
                sandbox_grpc_port: bg_grpc_port,
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

    /// Create or find existing user secret (updating credentials if found).
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
                if let Some(pod) = pick_best_pod(&list.items) {
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
            pick_best_pod(&list.items).and_then(|p| p.metadata.name.clone())
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
                    health::check_service_health(&service_name, self.k8s.namespace(), self.config.sandbox_grpc_port)
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
                        sandbox_port: self.config.sandbox_grpc_port,
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

    /// Derive server entries from the repository config in the DB.
    ///
    /// Used as a last-resort fallback when gRPC is unreachable and there's
    /// no stale Redis cache. Returns configured servers with `Starting`
    /// status so the frontend shows them while the sandbox boots.
    async fn servers_from_repo_config(&self, task_id: &str) -> Vec<ServerEntry> {
        let Some(ref pool) = self.db_pool else {
            return vec![];
        };

        let repo_servers = match crate::db::task_dao::find_by_id_internal(pool, task_id).await {
            Ok(Some(task)) => {
                match crate::db::repository_dao::find_repository_by_id(pool, &task.repository_id).await {
                    Ok(Some(repo)) => {
                        auto_provision::build_server_configs(&repo)
                    }
                    Ok(None) | Err(_) => return vec![],
                }
            }
            Ok(None) | Err(_) => return vec![],
        };

        repo_servers
            .into_iter()
            .map(|s| {
                let url = self.build_proxy_url(task_id, s.port);
                ServerEntry {
                    label: s.name.clone(),
                    default: s.name == "preview",
                    capabilities: get_server_capabilities(&s.name),
                    name: s.name,
                    port: s.port,
                    status: ServerStatus::Starting,
                    url,
                    error: None,
                }
            })
            .collect()
    }
}

/// Pick the best pod from a list when multiple pods exist for a task.
///
/// During Deployment rolling updates, both old (terminating) and new pods may
/// coexist.  We prefer the non-terminating pod so that status reflects the
/// replacement rather than the dying pod.
///
/// Priority among non-terminating pods: Running > Pending > Unknown > everything else.
/// If ALL pods are terminating, return the first one (genuinely removing).
fn pick_best_pod(pods: &[Pod]) -> Option<&Pod> {
    if pods.is_empty() {
        return None;
    }

    // Split into non-terminating and terminating
    let (alive, terminating): (Vec<_>, Vec<_>) = pods
        .iter()
        .partition(|p| p.metadata.deletion_timestamp.is_none());

    if alive.is_empty() {
        // All pods are terminating — return first (genuinely removing)
        return pods.first();
    }

    // Among alive pods, pick by phase priority
    fn phase_priority(pod: &Pod) -> u8 {
        match pod
            .status
            .as_ref()
            .and_then(|s| s.phase.as_deref())
            .unwrap_or("Unknown")
        {
            "Running" => 0,
            "Pending" => 1,
            "Unknown" => 2,
            _ => 3,
        }
    }

    alive
        .into_iter()
        .min_by_key(|p| phase_priority(p))
        .or(terminating.first().copied())
}

/// Convert `EnvironmentStatus` back to an approximate `PodPhase` for L1 cache compat.
fn env_status_to_pod_phase(status: &EnvironmentStatus) -> PodPhase {
    match status {
        EnvironmentStatus::NotCreated => PodPhase::NotFound,
        EnvironmentStatus::Initializing => PodPhase::Pending,
        EnvironmentStatus::Ready => PodPhase::Running,
        EnvironmentStatus::Stopped => PodPhase::Stopped,
        EnvironmentStatus::Removing => PodPhase::Terminating,
        EnvironmentStatus::Error => PodPhase::Failed,
    }
}
