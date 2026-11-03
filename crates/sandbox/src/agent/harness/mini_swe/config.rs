use crate::config::Config;
use crate::trajectory_tail::TrajectoryRecording;
use std::sync::Arc;

/// Dependencies fixed for the lifetime of a mini-swe-agent harness.
#[derive(Clone)]
pub struct MiniSweConfig {
    pub config: Arc<Config>,
    pub trajectory: Option<Arc<TrajectoryRecording>>,
}
