//! Workspace setup: server configs, node_modules, setup commands, server start.

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

/// Copy pre-installed node_modules from image layer if available.
pub async fn copy_prebuilt_deps(config: &Config) {
    let prebuilt = "/usr/share/prebuilt-deps/node_modules";
    let target = format!("{}/node_modules", config.workspace_dir);

    if Path::new(prebuilt).exists() && !Path::new(&target).exists() {
        info!("Copying pre-installed node_modules from image");
        let output = Command::new("cp")
            .args(["-a", prebuilt, &target])
            .output()
            .await;
        match output {
            Ok(o) if o.status.success() => {
                info!("Pre-installed node_modules copied");
            }
            Ok(o) => {
                let stderr = String::from_utf8_lossy(&o.stderr);
                warn!(error = %stderr, "Failed to copy pre-installed node_modules");
            }
            Err(e) => {
                warn!(error = %e, "Failed to run cp for node_modules");
            }
        }
    }
}

/// Set up Next.js build cache on tmpfs.
pub async fn setup_build_cache(config: &Config) {
    let cache_dir = "/opt/moatless/build-cache";
    if !Path::new(cache_dir).exists() {
        return;
    }

    let next_dir = format!("{}/.next", config.workspace_dir);

    // Remove stale dangling symlink
    if let Ok(meta) = fs::symlink_metadata(&next_dir).await {
        if meta.file_type().is_symlink() && fs::metadata(&next_dir).await.is_err() {
            let _ = fs::remove_file(&next_dir).await;
        }
    }

    if !Path::new(&next_dir).exists() {
        let next_cache = format!("{cache_dir}/next");
        let _ = fs::create_dir_all(&next_cache).await;
        if let Err(e) = tokio::fs::symlink(&next_cache, &next_dir).await {
            warn!(error = %e, "Failed to create .next symlink");
        } else {
            info!("Linked .next build cache to tmpfs");
        }
    }

    // Ensure node_modules symlink in tmpfs
    let nm_link = format!("{cache_dir}/node_modules");
    if !Path::new(&nm_link).exists() {
        let nm_target = format!("{}/node_modules", config.workspace_dir);
        let _ = tokio::fs::symlink(&nm_target, &nm_link).await;
    }

    // Seed Next.js cache from pre-built image if available
    let prebuilt_cache = "/usr/share/prebuilt-deps/next-cache";
    let target_cache = format!("{cache_dir}/next/cache");
    if Path::new(prebuilt_cache).exists() && !Path::new(&target_cache).exists() {
        let _ = Command::new("cp")
            .args(["-a", prebuilt_cache, &target_cache])
            .output()
            .await;
        info!("Seeded Next.js build cache from image");
    }
}

/// Write server config files from REPO_CONFIG.
pub async fn write_server_configs(config: &Config) {
    let Some(ref raw) = config.repo_config_raw else {
        return;
    };

    // Parse servers from raw config JSON
    let parsed: serde_json::Value = match serde_json::from_str(raw) {
        Ok(v) => v,
        Err(_) => return,
    };

    let servers = match parsed.get("servers").and_then(|s| s.as_array()) {
        Some(arr) => arr,
        None => return,
    };

    for srv in servers {
        let Some(name) = srv.get("name").and_then(|n| n.as_str()) else {
            continue;
        };
        let port = srv.get("port").and_then(|p| p.as_u64()).unwrap_or(0);
        let start_commands = srv
            .get("startCommands")
            .cloned()
            .unwrap_or(serde_json::json!([]));
        let timeout = srv
            .get("startupTimeoutSeconds")
            .and_then(|t| t.as_u64())
            .unwrap_or(30);

        let config_json = serde_json::json!({
            "port": port,
            "startCommands": start_commands,
            "startupTimeoutSeconds": timeout,
            "workspacePath": config.workspace_dir,
        });

        let config_path = format!("{}/{name}-server.config.json", config.data_dir);
        if let Err(e) = fs::write(&config_path, config_json.to_string()).await {
            warn!(server = %name, error = %e, "Failed to write server config");
        } else {
            info!(server = %name, port, "Wrote server config");
        }

        // Write initial "starting" status if server has commands
        if start_commands
            .as_array()
            .map(|a| !a.is_empty())
            .unwrap_or(false)
        {
            let status_path = format!("{}/{name}-server.status.json", config.data_dir);
            let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
            let status = serde_json::json!({
                "status": "starting",
                "startedAt": now,
            });
            let _ = fs::write(&status_path, status.to_string()).await;
        }
    }
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

/// Start a server via the generic `server` command.
pub async fn start_server(config: &Config, name: &str) -> Result<(), String> {
    let server_bin = "/usr/local/bin/server";
    if !Path::new(server_bin).exists() {
        return Err("server binary not found".to_string());
    }

    info!(server = %name, "Starting server");
    let output = Command::new(server_bin)
        .args([name, "start"])
        .current_dir(&config.data_dir)
        .output()
        .await
        .map_err(|e| format!("Failed to run server start: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("server start failed: {stderr}"));
    }

    Ok(())
}
