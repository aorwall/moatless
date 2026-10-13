//! Directory creation for sandbox initialization.
//!
//! Everything here is synchronous and recursive (skill copies, `/tmp` cleanup),
//! so it is only ever invoked through the blocking boundary in
//! [`super::run_init_blocking`] — never directly on a Tokio worker.

use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use tracing::{info, warn};

use crate::config::Config;

/// Recursively copy directory contents from `src` to `dst`. Returns the number
/// of files copied.
pub(super) fn copy_dir_contents(src: &Path, dst: &Path) -> std::io::Result<usize> {
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

/// Wipe the temp dir (`TMPDIR`, else `/tmp`) and re-apply 1777 permissions.
///
/// In a pod `/tmp` is a PVC subPath that K8s recreates at 0755, which breaks
/// tools that drop privileges. A local sandbox shares the host's `/tmp`, so
/// its provider points `TMPDIR` at a per-task directory.
fn reset_tmp() {
    let tmp_dir = std::env::temp_dir();
    let tmp = tmp_dir.as_path();
    match std::fs::read_dir(tmp) {
        Ok(entries) => {
            let mut removed = 0usize;
            for entry in entries.flatten() {
                let path = entry.path();
                let result = if entry.file_type().is_ok_and(|t| t.is_dir()) {
                    std::fs::remove_dir_all(&path)
                } else {
                    std::fs::remove_file(&path)
                };
                match result {
                    Ok(()) => removed += 1,
                    Err(e) => {
                        warn!(path = %path.display(), error = %e, "Failed to clean /tmp entry");
                    }
                }
            }
            if removed > 0 {
                info!(removed, "Cleaned leftover /tmp entries");
            }
        }
        Err(e) => warn!(error = %e, "Failed to read /tmp for cleanup"),
    }

    if let Err(e) = std::fs::set_permissions(tmp, std::fs::Permissions::from_mode(0o1777)) {
        warn!(error = %e, "Failed to chmod /tmp to 1777");
    }
}

/// Ensure all required directories exist.
///
/// Blocking and recursive — call it through [`super::run_init_blocking`].
pub(super) fn ensure_directories(config: &Config) {
    reset_tmp();

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

    // `CLAUDE_CODE_DEBUG_LOGS_DIR` is deliberately NOT set here: this runs
    // on the blocking pool with the runtime up, where `set_var` is unsound.
    // It's set once in `main` before the runtime builds (covers K8s, which
    // overrides the image entrypoint); `docker-entrypoint-sandbox.sh` still
    // exports it for image-entrypoint launches.

    info!("Directories initialized");
}
