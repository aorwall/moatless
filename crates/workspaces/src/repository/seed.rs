//! Seed default template repositories on startup.

use tracing::info;

use super::dao::RepositoryDao;
use super::models::Repository;
use moatless_types::repository::normalize_remote_url;
use moatless_types::access_control::Scope;
use moatless_types::repository::{RepositoryKind, RepositoryProvider};

/// Insert default template repositories if they don't already exist.
///
/// Called once during startup. Idempotent — skips if a row with the same
/// canonical URL and access owner already exists.
pub async fn seed_default_repositories(repos: &dyn RepositoryDao) {
    if let Err(e) = seed_moatable(repos).await {
        tracing::warn!(error = %e, "Failed to seed default Moatable repository");
    }
}

async fn seed_moatable(repos: &dyn RepositoryDao) -> Result<(), sqlx::Error> {
    const MOATABLE_URL: &str = "https://github.com/aorwall/moatable";

    // Check including soft-deleted rows so we don't re-create a repo the user removed.
    if repos
        .exists_by_normalized_remote_url_including_deleted(
            &normalize_remote_url(MOATABLE_URL),
            Scope::Global,
            None,
        )
        .await?
    {
        info!("Default repository 'Moatable' already exists, skipping seed");
        return Ok(());
    }

    let now = chrono::Utc::now();
    // Pure git pointer: run-config (setupCommands + the preview server) lives in
    // git at `aorwall/moatable` and is synced into the workspace on repo-create
    // / task-start, once that repo declares `.moatless/workspaces.json`.
    let row = Repository {
        id: uuid::Uuid::new_v4().to_string(),
        name: "Moatable".to_string(),
        kind: RepositoryKind::Template,
        remote_url: Some(MOATABLE_URL.to_string()),
        default_branch: Some("main".to_string()),
        description: Some("React + TypeScript + Vite + Tailwind CSS starter template".to_string()),
        provider: Some(RepositoryProvider::Github),
        scope: Scope::Global,
        owner_user_id: None,
        created_by: Some("system".to_string()),
        deleted: false,
        created_at: Some(now),
        updated_at: Some(now),
    };

    repos.insert(&row).await?;
    info!(id = %row.id, "Seeded default repository 'Moatable'");
    Ok(())
}
