//! [`FleetObserver`] for the local-process runtime: reads the in-memory
//! registry directly, with no external LIST call needed.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;

use moatless_sandbox_runtime::convergence::SandboxObservedStatus;
use moatless_sandbox_runtime::fleet::{FleetObserver, ObserveError, RuntimeObservation};
use moatless_sandbox_runtime::status_reader::SandboxStatusReader;
use moatless_sandbox_runtime::status_reader::confirm;
use moatless_types::sandbox::SandboxDesiredState;
use moatless_types::task::TaskStatus;

use super::client::LocalClient;

/// Local fleet observer: reads every registered task's process liveness from
/// the shared in-memory registry.
pub struct LocalFleetObserver {
    client: LocalClient,
    observer: Arc<dyn SandboxStatusReader>,
}

impl LocalFleetObserver {
    pub fn new(client: LocalClient, observer: Arc<dyn SandboxStatusReader>) -> Self {
        Self { client, observer }
    }
}

#[async_trait]
impl FleetObserver for LocalFleetObserver {
    async fn observe_fleet(&self) -> Result<HashMap<String, RuntimeObservation>, ObserveError> {
        let mut out = HashMap::new();
        for (task_id, snapshot) in self.client.snapshot_all().await {
            let status = if snapshot.alive {
                SandboxObservedStatus::Ready
            } else {
                SandboxObservedStatus::Stopped
            };
            out.insert(
                task_id,
                RuntimeObservation::new(status, snapshot.generation),
            );
        }
        Ok(out)
    }

    async fn observe(
        &self,
        task_id: &str,
        task_status: TaskStatus,
        desired_state: SandboxDesiredState,
    ) -> Result<RuntimeObservation, ObserveError> {
        let status = confirm(&self.observer, task_id, task_status, desired_state).await?;
        let generation = self
            .client
            .snapshot(task_id)
            .await
            .and_then(|snapshot| snapshot.generation);
        Ok(RuntimeObservation::new(status, generation))
    }
}
