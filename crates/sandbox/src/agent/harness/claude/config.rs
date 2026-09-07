use crate::agent::agent_state::AgentState;
use crate::config::Config;
use std::sync::Arc;
/// Shared configuration for the lifetime of a Claude harness.
#[derive(Clone)]
pub struct ClaudeConfig {
    pub config: Arc<Config>,
    pub agent_state: Arc<AgentState>,
    pub agent_type: Option<String>,
    /// Publishes the interactive calls the permission handler parks on. `None`
    /// in tests, which drive the harness without a backend to publish to.
    pub events: Option<crate::events::GrpcEventPublisher>,
}
