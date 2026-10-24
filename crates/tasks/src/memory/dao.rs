//! Memory DAO — queries for `memory_entries`, `memory_events` and
//! `memory_reads`.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::PgConnection;

use super::models::{
    ChangedSince, ClaimOutcome, ListFilter, MemoryEntryRow, MemoryEventRow, PutEntry, PutOutcome,
    ScopeRef, SeenAt, TtlChange, Writer,
};
use moatless_api_schemas::memory::DEFAULT_TTL_SECONDS;
use moatless_core::db::DbPool;
use moatless_types::memory::MemoryOp;

/// The [`MemoryEntryRow`] projection of `memory_entries e`. A claim reads as
/// held only while unexpired and its Task open, so a crashed holder frees it.
macro_rules! entry_columns {
    () => {
        "e.entry_id, e.scope_kind, e.scope_id, e.key, e.summary, e.body, e.data, e.tags, \
         e.citations, e.version, e.ttl_seconds, e.expires_at, e.stale_after_seconds, \
         e.verified_at, \
         CASE WHEN e.claim_expires_at > now() AND EXISTS ( \
             SELECT 1 FROM tasks t WHERE t.task_id = e.claimed_by_task_id AND t.status = 'open' \
         ) THEN e.claimed_by_task_id END AS claimed_by_task_id, \
         e.claim_expires_at, e.updated_by_task_id, e.created_by_task_id, \
         e.created_by_loop_id, e.created_by_user_id, e.created_at, e.updated_at, \
         (e.stale_after_seconds IS NOT NULL AND COALESCE(e.verified_at, e.updated_at) \
             + (e.stale_after_seconds::double precision * interval '1 second') < now()) AS stale"
    };
}

/// Rows in the scopes bound as `$1` (kinds) and `$2` (ids) that have not
/// expired.
macro_rules! visible {
    () => {
        "(e.scope_kind, e.scope_id) IN (SELECT * FROM unnest($1::text[], $2::text[])) \
         AND (e.expires_at IS NULL OR e.expires_at > now())"
    };
}

