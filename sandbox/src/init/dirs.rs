// Safety: set_var is unsafe in multi-threaded context but this module runs
// during single-threaded init phase before any agent execution starts.
#![allow(unsafe_code)]

//! Directory creation for sandbox initialization.

use std::path::Path;

use tracing::{info, warn};

use crate::config::Config;

/// Recursively copy directory contents from `src` to `dst`.
/// Returns the number of files copied.
fn copy_dir_contents(src: &Path, dst: &Path) -> std::io::Result<usize> {
    let mut count = 0;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if src_path.is_dir() {
            std::fs::create_dir_all(&dst_path)?;
            count += copy_dir_contents(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)?;
            count += 1;
        }
    }
    Ok(count)
}

/// Ensure all required directories exist.
pub fn ensure_directories(config: &Config) {
    let dirs = [
        &config.home_dir,
        &config.codex_home_dir,
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

    // Copy staged skills from image into config dir (survives volume mounts).
    // Downstream Dockerfiles COPY skills to /usr/share/moatless-skills/ in the image;
    // we copy them here so they land inside the mounted volume.
    let staged_skills = std::path::Path::new("/usr/share/moatless-skills");
    if staged_skills.is_dir() {
        let target_skills = format!("{}/skills", config.claude_config_dir);
        if let Err(e) = std::fs::create_dir_all(&target_skills) {
            warn!(error = %e, "Failed to create skills directory");
        } else {
            match copy_dir_contents(staged_skills, std::path::Path::new(&target_skills)) {
                Ok(0) => {}
                Ok(count) => info!(count, "Copied staged skills into config dir"),
                Err(e) => warn!(error = %e, "Failed to copy staged skills"),
            }
        }

        let codex_skills = format!("{}/skills", config.codex_home_dir);
        if let Err(e) = std::fs::create_dir_all(&codex_skills) {
            warn!(error = %e, "Failed to create Codex skills directory");
        } else {
            match copy_dir_contents(staged_skills, std::path::Path::new(&codex_skills)) {
                Ok(0) => {}
                Ok(count) => info!(count, "Copied staged skills into Codex home"),
                Err(e) => warn!(error = %e, "Failed to copy staged Codex skills"),
            }
        }
    }

    // SAFETY: called during single-threaded init phase before any async
    // workers are spawned. No concurrent threads can observe the env mutation.
    unsafe {
        std::env::set_var("CLAUDE_CODE_DEBUG_LOGS_DIR", "/tmp/claude-debug.log");
    }

    info!("Directories initialized");
}
