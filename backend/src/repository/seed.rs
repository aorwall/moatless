//! Seed default template repositories on startup.

use serde_json::json;
use tracing::info;

use super::dao::RepoRepository;
use super::models::{Repository, RepositoryKind, RepositoryProvider};
use crate::access_control::SYSTEM_TEAM_ID;
use moatless_backend_core::scope::Scope;

/// Insert default template repositories if they don't already exist.
///
/// Called once during startup. Idempotent — skips if a row with the
/// same name and `created_by = "system"` already exists.
pub async fn seed_default_repositories(repos: &dyn RepoRepository) {
    if let Err(e) = seed_moatable(repos).await {
        tracing::warn!(error = %e, "Failed to seed default Moatable repository");
    }
}

async fn seed_moatable(repos: &dyn RepoRepository) -> Result<(), sqlx::Error> {
    // Check including soft-deleted rows so we don't re-create a repo the user removed.
    if repos
        .exists_by_name_including_deleted("Moatable", "system")
        .await?
    {
        info!("Default repository 'Moatable' already exists, skipping seed");
        return Ok(());
    }

    let now = chrono::Utc::now();
    let row = Repository {
        id: uuid::Uuid::new_v4().to_string(),
        name: "Moatable".to_string(),
        kind: RepositoryKind::Template,
        remote_url: Some("https://github.com/aorwall/moatable".to_string()),
        default_branch: Some("main".to_string()),
        docker_image: None,
        docker_access: None,
        sandbox_service_account: None,
        network_profile: None,
        system_prompt: None,
        setup_commands: Some(json!(["bun install"])),
        env_vars: None,
        servers: Some(json!([{
            "name": "preview",
            "label": "Preview",
            "port": 3000,
            "startCommands": ["bun run dev"],
            "startupTimeoutSeconds": 30,
            "autoReload": true,
            "enableInspector": true,
            "default": true
        }])),
        plugins: None,
        server_start_commands: None,
        startup_timeout_seconds: None,
        build_command: None,
        build_output_directory: None,
        enable_inspector: None,
        auto_reload: None,
        resources: None,
        description: Some("React + TypeScript + Vite + Tailwind CSS starter template".to_string()),
        provider: Some(RepositoryProvider::Github),
        scope: Scope::Global,
        team_id: Some(SYSTEM_TEAM_ID.to_string()),
        owner_user_id: None,
        created_by: Some("system".to_string()),
        deleted: Some(false),
        created_at: Some(now),
        updated_at: Some(now),
    };

    repos.insert(&row).await?;
    info!(id = %row.id, "Seeded default repository 'Moatable'");
    Ok(())
}
