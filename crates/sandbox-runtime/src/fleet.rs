//! What the reconciler asks a runtime for, and what a runtime answers with.
//!
//! The reconciler itself lives in the orchestrator band. What crosses to a
//! runtime is one batch read of the fleet, so that is what is declared here.

use std::collections::HashMap;

use async_trait::async_trait;

use moatless_types::sandbox::SandboxDesiredState;
use moatless_types::task::TaskStatus;

use crate::convergence::SandboxObservedStatus;
use crate::operation::OperationGeneration;

/// Runtime observation paired with the lifecycle generation stamped on the
/// currently observed pod/container. Legacy resources have no generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeObservation {
    pub status: SandboxObservedStatus,
    pub generation: Option<OperationGeneration>,
}

impl RuntimeObservation {
    pub fn new(status: SandboxObservedStatus, generation: Option<OperationGeneration>) -> Self {
        Self { status, generation }
    }
}

impl From<SandboxObservedStatus> for RuntimeObservation {
    fn from(status: SandboxObservedStatus) -> Self {
        Self::new(status, None)
    }
}

/// A runtime-status read failed. The reconciler logs and skips the Task this
/// tick rather than acting on missing observation. Keeps the underlying source
/// (a K8s/Docker observation error) so fleet-LIST debugging retains the chain.
#[derive(Debug, thiserror::Error)]
#[error("{context}")]
pub struct ObserveError {
    pub context: String,
    #[source]
    pub source: Option<Box<dyn std::error::Error + Send + Sync + 'static>>,
}

impl ObserveError {
    pub fn new(context: impl Into<String>) -> Self {
        Self {
            context: context.into(),
            source: None,
        }
    }

    pub fn with_source(
        context: impl Into<String>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self {
            context: context.into(),
            source: Some(Box::new(source)),
        }
    }
}

/// The reconciler's runtime-status source, batch-first.
#[async_trait]
pub trait FleetObserver: Send + Sync {
    /// One label-selector LIST per tick: coarse observed status for every Task
    /// with live resources. Tasks absent from the map have no resources and are
    /// treated as [`SandboxObservedStatus::Absent`].
    async fn observe_fleet(&self) -> Result<HashMap<String, RuntimeObservation>, ObserveError>;

    /// Authoritative per-Task status, read to confirm a divergence before acting
    /// so the reconciler never drives an operation off stale coarse data. The
    /// intent fields are the ones the observer's live-status projection needs.
    async fn observe(
        &self,
        task_id: &str,
        task_status: TaskStatus,
        desired_state: SandboxDesiredState,
    ) -> Result<RuntimeObservation, ObserveError>;
}
