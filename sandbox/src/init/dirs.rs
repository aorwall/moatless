// Safety: set_var is unsafe in multi-threaded context but this module runs
// during single-threaded init phase before any agent execution starts.
#![allow(unsafe_code)]

//! Directory creation for sandbox initialization.

use tracing::{info, warn};

use crate::config::Config;

/// Ensure all required directories exist.
pub fn ensure_directories(config: &Config) {
    let dirs = [
        &config.home_dir,
        &config.workspace_dir,
        &config.data_dir,
        &config.claude_config_dir,
        &format!("{}/debug", config.claude_config_dir),
    ];

    for dir in dirs {
        if let Err(e) = std::fs::create_dir_all(dir) {
            warn!(dir = %dir, error = %e, "Failed to create directory");
        }
    }

    // SAFETY: called during single-threaded init phase before any async
    // workers are spawned. No concurrent threads can observe the env mutation.
    unsafe {
        std::env::set_var("CLAUDE_CODE_DEBUG_LOGS_DIR", "/tmp/claude-debug.log");
    }

    info!("Directories initialized");
}
