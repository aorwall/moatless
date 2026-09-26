//! The rows a profile switch reads and writes.

use moatless_core::db::DbPool;

/// Whether `sandbox_id`'s pod runs Claude Code over tmux; `None` for no row.
pub(crate) async fn sandbox_tmux(
    pool: &DbPool,
    sandbox_id: &str,
) -> Result<Option<bool>, sqlx::Error> {
    sqlx::query_scalar("SELECT use_tmux FROM sandboxes WHERE sandbox_id = $1")
        .bind(sandbox_id)
        .fetch_optional(pool)
        .await
}

/// Set `task_id`'s profile and model, and the profile of the Sandbox it owns,
/// together.
pub(crate) async fn write_profile(
    pool: &DbPool,
    task_id: &str,
    profile: &str,
    model: Option<&str>,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("UPDATE tasks SET agent_profile = $2, model = $3 WHERE task_id = $1")
        .bind(task_id)
        .bind(profile)
        .bind(model)
        .execute(&mut *tx)
        .await?;
    // A replacement pod boots with this; a joined Member's switch leaves the
    // owner's.
    sqlx::query("UPDATE sandboxes SET agent_profile = $2 WHERE sandbox_id = $1")
        .bind(task_id)
        .bind(profile)
        .execute(&mut *tx)
        .await?;
    tx.commit().await
}
