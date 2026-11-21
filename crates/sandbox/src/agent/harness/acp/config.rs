use crate::config::Config;
use crate::trajectory_tail::TrajectoryRecording;
use moatless_types::agent::AgentProfile;
use std::sync::Arc;

/// Dependencies fixed for the lifetime of an ACP harness.
#[derive(Clone)]
pub struct AcpConfig {
    pub config: Arc<Config>,
    pub trajectory: Option<Arc<TrajectoryRecording>>,
    /// The profile naming which ACP agent to run.
    pub profile: AgentProfile,
}