/// DAO trait for memory persistence. Callers resolve and authorize scopes
/// first: the tables carry no access-control columns.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait MemoryDao: Send + Sync {
    /// Create or update `(scope, key)` and record a `put` event, in one
    /// transaction. An expired entry is replaced as if absent.
    async fn put(&self, entry: &PutEntry) -> Result<PutOutcome, sqlx::Error>;
    /// The unexpired entries named `key` in any of `scopes`.
    async fn find(
        &self,
        scopes: &[ScopeRef],
        key: &str,
    ) -> Result<Vec<MemoryEntryRow>, sqlx::Error>;
    /// Unexpired entries in `scopes`, newest first, at most `filter.limit`.
    async fn list(
        &self,
        scopes: &[ScopeRef],
        filter: &ListFilter,
    ) -> Result<Vec<MemoryEntryRow>, sqlx::Error>;
    /// Unexpired entries in `scopes` matching `query` as a web-search
    /// expression, best match first.
    async fn search(
        &self,
        scopes: &[ScopeRef],
        query: &str,
        limit: i64,
    ) -> Result<Vec<MemoryEntryRow>, sqlx::Error>;
    /// Unexpired entries in `scopes` whose key is one of `keys` or starts with
    /// one of `prefixes`, newest first, at most `limit`, narrowed by `changed`.
    async fn preload(
        &self,
        scopes: &[ScopeRef],
        keys: &[String],
        prefixes: &[String],
        changed: Option<ChangedSince>,
        limit: i64,
    ) -> Result<Vec<MemoryEntryRow>, sqlx::Error>;
    /// Record that `task_id` read each `(entry_id, version)`.
    async fn record_reads(&self, task_id: &str, reads: &[(String, i32)])
    -> Result<(), sqlx::Error>;
    /// Record an `append` event with `text`; the entry's version stays.
    async fn append(
        &self,
        entry_id: &str,
        text: &str,
        writer: &Writer,
    ) -> Result<Option<MemoryEntryRow>, sqlx::Error>;
    /// Stamp `verified_at`, renew `expires_at` from `ttl_seconds`, and record
    /// a `verify` event.
    async fn verify(
        &self,
        entry_id: &str,
        writer: &Writer,
    ) -> Result<Option<MemoryEntryRow>, sqlx::Error>;
    /// Claim the entry for `writer.task_id` for `ttl_seconds`, unless another
    /// open Task holds an unexpired claim. Re-claiming renews. Advisory: a
    /// claim marks who is working the item and blocks only another claim, so
    /// other Tasks can still append notes to the entry.
    async fn claim(
        &self,
        entry_id: &str,
        ttl_seconds: i64,
        writer: &Writer,
    ) -> Result<Option<ClaimOutcome>, sqlx::Error>;
    /// Drop `task_id`'s claim. `false` when it held none.
    async fn release(&self, entry_id: &str, writer: &Writer) -> Result<bool, sqlx::Error>;
    /// Delete the entry with its history and reads. `false` when absent.
    async fn delete(&self, entry_id: &str) -> Result<bool, sqlx::Error>;
    /// The entry's history, newest first, at most `limit`.
    async fn log(&self, entry_id: &str, limit: i64) -> Result<Vec<MemoryEventRow>, sqlx::Error>;
    /// The task's committed `memory_seen` time, with the database's `now()`.
    async fn seen_at(&self, task_id: &str) -> Result<SeenAt, sqlx::Error>;
    /// Stage `at`, replacing any staged time an earlier fetch left; `None`
    /// clears it.
    async fn stage_seen(&self, task_id: &str, at: Option<DateTime<Utc>>)
    -> Result<(), sqlx::Error>;
    /// Commit the staged time, if any.
    async fn commit_seen(&self, task_id: &str) -> Result<(), sqlx::Error>;
}

/// PostgreSQL implementation of [`MemoryDao`].
pub struct PgMemoryDao(pub DbPool);

fn scope_arrays(scopes: &[ScopeRef]) -> (Vec<String>, Vec<String>) {
    scopes
        .iter()
        .map(|s| (s.scope.as_str().to_string(), s.id.clone()))
        .unzip()
}

async fn fetch_entry(
    conn: &mut PgConnection,
    entry_id: &str,
) -> Result<Option<MemoryEntryRow>, sqlx::Error> {
    sqlx::query_as::<_, MemoryEntryRow>(concat!(
        "SELECT ",
        entry_columns!(),
        " FROM memory_entries e WHERE e.entry_id = $1"
    ))
    .bind(entry_id)
    .fetch_optional(conn)
    .await
}

async fn insert_event(
    conn: &mut PgConnection,
    entry_id: &str,
    op: MemoryOp,
    version: i32,
    text: Option<&str>,
    writer: &Writer,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"INSERT INTO memory_events (entry_id, op, version, text, task_id, user_id)
           VALUES ($1, $2, $3, $4, $5, $6)"#,
    )
    .bind(entry_id)
    .bind(op.as_str())
    .bind(version)
    .bind(text)
    .bind(&writer.task_id)
    .bind(&writer.user_id)
    .execute(conn)
    .await?;
    Ok(())
}

/// The live, unexpired row id and version of `(scope, key)`, locked.
async fn lock_existing(
    conn: &mut PgConnection,
    entry: &PutEntry,
) -> Result<Option<(String, i32, Option<i64>)>, sqlx::Error> {
    sqlx::query_as::<_, (String, i32, Option<i64>)>(
        r#"SELECT entry_id, version, ttl_seconds FROM memory_entries
           WHERE scope_kind = $1 AND scope_id = $2 AND key = $3
           FOR UPDATE"#,
    )
    .bind(entry.scope.scope.as_str())
    .bind(&entry.scope.id)
    .bind(&entry.key)
    .fetch_optional(conn)
    .await
}

