//! The search query: one statement over the open Tasks a viewer may read.

use sqlx::AssertSqlSafe;

use moatless_core::db::DbPool;
use moatless_types::access_control::{ScopeContext, ScopedEntity};
use moatless_types::task::TaskStatus;

use super::TaskMatch;

/// How long one search may run. A term matching few Messages still scans
/// every readable open Task's Messages through an unindexed `ILIKE '%…%'`, so
/// without a bound a slow search holds a pool connection the other reads
/// need. A cancelled search fails as [`moatless_core::db::is_statement_timeout`].
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
    let newest_hit = |author: &str| {
        format!(
            r#"SELECT COALESCE(m.text, i.text) AS matched_text, m.created_at AS matched_at
               FROM conversation_messages m
               LEFT JOIN task_inbox i ON i.id = m.inbox_id
               WHERE m.task_id = t.task_id
                 AND m.author {author}
                 -- Subagent rows are never rendered inline and would
                 -- otherwise be the bulk of what a long conversation matches.
                 AND m.parent_tool_use_id IS NULL
                 -- A sandbox wake is in no conversation to jump to.
                 AND i.origin IS DISTINCT FROM 'sandbox'
                 AND COALESCE(m.text, i.text) ILIKE ${pattern} ESCAPE '\'
               ORDER BY m.created_at DESC
               LIMIT 1"#
        )
    };
    let (person_hit, agent_hit) = (newest_hit("= 'user'"), newest_hit("<> 'user'"));
    // Each source is walked Task by Task in the final order and stops at
    // `limit` Tasks, rather than matching every Message and ranking them all:
    // a common term then reads a few Tasks, where ranking every match sorted
    // the whole corpus past the statement timeout. A source cut short only
    // drops Tasks the sources above it already push past `limit`.
    let sql = format!(
        r#"WITH listed AS MATERIALIZED (
               SELECT t.task_id, t.workspace_id, t.name, t.updated_at
               FROM tasks t
               WHERE t.status = '{open}'
                 AND {access}
           ),
           person AS MATERIALIZED (
               SELECT t.task_id, t.workspace_id, t.updated_at, hit.matched_text, hit.matched_at
               FROM listed t
               CROSS JOIN LATERAL ({person_hit}) hit
               ORDER BY t.updated_at DESC NULLS LAST, t.task_id ASC
               LIMIT ${row_limit}
           ),
           agent AS MATERIALIZED (
               SELECT t.task_id, t.workspace_id, t.updated_at, hit.matched_text, hit.matched_at
               FROM listed t
               CROSS JOIN LATERAL ({agent_hit}) hit
               WHERE (SELECT count(*) FROM person) < ${row_limit}
                 AND NOT EXISTS (SELECT 1 FROM person p WHERE p.task_id = t.task_id)
               ORDER BY t.updated_at DESC NULLS LAST, t.task_id ASC
               LIMIT ${row_limit}
           ),
           matched AS (
               SELECT task_id, workspace_id, true AS from_user, matched_text, matched_at,
                      updated_at, 0 AS rank
               FROM person
               UNION ALL
               SELECT task_id, workspace_id, false, matched_text, matched_at, updated_at, 1
               FROM agent
               UNION ALL
               -- A name match, which the reference server has no equivalent
               -- of. `tasks.name` is Moatless's own handle for a Task and is
               -- often the only text a person remembers; it ranks below both
               -- Message sources because a Message match is the more
               -- specific answer about the same Task.
               SELECT t.task_id, t.workspace_id, true, t.name, NULL::timestamptz, t.updated_at, 2
               FROM listed t
               WHERE t.name ILIKE ${pattern} ESCAPE '\'
                 AND NOT EXISTS (SELECT 1 FROM person p WHERE p.task_id = t.task_id)
                 AND NOT EXISTS (SELECT 1 FROM agent a WHERE a.task_id = t.task_id)
           )
           SELECT task_id, workspace_id, from_user, matched_text, matched_at
           FROM matched
           ORDER BY rank ASC, updated_at DESC NULLS LAST, task_id ASC
           LIMIT ${row_limit}"#,
        access = access.sql(),
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
    if result
        .as_ref()
        .is_err_and(moatless_core::db::is_statement_timeout)
    {
        metrics::counter!(SEARCH_TIMEOUTS).increment(1);
    }
    let rows = result?;
    tx.commit().await?;
    Ok(rows)
}
