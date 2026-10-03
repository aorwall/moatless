//! The rows a profile switch reads and writes.

use moatless_core::db::DbPool;

use crate::conversation::projector::Projector;
use crate::task_event::models::InsertTaskEvent;

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
/// together, with `note` recorded and projected beside them.
pub(crate) async fn write_profile(
    pool: &DbPool,
    task_id: &str,
    profile: moatless_types::agent::AgentProfile,
    model: Option<&str>,
    note: Option<&InsertTaskEvent>,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("UPDATE tasks SET agent_profile = $2, model = $3 WHERE task_id = $1")
        .bind(task_id)
        .bind(profile.as_str())
        .bind(model)
        .execute(&mut *tx)
        .await?;
    // A replacement pod boots with this; a joined Member's switch leaves the
    // owner's.
    sqlx::query("UPDATE sandboxes SET agent_profile = $2 WHERE sandbox_id = $1")
        .bind(task_id)
        .bind(profile.as_str())
        .execute(&mut *tx)
        .await?;
    if let Some(note) = note {
        crate::task_event::dao::insert_in(&mut tx, note).await?;
        Projector.on_task_event(&mut tx, note).await?;
    }
    // Last: `record_change` takes the listing counter. The shell feed re-reads
    // only the rows it names, so without it the thread keeps its old model.
    moatless_core::change_feed::listing::record_change(&mut tx, task_id).await?;
    tx.commit().await
}

#[cfg(all(test, feature = "db-tests"))]
#[path = "switch_dao_db_tests.rs"]
mod switch_dao_db_tests;
