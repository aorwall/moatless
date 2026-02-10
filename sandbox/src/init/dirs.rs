//! Directory and symlink creation for sandbox initialization.

use std::path::Path;

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

    // Set debug log env var
    unsafe {
        std::env::set_var("CLAUDE_CODE_DEBUG_LOGS_DIR", "/tmp/claude-debug.log");
    }

    info!("Directories initialized");
}

/// Create symlinks for server management scripts.
///
/// The scripts are installed at `/usr/local/bin/` in the Docker image
/// but workspace mounts may shadow paths, so we symlink into
/// `/opt/moatless/bin/` which survives volume mounts.
pub fn ensure_symlinks() {
    let bin_dir = "/opt/moatless/bin";
    let _ = std::fs::create_dir_all(bin_dir);

    let links = [
        ("/usr/local/bin/server", format!("{bin_dir}/server")),
        (
            "/usr/local/bin/file-explorer",
            format!("{bin_dir}/file-explorer"),
        ),
    ];

    for (target, link_path) in links {
        if Path::new(target).exists() && !Path::new(&link_path).exists() {
            if let Err(e) = std::os::unix::fs::symlink(target, &link_path) {
                warn!(target, link = %link_path, error = %e, "Failed to create symlink");
            }
        }
    }
}