async fn insert_new(
    conn: &mut PgConnection,
    entry: &PutEntry,
    summary: &str,
) -> Result<Option<String>, sqlx::Error> {
    let ttl = match entry.ttl {
        TtlChange::Keep => Some(DEFAULT_TTL_SECONDS),
        TtlChange::Set(seconds) => Some(seconds),
        TtlChange::NoExpiry => None,
    };
    sqlx::query_scalar::<_, String>(
        r#"INSERT INTO memory_entries (
               entry_id, scope_kind, scope_id, key, summary, body, data, tags, citations,
               ttl_seconds, expires_at, stale_after_seconds,
               updated_by_task_id, updated_by_loop_id, updated_by_user_id,
               created_by_task_id, created_by_loop_id, created_by_user_id
           ) VALUES (
               $1, $2, $3, $4, $5, $6, $7, $8, $9,
               $10, now() + ($10::double precision * interval '1 second'), $11,
               $12, $13, $14, $12, $13, $14
           )
           ON CONFLICT (scope_kind, scope_id, key) DO NOTHING
           RETURNING entry_id"#,
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(entry.scope.scope.as_str())
    .bind(&entry.scope.id)
    .bind(&entry.key)
    .bind(summary)
    .bind(entry.body.as_deref().unwrap_or_default())
    .bind(&entry.data)
    .bind(entry.tags.clone().unwrap_or_default())
    .bind(entry.citations.clone().unwrap_or_default())
    .bind(ttl)
    .bind(entry.stale_after_seconds)
    .bind(&entry.writer.task_id)
    .bind(&entry.writer.loop_id)
    .bind(&entry.writer.user_id)
    .fetch_optional(conn)
    .await
}

async fn update_existing(
    conn: &mut PgConnection,
    entry: &PutEntry,
    entry_id: &str,
    current_ttl: Option<i64>,
) -> Result<(), sqlx::Error> {
    let ttl = match entry.ttl {
        TtlChange::Keep => current_ttl,
        TtlChange::Set(seconds) => Some(seconds),
        TtlChange::NoExpiry => None,
    };
    sqlx::query(
        r#"UPDATE memory_entries SET
               summary = COALESCE($2, summary),
               body = COALESCE($3, body),
               data = COALESCE($4, data),
               tags = COALESCE($5, tags),
               citations = COALESCE($6, citations),
               ttl_seconds = $7,
               expires_at = now() + ($7::double precision * interval '1 second'),
               stale_after_seconds = COALESCE($8, stale_after_seconds),
               version = version + 1,
               updated_by_task_id = $9,
               updated_by_loop_id = $10,
               updated_by_user_id = $11,
               updated_at = now()
           WHERE entry_id = $1"#,
    )
    .bind(entry_id)
    .bind(&entry.summary)
    .bind(&entry.body)
    .bind(&entry.data)
    .bind(&entry.tags)
    .bind(&entry.citations)
    .bind(ttl)
    .bind(entry.stale_after_seconds)
    .bind(&entry.writer.task_id)
    .bind(&entry.writer.loop_id)
    .bind(&entry.writer.user_id)
    .execute(conn)
    .await?;
    Ok(())
}

