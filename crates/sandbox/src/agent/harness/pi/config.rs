use crate::config::Config;
use crate::trajectory_tail::TrajectoryRecording;
use std::sync::Arc;

/// Dependencies fixed for the lifetime of a Pi harness.
#[derive(Clone)]
pub struct PiConfig {
    pub config: Arc<Config>,
    pub trajectory: Option<Arc<TrajectoryRecording>>,
}
