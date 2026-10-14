//! The search query: one statement over the open Tasks a viewer may read.

use sqlx::AssertSqlSafe;

use moatless_core::db::DbPool;
use moatless_types::access_control::{ScopeContext, ScopedEntity};
use moatless_types::task::TaskStatus;

use super::TaskMatch;

/// How long one search may run. The scan covers every readable open Task's
/// Messages through an unindexed `ILIKE '%…%'`, so without a bound a slow
/// search holds a pool connection the other reads need. A cancelled search
/// fails with Postgres's `57014`.
const SEARCH_STATEMENT_TIMEOUT: &str = "5s";
pub(crate) const SEARCH_TIMEOUTS: &str = "moatless_task_search_timeouts_total";

/// A search term as a `LIKE` operand rather than a `LIKE` pattern.
///
/// `%`, `_`, and the escape character itself (`\`) are escaped, since they're
/// ordinary characters to the person typing them; every query using this must
/// declare `ESCAPE '\'` explicitly, since Postgres's default depends on
/// `standard_conforming_strings`.
fn escape_like(query: &str) -> String {
    let mut escaped = String::with_capacity(query.len());
    for character in query.chars() {
        if matches!(character, '%' | '_' | '\\') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

pub(crate) async fn search(
    pool: &DbPool,
    viewer: &ScopeContext,
    query: &str,
    limit: i64,
) -> Result<Vec<TaskMatch>, sqlx::Error> {
    // The read rule is the shared access fragment rather than a copy: a
    // drift here is a disclosure, not a missing row.
    let open = TaskStatus::Open.as_str();
    let access = moatless_core::db::access_control::access_filter(ScopedEntity::Task, "t.", 1);
    let pattern = access.next_param();
    let row_limit = pattern + 1;
    let sql = format!(
        r#"WITH listed AS MATERIALIZED (
               SELECT t.task_id, t.workspace_id, t.name, t.updated_at
               FROM tasks t
               WHERE t.status = '{open}'
                 AND {}
           ),
           matched AS (
               -- A Message match. `author` distinguishes the person from
               -- the agent; `parent_tool_use_id IS NULL` drops subagent rows,
               -- which are never rendered inline and would otherwise be the
               -- bulk of what a long conversation matches.
               SELECT
                   t.task_id,
                   t.workspace_id,
                   m.author = 'user' AS from_user,
                   COALESCE(m.text, i.text) AS matched_text,
                   m.created_at          AS matched_at,
                   t.updated_at,
                   CASE WHEN m.author = 'user' THEN 0 ELSE 1 END AS rank
               FROM listed t
               JOIN conversation_messages m ON m.task_id = t.task_id
               LEFT JOIN task_inbox i ON i.id = m.inbox_id
               WHERE m.parent_tool_use_id IS NULL
                 -- A sandbox wake is in no conversation to jump to.
                 AND i.origin IS DISTINCT FROM 'sandbox'
                 AND COALESCE(m.text, i.text) ILIKE ${pattern} ESCAPE '\'
               UNION ALL
               -- A name match, which the reference server has no equivalent
               -- of. `tasks.name` is Moatless's own handle for a Task and is
               -- often the only text a person remembers; it ranks below both
               -- Message sources because a Message match is the more
               -- specific answer about the same Task.
               SELECT
                   t.task_id,
                   t.workspace_id,
                   true   AS from_user,
                   t.name AS matched_text,
                   NULL::timestamptz AS matched_at,
                   t.updated_at,
                   2 AS rank
               FROM listed t
               WHERE t.name ILIKE ${pattern} ESCAPE '\'
           ),
           ranked AS (
               SELECT
                   matched.*,
                   ROW_NUMBER() OVER (
                       PARTITION BY task_id
                       ORDER BY rank ASC, matched_at DESC NULLS LAST
                   ) AS per_task
               FROM matched
           )
           SELECT task_id, workspace_id, from_user, matched_text, matched_at
           FROM ranked
           WHERE per_task = 1
           ORDER BY rank ASC, updated_at DESC NULLS LAST, task_id ASC
           LIMIT ${row_limit}"#,
        access.sql(),
    );

    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('statement_timeout', $1, true)")
        .bind(SEARCH_STATEMENT_TIMEOUT)
        .execute(&mut *tx)
        .await?;
    let result = sqlx::query_as::<_, TaskMatch>(AssertSqlSafe(sql))
        .bind(viewer.is_admin)
        .bind(&viewer.user_id)
        .bind(format!("%{}%", escape_like(query)))
        .bind(limit)
        .fetch_all(&mut *tx)
        .await;
    if matches!(&result, Err(sqlx::Error::Database(error)) if error.code().as_deref() == Some("57014"))
    {
        metrics::counter!(SEARCH_TIMEOUTS).increment(1);
    }
    let rows = result?;
    tx.commit().await?;
    Ok(rows)
}
