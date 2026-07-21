//! [`SandboxRuntime`] over the existing [`SandboxProvider`].
//!
//! Step 2's runtime seam: the K8s and Docker providers are not yet native
//! `SandboxRuntime` implementations (that is step 3), so this bridge adapts the
//! provider's imperative surface into the typed `RuntimeOutcome` lifecycle needs.
//! It owns plan construction (moved behind lifecycle) and derives the effect axis
//! at the adapter from an observed-status read taken under the per-Task lock —
//! never inferred by the orchestrator. Removal is strict: it awaits observed
//! absence before reporting success.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use moatless_events::SandboxStatus;

use crate::auth::provider_token_dao::ProviderTokenRepository;
use crate::plugin_repo::dao::PluginRepoRepository;
use crate::repository::dao::RepoRepository;
use crate::sandbox::auto_provision::build_provision_request;
use crate::sandbox::error::SandboxError;
use crate::sandbox::models::ProvisionRequest;
use crate::sandbox::provider::SandboxProvider;
use crate::task::dao::TaskRepository;
use crate::workspace::dao::WorkspaceRepository;

use super::error::RuntimeError;
use super::runtime::{RuntimeOperationContext, RuntimeOutcome, SandboxRuntime};
use super::types::{ArchiveRetention, ObservedStatus, RuntimeEffect};

/// Poll bound for strict-remove absence: a delete is only success once every
/// runtime resource is observed absent.
const REMOVE_ABSENCE_POLLS: u32 = 45;
const REMOVE_ABSENCE_INTERVAL: Duration = Duration::from_secs(2);

/// Bridges the lifecycle runtime seam onto the provider plus plan construction.
pub struct ProviderRuntime {
    provider: Arc<dyn SandboxProvider>,
    tasks: Arc<dyn TaskRepository>,
    repos: Arc<dyn RepoRepository>,
    provider_tokens: Arc<dyn ProviderTokenRepository>,
    plugin_repos: Arc<dyn PluginRepoRepository>,
    workspaces: Arc<dyn WorkspaceRepository>,
}

impl ProviderRuntime {
    pub fn new(
        provider: Arc<dyn SandboxProvider>,
        tasks: Arc<dyn TaskRepository>,
        repos: Arc<dyn RepoRepository>,
        provider_tokens: Arc<dyn ProviderTokenRepository>,
        plugin_repos: Arc<dyn PluginRepoRepository>,
        workspaces: Arc<dyn WorkspaceRepository>,
    ) -> Self {
        Self {
            provider,
            tasks,
            repos,
            provider_tokens,
            plugin_repos,
            workspaces,
        }
    }

    /// The infrastructure status observed right now, under the operation's lock.
    async fn observed(&self, task_id: &str) -> Result<SandboxStatus, RuntimeError> {
        let task = self
            .tasks
            .find_by_id_internal(task_id)
            .await
            .map_err(|e| RuntimeError::Infrastructure(e.to_string()))?
            .ok_or_else(|| RuntimeError::Infrastructure(format!("task {task_id} not found")))?;
        let live = self
            .provider
            .live_status(task_id, task.status, task.sandbox_desired_state)
            .await
            .map_err(map_runtime_err)?;
        Ok(live.sandbox_status)
    }

    /// Construct the fresh provision plan (plan repositories live behind
    /// lifecycle).
    async fn build_request(&self, task_id: &str) -> Result<ProvisionRequest, RuntimeError> {
        build_provision_request(
            self.tasks.as_ref(),
            self.repos.as_ref(),
            self.provider_tokens.as_ref(),
            self.plugin_repos.as_ref(),
            self.workspaces.as_ref(),
            task_id,
        )
        .await
        .map_err(map_runtime_err)
    }

    /// Await observed absence of every runtime resource. Accepted-but-not-absent
    /// is not success; a delete that never drains surfaces `NotAbsent`.
    async fn wait_for_absence(&self, task_id: &str) -> Result<(), RuntimeError> {
        for _ in 0..REMOVE_ABSENCE_POLLS {
            if is_absent(self.observed(task_id).await?) {
                return Ok(());
            }
            tokio::time::sleep(REMOVE_ABSENCE_INTERVAL).await;
        }
        Err(RuntimeError::NotAbsent(format!(
            "resources for {task_id} still present after removal"
        )))
    }
}

