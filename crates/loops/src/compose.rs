//! The loops band's construction root.
//!
//! [`loop_graph`] owns every store this crate keeps private, and hands out only
//! independent capabilities and complete handles over them, so a host never
//! holds a store and never re-wires a service after the fact.

use std::sync::Arc;

use moatless_adapter_model::capabilities::AdapterDestinationBuilder;
use moatless_adapter_model::connections::{AdapterConnections, ConnectionRouting};
use moatless_adapter_model::delivery::DeliveryAdmission;
use moatless_adapter_model::loops::LoopReader;
use moatless_adapter_model::registry::Adapters;
use moatless_core::db::DbPool;
use moatless_core::file_storage::Files;
use moatless_ports::identity::{GitHostTokenIssuer, UserDisplay};
use moatless_ports::sandbox::SandboxWaker;
use moatless_ports::task::TagResolver;
use moatless_ports::task::{
    MessageAcceptor, TaskOpener, TaskReader, TranscriptAppend, TurnCounter,
};
use moatless_ports::workspace::WorkspaceResolver;
use moatless_types::auth::AuthConfig;

use crate::adapters::delivery::AdapterDeliveryWorker;
use crate::adapters::delivery::dao::AdapterDeliveryStore;
use crate::bindings::connect::Connector;
use crate::bindings::dao::PgTaskBindingDao;
use crate::bindings::operations::Bindings;
use crate::bindings::outbound::OutboundSender;
use crate::bindings::ports::BindingDirectory;
use crate::bindings::recently_sent::RecentlySent;
use crate::bindings::reply::ExternalReplyService;
use crate::bindings::runtime::RoutingService;
use crate::loops::dao::LoopDao;
use crate::loops::firing::receipts_dao::EventReceipts;
use crate::loops::firing::schedule_runner::ScheduleRunner;
use crate::loops::operations::Loops;
use crate::loops::sync::LoopSyncService;

/// Every store the loops band owns, built once over one pool.
///
/// The stores stay private: a host takes the capabilities and handles below.
/// Cloning shares every store, so a host that keeps the graph builds later
/// services over the same instances.
#[derive(Clone)]
pub struct LoopGraph {
    loops: Arc<LoopDao>,
    bindings: Arc<Bindings>,
    receipts: Arc<EventReceipts>,
    delivery: Arc<AdapterDeliveryStore>,
}

/// Build the loops band's stores over `db`.
pub fn loop_graph(db: DbPool) -> LoopGraph {
    LoopGraph {
        loops: Arc::new(LoopDao::new(db.clone())),
        bindings: Arc::new(Bindings::new(Arc::new(PgTaskBindingDao::new(db.clone())))),
        receipts: Arc::new(EventReceipts::new(db.clone())),
        delivery: Arc::new(AdapterDeliveryStore::new(db)),
    }
}

impl LoopGraph {
    /// The four Loop tables, as a host reads and writes them.
    pub fn loops(&self) -> Arc<Loops> {
        Arc::new(Loops::new(self.loops.clone()))
    }

    /// The Subscription reads an Adapter's webhook route makes.
    pub fn loop_reader(&self) -> Arc<dyn LoopReader> {
        self.loops.clone()
    }

    /// The `task_bindings` table behind the two seams a crate that does not
    /// drive the Binding lifecycle reads: `BindingProbe` and `TaskBindings`.
    ///
    /// Built before the Task band assembles, which reads it as
    /// `TaskWorkflowInputs::task_bindings`.
    pub fn binding_directory(&self) -> Arc<BindingDirectory> {
        Arc::new(BindingDirectory::new(
            self.bindings.clone(),
            self.loops.clone(),
        ))
    }

    /// The Binding lifecycle, as the Adapters that drive it read and write it.
    pub fn bindings(&self) -> Arc<Bindings> {
        self.bindings.clone()
    }

    /// Durable admission for every inbound Adapter delivery.
    pub fn delivery_admission(&self) -> Arc<dyn DeliveryAdmission> {
        self.delivery.clone()
    }

    /// The worker that owns admitted deliveries. A host starts it last, once
    /// every delivery handler is registered.
    pub fn delivery_worker(&self) -> AdapterDeliveryWorker {
        AdapterDeliveryWorker::new(self.delivery.clone())
    }

    /// Reconciles a repository's git-declared Loops, over the four ports the
    /// reconcile tree reads.
    pub fn loop_sync(
        &self,
        adapter_connections: Arc<ConnectionRouting>,
        tags: Arc<dyn TagResolver>,
        workspaces: Arc<dyn WorkspaceResolver>,
        git_host_tokens: Arc<dyn GitHostTokenIssuer>,
    ) -> Arc<LoopSyncService> {
        Arc::new(LoopSyncService::new(
            adapter_connections,
            self.loops.clone(),
            tags,
            workspaces,
            git_host_tokens,
        ))
    }

