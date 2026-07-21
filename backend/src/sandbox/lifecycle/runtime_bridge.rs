//! [`SandboxRuntime`] implemented over the [`SandboxProvider`].
//!
//! The bridge adapts the provider's imperative surface into the typed
//! `RuntimeOutcome` the lifecycle seam needs. It owns plan construction (the
//! provision repositories live behind lifecycle) and derives the effect axis at
//! the adapter from an observed-status read taken under the per-Task lock —
//! never inferred by the orchestrator. Removal is strict: it reports success
//! only on observed absence of every resource and durable archive deletion.

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
use super::outcome::{DegradationWarning, Diagnostics};
use super::runtime::{RuntimeOperationContext, RuntimeOutcome, SandboxRuntime};
use super::types::{ArchiveRetention, ObservedStatus, RuntimeEffect};

/// Poll bound for strict-remove absence: a delete is only success once every
/// runtime resource is observed absent.
const REMOVE_ABSENCE_POLLS: u32 = 45;
const REMOVE_ABSENCE_INTERVAL: Duration = Duration::from_secs(2);

/// The repositories [`build_provision_request`] reads to assemble a fresh plan.
///
/// Bundled into one dependency so the bridge — and future native adapters —
/// take a single value instead of five positional `Arc`s.
#[derive(Clone)]
pub struct ProvisionDeps {
    pub tasks: Arc<dyn TaskRepository>,
    pub repos: Arc<dyn RepoRepository>,
    pub provider_tokens: Arc<dyn ProviderTokenRepository>,
    pub plugin_repos: Arc<dyn PluginRepoRepository>,
    pub workspaces: Arc<dyn WorkspaceRepository>,
}

/// Bridges the lifecycle runtime seam onto the provider plus plan construction.
pub struct RuntimeBridge {
    provider: Arc<dyn SandboxProvider>,
    plan: ProvisionDeps,
}

impl RuntimeBridge {
    pub fn new(provider: Arc<dyn SandboxProvider>, plan: ProvisionDeps) -> Self {
        Self { provider, plan }
    }