#[async_trait]
impl SandboxRuntime for ProviderRuntime {
    async fn ensure_running(
        &self,
        task_id: &str,
        _ctx: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, RuntimeError> {
        let pre = self.observed(task_id).await?;
        let request = self.build_request(task_id).await?;
        let resp = self
            .provider
            .provision(task_id, &request)
            .await
            .map_err(map_runtime_err)?;
        // Reusing an already-ready sandbox is a no-op; anything else did work.
        let effect = if pre == SandboxStatus::Ready && resp.status == SandboxStatus::Ready {
            RuntimeEffect::NoOp
        } else {
            RuntimeEffect::Applied
        };
        Ok(RuntimeOutcome {
            observed: to_running_observed(resp.status),
            effect,
            diagnostics: Default::default(),
        })
    }

    async fn stop(
        &self,
        task_id: &str,
        _ctx: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, RuntimeError> {
        let pre = self.observed(task_id).await?;
        self.provider.stop(task_id).await.map_err(map_runtime_err)?;
        let effect = if is_stopped_or_absent(pre) {
            RuntimeEffect::NoOp
        } else {
            RuntimeEffect::Applied
        };
        Ok(RuntimeOutcome {
            observed: ObservedStatus::Stopped,
            effect,
            diagnostics: Default::default(),
        })
    }

    async fn restart(
        &self,
        task_id: &str,
        _ctx: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, RuntimeError> {
        // Edges deliberately supersede — always an applied effect.
        self.provider.restart(task_id).await.map_err(map_runtime_err)?;
        let observed = to_running_observed(self.observed(task_id).await.unwrap_or(SandboxStatus::Initializing));
        Ok(RuntimeOutcome::applied(observed))
    }

    async fn redeploy(
        &self,
        task_id: &str,
        _ctx: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, RuntimeError> {
        let request = self.build_request(task_id).await?;
        let resp = self
            .provider
            .redeploy(task_id, &request)
            .await
            .map_err(map_runtime_err)?;
        Ok(RuntimeOutcome::applied(to_running_observed(resp.status)))
    }

    async fn remove(
        &self,
        task_id: &str,
        retention: ArchiveRetention,
        _ctx: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, RuntimeError> {
        let pre = self.observed(task_id).await?;
        // `retain_storage = true` keeps S3 archives; runtime resources always go.
        self.provider
            .cleanup(task_id, retention.retains())
            .await
            .map_err(map_runtime_err)?;
        self.wait_for_absence(task_id).await?;
        let effect = if is_absent(pre) {
            RuntimeEffect::NoOp
        } else {
            RuntimeEffect::Applied
        };
        Ok(RuntimeOutcome {
            observed: ObservedStatus::Removed,
            effect,
            diagnostics: Default::default(),
        })
    }
}

fn is_absent(status: SandboxStatus) -> bool {
    matches!(status, SandboxStatus::NotCreated | SandboxStatus::Removed)
}

fn is_stopped_or_absent(status: SandboxStatus) -> bool {
    matches!(
        status,
        SandboxStatus::Stopped | SandboxStatus::NotCreated | SandboxStatus::Removed
    )
}

/// The observed status a toward-running operation reports. Providers differ:
/// Kubernetes returns `Initializing` (→ Starting), Docker completes and returns
/// `Ready`.
fn to_running_observed(status: SandboxStatus) -> ObservedStatus {
    match status {
        SandboxStatus::Ready => ObservedStatus::Ready,
        _ => ObservedStatus::Starting,
    }
}

/// Map a provider error onto the classifiable runtime error. An unbuildable plan
/// or misconfiguration is permanent; infrastructure faults are transient.
fn map_runtime_err(e: SandboxError) -> RuntimeError {
    match e {
        SandboxError::BadRequest(m)
        | SandboxError::ConfigMissing(m)
        | SandboxError::PodBuildError(m)
        | SandboxError::Forbidden(m) => RuntimeError::Rejected(m),
        SandboxError::JwtError(m)
        | SandboxError::DockerError(m)
        | SandboxError::NotReady(m)
        | SandboxError::NotFound(m)
        | SandboxError::Internal(m) => RuntimeError::Infrastructure(m),
        SandboxError::KubeError(e) => RuntimeError::Infrastructure(e.to_string()),
    }
}
