//! Repository repository — read-only queries for auto-provisioning.

use super::models::RepositoryRow;
use super::DbPool;

/// Find a repository by its id.
pub async fn find_repository_by_id(
    pool: &DbPool,
    id: &str,
) -> Result<Option<RepositoryRow>, sqlx::Error> {
    sqlx::query_as::<_, RepositoryRow>(
        r#"SELECT
            id, name, kind, remote_url, default_branch,
            docker_image, system_prompt, setup_commands, env_vars,
            servers, plugins,
            server_start_commands, startup_timeout_seconds,
            build_command, build_output_directory,
            enable_inspector, auto_reload
        FROM repositories
        WHERE id = $1"#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await
}