#[async_trait]
impl MemoryDao for PgMemoryDao {
    async fn put(&self, entry: &PutEntry) -> Result<PutOutcome, sqlx::Error> {
        let mut tx = self.0.begin().await?;
        sqlx::query(
            r#"DELETE FROM memory_entries
               WHERE scope_kind = $1 AND scope_id = $2 AND key = $3 AND expires_at <= now()"#,
        )
        .bind(entry.scope.scope.as_str())
        .bind(&entry.scope.id)
        .bind(&entry.key)
        .execute(&mut *tx)
        .await?;

        // Two passes: an insert that loses a race to a concurrent creation
        // waits for it, then updates the row it made.
        for _ in 0..2 {
            let existing = lock_existing(&mut tx, entry).await?;
            let entry_id = if let Some((entry_id, version, current_ttl)) = existing {
                if entry.if_version.is_some_and(|v| v != version) {
                    return Ok(PutOutcome::VersionMismatch {
                        current: Some(version),
                    });
                }
                update_existing(&mut tx, entry, &entry_id, current_ttl).await?;
                entry_id
            } else {
                if entry.if_version.is_some_and(|v| v != 0) {
                    return Ok(PutOutcome::VersionMismatch { current: None });
                }
                let Some(summary) = entry.summary.as_deref() else {
                    return Ok(PutOutcome::SummaryRequired);
                };
                let Some(entry_id) = insert_new(&mut tx, entry, summary).await? else {
                    continue;
                };
                entry_id
            };
            let row = fetch_entry(&mut tx, &entry_id)
                .await?
                .ok_or(sqlx::Error::RowNotFound)?;
            insert_event(
                &mut tx,
                &entry_id,
                MemoryOp::Put,
                row.version,
                Some(&row.summary),
                &entry.writer,
            )
            .await?;
            tx.commit().await?;
            return Ok(PutOutcome::Written(Box::new(row)));
        }
        Err(sqlx::Error::Protocol(format!(
            "memory entry {} kept changing during a put",
            entry.key
        )))
    }

    async fn find(
        &self,
        scopes: &[ScopeRef],
        key: &str,
    ) -> Result<Vec<MemoryEntryRow>, sqlx::Error> {
        let (kinds, ids) = scope_arrays(scopes);
        sqlx::query_as::<_, MemoryEntryRow>(concat!(
            "SELECT ",
            entry_columns!(),
            " FROM memory_entries e WHERE ",
            visible!(),
            " AND e.key = $3"
        ))
        .bind(kinds)
        .bind(ids)
        .bind(key)
        .fetch_all(&self.0)
        .await
    }

    async fn list(
        &self,
        scopes: &[ScopeRef],
        filter: &ListFilter,
    ) -> Result<Vec<MemoryEntryRow>, sqlx::Error> {
        let (kinds, ids) = scope_arrays(scopes);
        sqlx::query_as::<_, MemoryEntryRow>(concat!(
            "SELECT ",
            entry_columns!(),
            " FROM memory_entries e WHERE ",
            visible!(),
            " AND ($3::text IS NULL OR starts_with(e.key, $3))",
            " AND ($4::text IS NULL OR $4 = ANY(e.tags))",
            " AND ($5::timestamptz IS NULL OR e.updated_at >= $5)",
            " ORDER BY e.updated_at DESC, e.key LIMIT $6"
        ))
        .bind(kinds)
        .bind(ids)
        .bind(&filter.prefix)
        .bind(&filter.tag)
        .bind(filter.updated_since)
        .bind(filter.limit)
        .fetch_all(&self.0)
        .await
    }

    async fn search(
        &self,
        scopes: &[ScopeRef],
        query: &str,
        limit: i64,
    ) -> Result<Vec<MemoryEntryRow>, sqlx::Error> {
        let (kinds, ids) = scope_arrays(scopes);
        sqlx::query_as::<_, MemoryEntryRow>(concat!(
            "SELECT ",
            entry_columns!(),
            " FROM memory_entries e WHERE ",
            visible!(),
            " AND e.search @@ websearch_to_tsquery('simple', $3)",
            " ORDER BY ts_rank(e.search, websearch_to_tsquery('simple', $3)) DESC,",
            " e.updated_at DESC LIMIT $4"
        ))
        .bind(kinds)
        .bind(ids)
        .bind(query)
        .bind(limit)
        .fetch_all(&self.0)
        .await
    }

