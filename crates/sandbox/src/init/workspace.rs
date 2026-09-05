//! Workspace setup: `node_modules` and setup commands.

use std::path::Path;

use tokio::fs;
use tokio::process::Command;
use tracing::{info, warn};

use crate::config::Config;

/// Clone each of the task owner's plugins into `/opt/moatless/plugins/{name}/`
/// and remove the skills they did not pick.
///
/// Clone is `git clone --depth 1` using `GIT_TOKEN` for auth, and skips a
/// plugin that is already there. Non-fatal on failure. The prune runs on every
/// pass, including for an already-cloned plugin, because the picked set can
/// change between two starts of the same sandbox.
///
/// Returns the directories that are ready to use.
pub async fn clone_plugins(config: &Config) -> Vec<String> {
    if config.plugins.is_empty() {
        return Vec::new();
    }

    let base_dir = "/opt/moatless/plugins";
    if let Err(e) = fs::create_dir_all(base_dir).await {
        warn!(error = %e, "Failed to create plugins directory");
        return Vec::new();
    }

    let git_token = std::env::var("GIT_TOKEN").ok();

    let mut cloned_paths = Vec::new();

    for plugin in &config.plugins {
        let name = &plugin.name;
        let dest = format!("{base_dir}/{name}");

        if Path::new(&dest).join(".git").exists() {
            info!(name = %name, "Plugin already cloned, skipping clone");
        } else {
            // Build clone URL with token auth if available
            let clone_url = match &git_token {
                Some(token) => super::git::inject_token_into_url(&plugin.git_url, token),
                None => plugin.git_url.clone(),
            };

            let args = vec!["clone", "--depth", "1", &clone_url, &dest];
            if let Err(e) = super::git::run_command("git", &args).await {
                warn!(
                    name = %name,
                    url = %plugin.git_url,
                    error = %e,
                    "Failed to clone plugin (non-fatal)"
                );
                continue;
            }
            info!(name = %name, "Plugin cloned successfully");
        }

        let dropped = super::plugin_skills::keep_only(Path::new(&dest), name, &plugin.skill_names);
        if dropped > 0 {
            info!(name = %name, dropped, "Removed skills this task's owner did not pick");
        }
        cloned_paths.push(dest);
    }

    cloned_paths
}

/// Run a single setup command in the repository directory.
pub async fn run_setup_command(config: &Config, cmd: &str) -> Result<(), String> {
    if cmd.is_empty() {
        return Ok(());
    }

    info!(command = %cmd, "Running setup command");
    let output = Command::new("sh")
        .args(["-c", cmd])
        .current_dir(&config.repo_dir)
        .output()
        .await
        .map_err(|e| format!("Failed to execute: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Exit code {}: {stderr}", output.status));
    }

    Ok(())
}