    /// The infrastructure status observed right now, under the operation's lock.
    async fn observed(&self, task_id: &str) -> Result<SandboxStatus, RuntimeError> {
        let task = self
            .plan
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
            self.plan.tasks.as_ref(),
            self.plan.repos.as_ref(),
            self.plan.provider_tokens.as_ref(),
            self.plan.plugin_repos.as_ref(),
            self.plan.workspaces.as_ref(),
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
impl SandboxRuntime for RuntimeBridge {
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
        // Reporting `Starting` on an observation failure is the correct floor for
        // a just-issued restart, but the failure itself is not swallowed: it is
        // logged and surfaced as a diagnostic.
        let (observed, diagnostics) = match self.observed(task_id).await {
            Ok(status) => (to_running_observed(status), Diagnostics::default()),
            Err(e) => {
                tracing::warn!(task_id, error = %e, "failed to observe status after restart; reporting Starting");
                let diagnostics = Diagnostics {
                    degradations: vec![DegradationWarning {
                        code: "restart_observation_failed".into(),
                        message: e.to_string(),
                    }],
                    server_failures: vec![],
                };
                (ObservedStatus::Starting, diagnostics)
            }
        };
        Ok(RuntimeOutcome::applied(observed).with_diagnostics(diagnostics))
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
        // `cleanup` is strict: it awaits pod + PVC absence and, on a hard delete,
        // durable archive deletion, surfacing any failure as a typed error.
        self.provider
            .cleanup(task_id, retention.retains())
            .await
            .map_err(map_runtime_err)?;
        self.wait_for_absence(task_id).await?;
        // The effect reflects what cleanup did, not the runtime pre-read alone: a
        // `DeleteArchives` remove always performs S3 deletion, so it is `Applied`
        // even when the runtime was already gone. Only a retention-preserving
        // remove against already-absent resources did nothing.
        let effect = if is_absent(pre) && retention.retains() {
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

/// Exercises the real plan path against local Postgres: a bad plan surfaces as a
/// permanent `Rejected`, not a transient. Gated on `db-tests`.
#[cfg(all(test, feature = "db-tests"))]
mod db_tests {
    use super::super::error::ErrorClass;
    use super::super::runtime::RuntimeOperationContext;
    use super::super::test_support::seed_task_with;
    use super::super::types::OperationGeneration;
    use super::*;

    use axum::response::sse::Event;
    use futures::stream::BoxStream;
    use moatless_events::SandboxStatus;
    use sqlx::PgPool;

    use crate::auth::provider_token_dao::PgProviderTokenRepository;
    use crate::plugin_repo::dao::PgPluginRepoRepository;
    use crate::repository::dao::PgRepoRepository;
    use crate::sandbox::client::SandboxClient;
    use crate::sandbox::live_status::LiveStatus;
    use crate::sandbox::models::{ContainerObservedStatus, ServerConfig};
    use crate::sandbox::schemas::{ProvisionResponse, SandboxStatusFullResponse};
    use crate::task::dao::PgTaskRepository;
    use crate::task::models::{SandboxDesiredState, TaskStatus};
    use crate::workspace::dao::PgWorkspaceRepository;

    /// The plan is never reached past `observed()`, which only needs a canned
    /// `live_status`; every mutation is unused here.
    struct NoopProvider;

    #[async_trait]
    impl SandboxProvider for NoopProvider {
        async fn provision(
            &self,
            _t: &str,
            _r: &ProvisionRequest,
        ) -> Result<ProvisionResponse, SandboxError> {
            unimplemented!()
        }
        async fn sandbox_status(
            &self,
            _t: &str,
            _s: &[ServerConfig],
            _ts: TaskStatus,
            _d: SandboxDesiredState,
        ) -> Result<SandboxStatusFullResponse, SandboxError> {
            unimplemented!()
        }
        async fn live_status(
            &self,
            _t: &str,
            _ts: TaskStatus,
            _d: SandboxDesiredState,
        ) -> Result<LiveStatus, SandboxError> {
            Ok(LiveStatus {
                sandbox_status: SandboxStatus::NotCreated,
                agent_status: None,
                sandbox_error: None,
            })
        }
        fn expected_sandbox_image(&self, _r: Option<&str>) -> String {
            String::new()
        }
        async fn stop(&self, _t: &str) -> Result<(), SandboxError> {
            unimplemented!()
        }
        async fn restart(&self, _t: &str) -> Result<(), SandboxError> {
            unimplemented!()
        }
        async fn redeploy(
            &self,
            _t: &str,
            _r: &ProvisionRequest,
        ) -> Result<ProvisionResponse, SandboxError> {
            unimplemented!()
        }
        async fn cleanup(&self, _t: &str, _r: bool) -> Result<(), SandboxError> {
            unimplemented!()
        }
        async fn container_statuses(
            &self,
            _t: &str,
        ) -> Result<Vec<ContainerObservedStatus>, SandboxError> {
            unimplemented!()
        }
        async fn server_logs(
            &self,
            _t: &str,
            _s: &str,
            _p: bool,
        ) -> Result<BoxStream<'static, Result<Event, SandboxError>>, SandboxError> {
            unimplemented!()
        }
        async fn sandbox_client(&self, _t: &str) -> Result<SandboxClient, SandboxError> {
            unimplemented!()
        }
    }

    fn bridge(pool: PgPool) -> RuntimeBridge {
        RuntimeBridge::new(
            Arc::new(NoopProvider),
            ProvisionDeps {
                tasks: Arc::new(PgTaskRepository(pool.clone())),
                repos: Arc::new(PgRepoRepository(pool.clone())),
                provider_tokens: Arc::new(PgProviderTokenRepository(pool.clone())),
                plugin_repos: Arc::new(PgPluginRepoRepository(pool.clone())),
                workspaces: Arc::new(PgWorkspaceRepository(pool)),
            },
        )
    }

    #[sqlx::test]
    async fn bad_plan_start_is_permanent_rejected(pool: PgPool) {
        // A Closed task's plan is refused by `build_provision_request`, so the
        // real plan path surfaces a permanent `Rejected` — not a transient — and
        // the orchestrator therefore keeps intent without a reconciler retry.
        seed_task_with(&pool, "t1", "closed", "running").await;
        let bridge = bridge(pool);

        let err = bridge
            .ensure_running("t1", RuntimeOperationContext::new(OperationGeneration(1)))
            .await
            .expect_err("closed task plan is rejected");

        assert!(matches!(err, RuntimeError::Rejected(_)), "got {err:?}");
        assert_eq!(err.class(), ErrorClass::Permanent);
    }
}
