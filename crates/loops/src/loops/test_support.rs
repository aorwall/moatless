//! Loop row fixtures for a DB-backed test, so no test names the private store.

use moatless_core::db::DbPool;
use moatless_types::access_control::ScopeContext;
use moatless_types::loops::SubscriptionMatch;

use crate::loops::operations::Loops;

/// Activate a Loop under `run_as_user_id` without resolving that login.
///
/// [`Loops::activate`] refuses a login no user row carries; a fixture names a
/// symbolic runner it never creates.
pub async fn activate_loop(
    loops: &Loops,
    ctx: &ScopeContext,
    loop_id: &str,
    run_as_user_id: &str,
) -> Result<bool, sqlx::Error> {
    loops.store().activate(loop_id, run_as_user_id, ctx).await
}

/// Write the Subscription Loop an inbound Adapter test delivers events against.
pub async fn upsert_subscription(
    loops: &Loops,
    subscription: &SubscriptionMatch,
) -> Result<SubscriptionMatch, sqlx::Error> {
    loops.store().upsert_subscription(subscription).await
}

/// Insert a manual Loop on `workspace_id`, which git sync never reconciles.
pub async fn insert_manual_loop(
    db: &DbPool,
    loop_id: &str,
    name: &str,
    workspace_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO loops (id, name, kind, active, scope, created_by, owner_user_id) \
         VALUES ($1, $2, 'adapter_event', true, 'user', 'user-1', 'user-1')",
    )
    .bind(loop_id)
    .bind(name)
    .execute(db)
    .await?;
    sqlx::query(
        "INSERT INTO loop_configs (loop_id, workspace_id, routing_mode, agent_profile) \
         VALUES ($1, $2, 'by_subject', 'claude-code')",
    )
    .bind(loop_id)
    .bind(workspace_id)
    .execute(db)
    .await?;
    Ok(())
}
