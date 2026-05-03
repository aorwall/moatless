//! Workspace setup: node_modules, build cache, setup commands.

use std::path::Path;

use tokio::fs;
use tokio::process::Command;
use tracing::{info, warn};

use crate::config::Config;

/// Sync plugins from backend API.
pub async fn sync_plugins(config: &Config) -> Result<usize, String> {
    let count = config.plugin_ids.len();
    if count == 0 {
        return Ok(0);
    }

    info!(count, "Syncing plugins from backend API");
    let script = "/usr/local/bin/sync-plugins.sh";
    if Path::new(script).exists() {
        let output = Command::new(script)
            .output()
            .await
            .map_err(|e| format!("Failed to run sync-plugins.sh: {e}"))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("sync-plugins.sh failed: {stderr}"));
        }
    } else {
        warn!("sync-plugins.sh not found, skipping plugin sync");
    }
    Ok(count)
}

/// Clone plugin git repos into `/opt/moatless/plugins/{name}/`.
///
/// For each URL: derive name from last path segment (minus `.git`),
/// clone with `git clone --depth 1`, use `GIT_TOKEN` for auth.
/// Skip if already cloned. Non-fatal on failure.
/// Returns list of cloned directory paths.
pub async fn clone_plugin_repos(config: &Config) -> Vec<String> {
    if config.plugin_repos.is_empty() {
        return Vec::new();
    }

    let base_dir = "/opt/moatless/plugins";
    if let Err(e) = fs::create_dir_all(base_dir).await {
        warn!(error = %e, "Failed to create plugins directory");
        return Vec::new();
    }

    let git_token = std::env::var("GIT_TOKEN").ok();

    let mut cloned_paths = Vec::new();

    for url in &config.plugin_repos {
        let name = derive_plugin_name(url);
        let dest = format!("{base_dir}/{name}");

        // Skip if already cloned
        if Path::new(&dest).join(".git").exists() {
            info!(name = %name, "Plugin repo already cloned, skipping");
            cloned_paths.push(dest);
            continue;
        }

        // Build clone URL with token auth if available
        let clone_url = match &git_token {
            Some(token) => super::git::inject_token_into_url(url, token),
            None => url.clone(),
        };

        let args = vec!["clone", "--depth", "1", &clone_url, &dest];
        match super::git::run_command("git", &args).await {
            Ok(()) => {
                info!(name = %name, "Plugin repo cloned successfully");
                cloned_paths.push(dest);
            }
            Err(e) => {
                warn!(name = %name, url = %url, error = %e, "Failed to clone plugin repo (non-fatal)");
            }
        }
    }

    cloned_paths
}

/// Derive a plugin name from a git URL.
///
/// Extracts the last path segment and strips `.git` suffix.
/// e.g. `https://github.com/org/foo-plugin.git` → `foo-plugin`
fn derive_plugin_name(url: &str) -> String {
    url.rsplit('/')
        .next()
        .unwrap_or("plugin")
        .trim_end_matches(".git")
        .to_string()
}

/// Run a single setup command in the workspace directory.
pub async fn run_setup_command(config: &Config, cmd: &str) -> Result<(), String> {
    if cmd.is_empty() {
        return Ok(());
    }

    info!(command = %cmd, "Running setup command");
    let output = Command::new("sh")
        .args(["-c", cmd])
        .current_dir(&config.workspace_dir)
        .output()
        .await
        .map_err(|e| format!("Failed to execute: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Exit code {}: {stderr}", output.status));
    }

    Ok(())
}