    /// The durable schedule-firing runner. The caller owns the join handle
    /// [`ScheduleRunner::spawn`] returns.
    pub fn schedule_runner(&self, inputs: ScheduleRunnerInputs) -> ScheduleRunner {
        ScheduleRunner::new(
            self.loops.clone(),
            inputs.tasks,
            inputs.message_send,
            inputs.adapters,
            inputs.task_creation,
            inputs.sandbox_waker,
        )
    }

    /// The inbound-routing subgraph, over the Binding, Loop and receipt stores
    /// and the foreign ports it reads.
    pub fn routing(&self, inputs: RoutingInputs) -> Arc<RoutingService> {
        Arc::new(RoutingService::new(
            inputs.adapter_connections,
            self.loops.clone(),
            self.bindings.clone(),
            inputs.messages,
            inputs.tasks,
            inputs.task_creation,
            inputs.message_send,
            self.receipts.clone(),
            inputs.sandbox_waker,
            inputs.adapters,
        ))
    }

    /// Binding-based outbound delivery. Held by the reply service below, which
    /// is its only consumer.
    pub fn outbound_sender(&self, inputs: OutboundSenderInputs) -> Arc<OutboundSender> {
        Arc::new(OutboundSender::new(
            self.bindings.clone(),
            inputs.adapter_connections,
            self.loops.clone(),
            inputs.tasks,
            inputs.recently_sent,
            inputs.messages,
            inputs.task_turns,
            inputs.auth_config,
            inputs.files,
            inputs.workspaces,
            inputs.users,
        ))
    }

    /// Reply delivery for external threads, over the outbound sender it owns.
    pub fn external_reply(&self, inputs: ExternalReplyInputs) -> Arc<ExternalReplyService> {
        Arc::new(ExternalReplyService::new(
            inputs.adapters,
            self.bindings.clone(),
            inputs.adapter_connections,
            inputs.connection_routing,
            inputs.messages,
            inputs.tasks,
            self.loops.clone(),
            inputs.recently_sent,
            inputs.task_turns,
            inputs.outbound,
        ))
    }

    /// Wires a task to an external Adapter subject.
    pub fn connector(
        &self,
        github_destination: Option<Arc<dyn AdapterDestinationBuilder>>,
        adapter_connections: Arc<ConnectionRouting>,
    ) -> Connector {
        Connector::new(
            github_destination,
            adapter_connections,
            self.loop_reader(),
            self.bindings.clone(),
        )
    }
}

/// The foreign ports the schedule runner fires a Loop through.
pub struct ScheduleRunnerInputs {
    pub tasks: Arc<dyn TaskReader>,
    pub message_send: Arc<dyn MessageAcceptor>,
    pub adapters: Option<Arc<Adapters>>,
    pub task_creation: Arc<dyn TaskOpener>,
    pub sandbox_waker: Arc<dyn SandboxWaker>,
}

/// The foreign ports the inbound-routing subgraph reads.
pub struct RoutingInputs {
    pub adapter_connections: Arc<ConnectionRouting>,
    pub messages: Arc<dyn TranscriptAppend>,
    pub tasks: Arc<dyn TaskReader>,
    pub task_creation: Arc<dyn TaskOpener>,
    pub message_send: Arc<dyn MessageAcceptor>,
    pub sandbox_waker: Arc<dyn SandboxWaker>,
    pub adapters: Option<Arc<Adapters>>,
}

/// The foreign ports the outbound send path reads.
pub struct OutboundSenderInputs {
    pub adapter_connections: Arc<ConnectionRouting>,
    pub tasks: Arc<dyn TaskReader>,
    pub recently_sent: Arc<RecentlySent>,
    pub messages: Arc<dyn TranscriptAppend>,
    pub task_turns: Arc<dyn TurnCounter>,
    pub auth_config: Arc<AuthConfig>,
    pub files: Arc<Files>,
    pub workspaces: Arc<dyn WorkspaceResolver>,
    pub users: Arc<dyn UserDisplay>,
}

/// The foreign ports the reply paths read, plus the outbound sender they
/// delegate the send itself to.
pub struct ExternalReplyInputs {
    pub adapters: Option<Arc<Adapters>>,
    pub adapter_connections: Arc<AdapterConnections>,
    pub connection_routing: Arc<ConnectionRouting>,
    pub messages: Arc<dyn TranscriptAppend>,
    pub tasks: Arc<dyn TaskReader>,
    pub recently_sent: Arc<RecentlySent>,
    pub task_turns: Arc<dyn TurnCounter>,
    pub outbound: Arc<OutboundSender>,
}
