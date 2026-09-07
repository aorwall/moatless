use super::models::CheckoutMode;
use crate::config::Config;
use moatless_types::workspace::sanitize_mount_name;
use std::path::Path;
use tracing::warn;

pub(super) async fn materialize_checkout(
    config: &Config,
    task_id: &str,
    checkout_mode: CheckoutMode,
) -> Result<(String, String), String> {
    match checkout_mode {
        CheckoutMode::Shared => Ok((config.workspace_dir.clone(), config.repo_dir.clone())),
        CheckoutMode::Isolated => materialize_isolated_checkout(config, task_id).await,
    }
}

async fn materialize_isolated_checkout(
    config: &Config,
    task_id: &str,
) -> Result<(String, String), String> {
    let workspace_dir = isolated_workspace_dir(&config.workspace_dir, task_id)?;
    tokio::fs::create_dir_all(&workspace_dir)
        .await
        .map_err(|e| format!("create isolated workspace {workspace_dir}: {e}"))?;

    let mut task_config = config.clone();
    task_config.task_id = task_id.to_string();
    task_config.workspace_dir = workspace_dir.clone();
    task_config.repo_dir = primary_repo_dir_for_workspace(&task_config);

    if !task_config.repos.is_empty() {
        if let Err(e) = crate::init::git::clone_repositories(&task_config).await {
            if let Err(cleanup) = tokio::fs::remove_dir_all(&workspace_dir).await {
                warn!(
                    task_id = %task_id,
                    dir = %workspace_dir,
                    error = %cleanup,
                    "Failed to clean isolated checkout after clone failure"
                );
            }
            return Err(e.to_string());
        }
    }

    Ok((workspace_dir, task_config.repo_dir))
}

fn isolated_workspace_dir(root_workspace_dir: &str, task_id: &str) -> Result<String, String> {
    let segment = sanitize_mount_name(task_id)
        .ok_or_else(|| format!("task_id cannot form a safe checkout directory: {task_id}"))?;
    Ok(Path::new(root_workspace_dir)
        .join(".moatless")
        .join("members")
        .join(segment)
        .to_string_lossy()
        .into_owned())
}

fn primary_repo_dir_for_workspace(config: &Config) -> String {
    config
        .repos
        .iter()
        .find(|repo| repo.is_primary)
        .or_else(|| config.repos.first())
        .map_or_else(
            || config.workspace_dir.clone(),
            |repo| repo.dir(&config.workspace_dir),
        )
}
