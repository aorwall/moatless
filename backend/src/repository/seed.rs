//! Seed default template repositories on startup.

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
    // Pure git pointer: run-config (setupCommands + the preview server) lives in
    // git at `aorwall/moatable` `.moatless/config.json` and is synced into the
    // workspace on repo-create / task-start.
    let row = Repository {
        id: uuid::Uuid::new_v4().to_string(),
        name: "Moatable".to_string(),
        kind: RepositoryKind::Template,
        remote_url: Some("https://github.com/aorwall/moatable".to_string()),
        default_branch: Some("main".to_string()),
        description: Some("React + TypeScript + Vite + Tailwind CSS starter template".to_string()),
        provider: Some(RepositoryProvider::Github),
        scope: Scope::Global,
        team_id: Some(SYSTEM_TEAM_ID.to_string()),
        owner_user_id: None,
        created_by: Some("system".to_string()),
        access_status: "unchecked".to_string(),
        access_checked_at: None,
        access_error: None,
        deleted: Some(false),
        created_at: Some(now),
        updated_at: Some(now),
    };

    repos.insert(&row).await?;
    info!(id = %row.id, "Seeded default repository 'Moatable'");
    Ok(())
}