    async fn preload(
        &self,
        scopes: &[ScopeRef],
        keys: &[String],
        prefixes: &[String],
        changed: Option<ChangedSince>,
        limit: i64,
    ) -> Result<Vec<MemoryEntryRow>, sqlx::Error> {
        let (kinds, ids) = scope_arrays(scopes);
        sqlx::query_as::<_, MemoryEntryRow>(concat!(
            "SELECT ",
            entry_columns!(),
            " FROM memory_entries e WHERE ",
            visible!(),
            " AND (e.key = ANY($3) OR EXISTS (",
            "     SELECT 1 FROM unnest($4::text[]) AS p(prefix) WHERE starts_with(e.key, p.prefix)))",
            " AND ($5::timestamptz IS NULL OR",
            "      (e.updated_at > $5 AND e.updated_by_task_id IS DISTINCT FROM $6))",
            " ORDER BY e.updated_at DESC, e.key LIMIT $7"
        ))
        .bind(kinds)
        .bind(ids)
        .bind(keys)
        .bind(prefixes)
        .bind(changed.as_ref().map(|c| c.since))
        .bind(changed.map(|c| c.reader_task_id))
        .bind(limit)
        .fetch_all(&self.0)
        .await
    }

    async fn record_reads(
        &self,
        task_id: &str,
        reads: &[(String, i32)],
    ) -> Result<(), sqlx::Error> {
        let (entry_ids, versions): (Vec<String>, Vec<i32>) = reads.iter().cloned().unzip();
        sqlx::query(
            r#"INSERT INTO memory_reads (entry_id, task_id, version)
               SELECT r.entry_id, $1, r.version
               FROM unnest($2::text[], $3::int[]) AS r(entry_id, version)
               ON CONFLICT (entry_id, task_id, version) DO UPDATE SET read_at = now()"#,
        )
        .bind(task_id)
        .bind(entry_ids)
        .bind(versions)
        .execute(&self.0)
        .await?;
        Ok(())
    }

