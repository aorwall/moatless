//! Install a `gh` shim that routes the agent's `gh` calls through `moat gh`.
//!
//! The shim sits at `<shim_dir>/gh` and `exec`s the `moat` CLI with the same
//! args, which then runs the real `gh` (resolved at sandbox startup and
//! exported as `MOAT_GH_REAL`). The shim is only put on the agent
//! subprocess's PATH (see `agent/handle.rs`) — sandbox-process callers
//! (`init/gh_hosts.rs`, `init/git.rs`) keep their unmodified PATH and hit
//! the real `gh` directly, so there is no risk of recursion.

use std::path::Path;

use tokio::fs;
use tracing::{info, warn};

/// Shim script body. Tiny on purpose — the real logic lives in `moat gh`.
const SHIM_SCRIPT: &str = "#!/usr/bin/env bash\nexec moat gh \"$@\"\n";

/// Write `<shim_dir>/gh` and chmod it executable. Idempotent.
pub async fn install_gh_shim(shim_dir: &str) -> Result<(), String> {
    fs::create_dir_all(shim_dir)
        .await
        .map_err(|e| format!("Failed to create shim dir {shim_dir}: {e}"))?;

    let shim_path = Path::new(shim_dir).join("gh");
    fs::write(&shim_path, SHIM_SCRIPT)
        .await
        .map_err(|e| format!("Failed to write {}: {e}", shim_path.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&shim_path, std::fs::Permissions::from_mode(0o755))
            .await
            .map_err(|e| format!("Failed to chmod {}: {e}", shim_path.display()))?;
    }

    info!(shim = %shim_path.display(), "Installed gh shim → moat gh");
    Ok(())
}

/// Best-effort wrapper that logs failures but never blocks init.
pub async fn install_gh_shim_or_warn(shim_dir: &str) {
    if let Err(e) = install_gh_shim(shim_dir).await {
        warn!(error = %e, shim_dir, "Failed to install gh shim (agent will use real gh directly)");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn writes_shim_with_exec_permission() {
        let tmp = TempDir::new().unwrap();
        let shim_dir = tmp.path().join("shims");
        install_gh_shim(shim_dir.to_str().unwrap()).await.unwrap();

        let path = shim_dir.join("gh");
        let body = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(body.contains("exec moat gh"));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = tokio::fs::metadata(&path).await.unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o755);
        }
    }

    #[tokio::test]
    async fn install_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        let shim_dir = tmp.path().join("shims");
        install_gh_shim(shim_dir.to_str().unwrap()).await.unwrap();
        install_gh_shim(shim_dir.to_str().unwrap()).await.unwrap();
    }
}
