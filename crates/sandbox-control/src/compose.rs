//! The sandbox band's construction root.
//!
//! [`sandbox_graph`] owns the lifecycle store this crate keeps private and
//! hands out only the capabilities built over it: the provisioning knowledge
//! the provider adapters read, the gRPC-service seam onto the `sandboxes` row,
//! the [`Lifecycle`] handle every intent write goes through, and the reconciler
//! a host supervises.

use std::sync::Arc;

use moatless_core::db::DbPool;
use moatless_core::events::EventPublisher;
use moatless_ports::identity::CommitIdentity;
use moatless_ports::task::PendingWorkReader;
use moatless_sandbox_runtime::client::SandboxConnector;
use moatless_sandbox_runtime::fleet::FleetObserver;
use moatless_sandbox_runtime::runtime::SandboxRuntime;
use moatless_workspace_ports::WorkspaceResolver;

use crate::lifecycle::agent_stop::GrpcAgentStop;
use crate::lifecycle::dao::{LifecycleDao, PgLifecycleDao};
use crate::lifecycle::db::lock::PgLifecycleLock;
use crate::lifecycle::event_sink::OutboxEventSink;
use crate::lifecycle::ingress::SandboxIngress;
use crate::lifecycle::orchestrator::{Lifecycle, SystemWallClock};
use crate::lifecycle::plan::ProvisionDeps;
use crate::lifecycle::reconciler::{
    MemberRegistryPort, NotifyPort, Reconciler, ReconcilerConfig, ReconcilerDeps,
};
use crate::lifecycle::workspace_archive::S3WorkspaceArchive;
use crate::lifecycle::workspace_sync::GrpcWorkspaceSync;

/// Bounded lock-acquire attempts before an operation surfaces
/// `OperationInProgress`.
const LIFECYCLE_LOCK_ATTEMPTS: u32 = 3;

/// The lifecycle store, built once over one pool.
///
/// Stays private: a host takes the capabilities below, each built over it.
/// Cloning shares the store, so a host keeping the graph beside its
/// capabilities assembles over the same instance.
#[derive(Clone)]
pub struct SandboxGraph {
    store: Arc<dyn LifecycleDao>,
}

/// Build the sandbox band's store over `db`.
pub fn sandbox_graph(db: DbPool) -> SandboxGraph {
    SandboxGraph {
        store: Arc::new(PgLifecycleDao::new(db)),
    }
}

impl SandboxGraph {
    /// The plan knowledge a provider adapter asks for a fresh provision
    /// request, over the workspace and identity ports the host owns.
    pub fn provision_deps(
        &self,
        workspaces: Arc<dyn WorkspaceResolver>,
        users: Arc<dyn CommitIdentity>,
    ) -> ProvisionDeps {
        ProvisionDeps::new(self.store.clone(), workspaces, users)
    }

    /// The `sandboxes` row as the Sandbox's own gRPC service reads and records
    /// it.
    pub fn sandbox_ingress(&self) -> Arc<SandboxIngress> {
        Arc::new(SandboxIngress::new(self.store.clone()))
    }

    /// The shared durable lifecycle handle, over its Postgres and runtime
    /// adapters. One instance serves the sandbox API, the pod watcher and the
    /// reconciler, so intent writes and per-Task locks stay single-sourced.
    pub fn lifecycle(&self, inputs: LifecycleInputs) -> Arc<Lifecycle> {
        let LifecycleInputs {
            lock_pool,
            runtime,
            connector,
            plan,
            default_sandbox_image,
            events,
            workspace_archive,
        } = inputs;
        let lifecycle = Lifecycle::new(
            self.store.clone(),
            runtime,
            Arc::new(GrpcAgentStop::new(connector.clone())),
            Arc::new(OutboxEventSink::new(events)),
            Arc::new(PgLifecycleLock::new(lock_pool)),
            Arc::new(SystemWallClock),
        )
        .with_workspace_sync(Arc::new(GrpcWorkspaceSync::new(connector)))
        .with_plan_knowledge(plan, default_sandbox_image)
        .with_lock_attempts(LIFECYCLE_LOCK_ATTEMPTS);
        // Without S3 the cold transition stays off: `with_workspace_archive` is
        // what tells lifecycle a durable copy exists to reclaim toward.
        Arc::new(match workspace_archive {
            Some(archive) => lifecycle.with_workspace_archive(archive),
            None => lifecycle,
        })
    }

    /// The desired-state reconciler, over the lifecycle handle it drives
    /// operations through and a fleet observer. The host spawns it with
    /// [`crate::lifecycle::reconciler::spawn`].
    pub fn reconciler(&self, lifecycle: Arc<Lifecycle>, inputs: ReconcilerInputs) -> Reconciler {
        let ReconcilerInputs {
            observer,
            notify,
            pending_work,
            member_registry,
            config,
        } = inputs;
        Reconciler::new(
            (*lifecycle).clone(),
            ReconcilerDeps {
                store: self.store.clone(),
                observer,
                notify,
                pending_work,
                member_registry,
                clock: Arc::new(SystemWallClock),
            },
            config,
        )
    }
}

/// The infrastructure and foreign ports a [`Lifecycle`] is built over, supplied
/// by the host that owns them.
pub struct LifecycleInputs {
    /// The dedicated lock-only pool the per-Task advisory lock is taken on.
    pub lock_pool: DbPool,
    pub runtime: Arc<dyn SandboxRuntime>,
    pub connector: Arc<dyn SandboxConnector>,
    pub plan: ProvisionDeps,
    pub default_sandbox_image: String,
    /// Event publication; `None` without a database.
    pub events: Option<Arc<dyn EventPublisher>>,
    /// The workspace-archive reader the cold transition proves itself against;
    /// `None` without S3, which leaves the transition off.
    pub workspace_archive: Option<Arc<S3WorkspaceArchive>>,
}

/// The foreign ports the desired-state reconciler reads, supplied by the host
/// that owns them.
pub struct ReconcilerInputs {
    pub observer: Arc<dyn FleetObserver>,
    pub notify: Arc<dyn NotifyPort>,
    /// The Task band owns `task_inbox`, so it answers which members of a
    /// shared Sandbox a wake has to reach.
    pub pending_work: Arc<dyn PendingWorkReader>,
    pub member_registry: Arc<dyn MemberRegistryPort>,
    pub config: ReconcilerConfig,
}