    async fn append(
        &self,
        entry_id: &str,
        text: &str,
        writer: &Writer,
    ) -> Result<Option<MemoryEntryRow>, sqlx::Error> {
        let mut tx = self.0.begin().await?;
        let version: Option<i32> = sqlx::query_scalar(
            r#"UPDATE memory_entries SET updated_at = now(), updated_by_task_id = $2,
                   updated_by_loop_id = $3, updated_by_user_id = $4
               WHERE entry_id = $1 RETURNING version"#,
        )
        .bind(entry_id)
        .bind(&writer.task_id)
        .bind(&writer.loop_id)
        .bind(&writer.user_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(version) = version else {
            return Ok(None);
        };
        insert_event(
            &mut tx,
            entry_id,
            MemoryOp::Append,
            version,
            Some(text),
            writer,
        )
        .await?;
        let row = fetch_entry(&mut tx, entry_id).await?;
        tx.commit().await?;
        Ok(row)
    }

    async fn verify(
        &self,
        entry_id: &str,
        writer: &Writer,
    ) -> Result<Option<MemoryEntryRow>, sqlx::Error> {
        let mut tx = self.0.begin().await?;
        let version: Option<i32> = sqlx::query_scalar(
            r#"UPDATE memory_entries SET
                   verified_at = now(),
                   expires_at = now() + (ttl_seconds::double precision * interval '1 second')
               WHERE entry_id = $1 RETURNING version"#,
        )
        .bind(entry_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(version) = version else {
            return Ok(None);
        };
        insert_event(&mut tx, entry_id, MemoryOp::Verify, version, None, writer).await?;
        let row = fetch_entry(&mut tx, entry_id).await?;
        tx.commit().await?;
        Ok(row)
    }

    async fn claim(
        &self,
        entry_id: &str,
        ttl_seconds: i64,
        writer: &Writer,
    ) -> Result<Option<ClaimOutcome>, sqlx::Error> {
        let mut tx = self.0.begin().await?;
        let claimed: Option<i32> = sqlx::query_scalar(
            r#"UPDATE memory_entries e SET
                   claimed_by_task_id = $2,
                   claim_expires_at = now() + ($3::double precision * interval '1 second')
               WHERE e.entry_id = $1
                 AND (e.claimed_by_task_id IS NULL
                      OR e.claimed_by_task_id = $2
                      OR e.claim_expires_at <= now()
                      OR NOT EXISTS (SELECT 1 FROM tasks t
                                     WHERE t.task_id = e.claimed_by_task_id
                                       AND t.status = 'open'))
               RETURNING e.version"#,
        )
        .bind(entry_id)
        .bind(&writer.task_id)
        .bind(ttl_seconds)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(row) = fetch_entry(&mut tx, entry_id).await? else {
            return Ok(None);
        };
        let outcome = match claimed {
            Some(version) => {
                insert_event(&mut tx, entry_id, MemoryOp::Claim, version, None, writer).await?;
                ClaimOutcome::Claimed(Box::new(row))
            }
            None => ClaimOutcome::HeldBy(row.claimed_by_task_id.unwrap_or_default()),
        };
        tx.commit().await?;
        Ok(Some(outcome))
    }

    async fn release(&self, entry_id: &str, writer: &Writer) -> Result<bool, sqlx::Error> {
        let mut tx = self.0.begin().await?;
        let version: Option<i32> = sqlx::query_scalar(
            r#"UPDATE memory_entries SET claimed_by_task_id = NULL, claim_expires_at = NULL
               WHERE entry_id = $1 AND claimed_by_task_id = $2 RETURNING version"#,
        )
        .bind(entry_id)
        .bind(&writer.task_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(version) = version else {
            return Ok(false);
        };
        insert_event(&mut tx, entry_id, MemoryOp::Release, version, None, writer).await?;
        tx.commit().await?;
        Ok(true)
    }

    async fn delete(&self, entry_id: &str) -> Result<bool, sqlx::Error> {
        let done = sqlx::query("DELETE FROM memory_entries WHERE entry_id = $1")
            .bind(entry_id)
            .execute(&self.0)
            .await?;
        Ok(done.rows_affected() > 0)
    }

    async fn log(&self, entry_id: &str, limit: i64) -> Result<Vec<MemoryEventRow>, sqlx::Error> {
        sqlx::query_as::<_, MemoryEventRow>(
            r#"SELECT op, version, text, task_id, created_at FROM memory_events
               WHERE entry_id = $1 ORDER BY event_id DESC LIMIT $2"#,
        )
        .bind(entry_id)
        .bind(limit)
        .fetch_all(&self.0)
        .await
    }

    async fn seen_at(&self, task_id: &str) -> Result<SeenAt, sqlx::Error> {
        let (read_at, seen_at) = sqlx::query_as::<_, (DateTime<Utc>, Option<DateTime<Utc>>)>(
            "SELECT now(), (SELECT seen_at FROM memory_seen WHERE task_id = $1)",
        )
        .bind(task_id)
        .fetch_one(&self.0)
        .await?;
        Ok(SeenAt { read_at, seen_at })
    }

    async fn stage_seen(
        &self,
        task_id: &str,
        at: Option<DateTime<Utc>>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"INSERT INTO memory_seen (task_id, staged_at) VALUES ($1, $2)
               ON CONFLICT (task_id) DO UPDATE SET staged_at = EXCLUDED.staged_at"#,
        )
        .bind(task_id)
        .bind(at)
        .execute(&self.0)
        .await?;
        Ok(())
    }

    async fn commit_seen(&self, task_id: &str) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"UPDATE memory_seen SET seen_at = staged_at, staged_at = NULL
               WHERE task_id = $1 AND staged_at IS NOT NULL"#,
        )
        .bind(task_id)
        .execute(&self.0)
        .await?;
        Ok(())
    }
}

#[cfg(all(test, feature = "db-tests"))]
#[path = "dao_db_tests.rs"]
mod dao_db_tests;
