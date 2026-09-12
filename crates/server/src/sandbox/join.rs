//! Attach a task to an existing sandbox through its gRPC connector.

use moatless_ports::sandbox::{SandboxError, SandboxJoiner};
use moatless_types::agent::AgentType;
use moatless_types::task::CheckoutMode;
use sandbox_proto::client::{SandboxConnector, classify_grpc_error};
use std::sync::Arc;

pub(crate) struct ConnectorSandboxJoiner {
    connector: Arc<dyn SandboxConnector>,
}

#[async_trait::async_trait]
impl SandboxJoiner for ConnectorSandboxJoiner {
    async fn join_task(
        &self,
        task_id: &str,
        checkout_mode: CheckoutMode,
        agent_type: &AgentType,
    ) -> Result<(), SandboxError> {
        let client = self.connector.sandbox_client(task_id).await?;
        client
            .join_task(
                task_id,
                Some(checkout_mode.as_str()),
                Some(agent_type.as_str()),
            )
            .await
            .map_err(|error| classify_grpc_error(error, "JoinTask"))
    }
}

impl ConnectorSandboxJoiner {
    pub(crate) fn new(connector: Arc<dyn SandboxConnector>) -> Self {
        Self { connector }
    }
}
