//! Postgres persistence for inbound events waiting out their buffer window.
//!
//! Every state change a buffered event can make lives here as one statement, so
//! the admission, claim, and recovery invariants read in one place rather than
//! spread across the flush loop.

use std::time::Duration;

use chrono::Utc;
use sqlx::Row as _;

use moatless_core::db::DbPool;
use moatless_types::adapter::InboundMessage;

use super::combine::MAX_PARTS_PER_MESSAGE;
use super::policy::QUIET_PERIOD;

/// Attempts a buffered event gets before it is abandoned. Spent per flush of the
/// whole binding, so a poison event cannot hold a subject's later events back
/// forever.
const MAX_ATTEMPTS: i32 = 5;

/// Delay added to `flush_at` after a failed flush.
const RETRY_BACKOFF: Duration = Duration::from_secs(30);

/// How long a flush claim is honoured before any worker may take it back.
const CLAIM_LEASE: Duration = Duration::from_mins(5);

/// The timestamp arithmetic here is in `chrono`, the durations are in `std`.
/// Only this file's minute-scale constants are converted, so the out-of-range
/// case cannot arise.
fn delta(duration: Duration) -> chrono::TimeDelta {
    chrono::TimeDelta::from_std(duration).unwrap_or_else(|_| chrono::TimeDelta::zero())
}

/// One binding's buffered events, claimed together and delivered as one turn.
pub(crate) struct BufferedBatch {
    pub(crate) row_ids: Vec<String>,
    pub(crate) binding_id: String,
    pub(crate) loop_id: String,
    pub(crate) adapter_label: String,
    pub(crate) messages: Vec<InboundMessage>,
    /// The highest attempt count among the claimed rows: this flush's budget.
    pub(crate) attempts: i32,
}

impl BufferedBatch {
    pub(crate) fn is_last_attempt(&self) -> bool {
        self.attempts >= MAX_ATTEMPTS
    }
}

/// One claimed row, before rows are grouped into batches.
struct ClaimedRow {
    id: String,
    seq: i64,
    binding_id: String,
    loop_id: String,
    adapter_label: String,
    message: InboundMessage,
    attempts: i32,
}

/// The `inbound_message_buffer` table. INVARIANT: must stay a stateless wrapper
/// over the pool — every backend replica writes it, so in-memory batching here
/// would hold events one replica's flusher never sees.
pub(crate) struct MessageBufferStore {
    pool: DbPool,
}

impl MessageBufferStore {
    pub(crate) fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    /// Hold `message` for `binding_id`, joining the window already open there.
    ///
    /// Moves every unclaimed row to one deadline: [`QUIET_PERIOD`] from now,
    /// capped at `window` after the window opened so a steady stream cannot
    /// starve the flush. Never moves earlier, so a [`RETRY_BACKOFF`] holds.
    pub(crate) async fn push(
        &self,
        binding_id: &str,
        loop_id: &str,
        adapter_label: &str,
        message: &InboundMessage,
        window: Duration,
    ) -> Result<(), sqlx::Error> {
        let quiet = QUIET_PERIOD.min(window);
        sqlx::query(
            r#"WITH open_window AS (
                   SELECT MIN(flush_at) AS flush_at, MIN(created_at) AS opened_at
                   FROM inbound_message_buffer
                   WHERE binding_id = $2 AND claimed_at IS NULL
               ),
               deadline AS (
                   SELECT CASE WHEN opened_at IS NULL
                              THEN NOW() + make_interval(secs => $6)
                              ELSE GREATEST(
                                  flush_at,
                                  LEAST(opened_at + make_interval(secs => $7),
                                        NOW() + make_interval(secs => $6))
                              )
                          END AS flush_at
                   FROM open_window
               ),
               extended AS (
                   UPDATE inbound_message_buffer b SET flush_at = deadline.flush_at
                   FROM deadline
                   WHERE b.binding_id = $2 AND b.claimed_at IS NULL
               )
               INSERT INTO inbound_message_buffer
                   (id, binding_id, loop_id, adapter_label, message, flush_at)
               SELECT $1, $2, $3, $4, $5, deadline.flush_at FROM deadline"#,
        )
        .bind(format!("imb_{}", uuid::Uuid::new_v4()))
        .bind(binding_id)
        .bind(loop_id)
        .bind(adapter_label)
        .bind(sqlx::types::Json(message))
        .bind(quiet.as_secs_f64())
        .bind(window.as_secs_f64())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Hold `message` in the window already open on `binding_id`, keeping its
    /// deadline; `false`, writing nothing, when none is open. An event that is
    /// not held on its own still lands after, and with, the burst ahead of it.
    pub(crate) async fn join_open_window(
        &self,
        binding_id: &str,
        loop_id: &str,
        adapter_label: &str,
        message: &InboundMessage,
    ) -> Result<bool, sqlx::Error> {
        let joined = sqlx::query(
            r#"INSERT INTO inbound_message_buffer
                   (id, binding_id, loop_id, adapter_label, message, flush_at)
               SELECT $1, $2, $3, $4, $5, MIN(flush_at)
               FROM inbound_message_buffer
               WHERE binding_id = $2 AND claimed_at IS NULL
               HAVING COUNT(*) > 0"#,
        )
        .bind(format!("imb_{}", uuid::Uuid::new_v4()))
        .bind(binding_id)
        .bind(loop_id)
        .bind(adapter_label)
        .bind(sqlx::types::Json(message))
        .execute(&self.pool)
        .await?;
        Ok(joined.rows_affected() > 0)
    }

    /// Claim the buffered events of up to `bindings` due bindings, counting the
    /// attempt.
    ///
    /// A due binding is emptied up to [`MAX_PARTS_PER_MESSAGE`], applied before
    /// the claim so a row past the cap keeps its full attempt budget for the
    /// poll that does flush it.
    pub(crate) async fn claim_due(
        &self,
        owner: &str,
        bindings: i64,
    ) -> Result<Vec<BufferedBatch>, sqlx::Error> {
        let rows = sqlx::query(
            r#"WITH ranked AS (
                   SELECT id, row_number() OVER (
                              PARTITION BY binding_id ORDER BY seq
                          ) AS rank
                   FROM inbound_message_buffer
                   WHERE claimed_at IS NULL
                     AND binding_id IN (
                         SELECT binding_id FROM inbound_message_buffer
                         WHERE claimed_at IS NULL AND flush_at <= NOW()
                         GROUP BY binding_id
                         ORDER BY MIN(flush_at)
                         LIMIT $2
                     )
               ),
               due AS (
                   -- Not redundant with `ranked`: `SKIP LOCKED` skips only a row
                   -- another claim still holds, so this is what EvalPlanQual
                   -- re-tests against one that committed after this statement's
                   -- snapshot. Drop it and two flushers claim the same events.
                   SELECT id FROM inbound_message_buffer
                   WHERE claimed_at IS NULL
                     AND id IN (SELECT id FROM ranked WHERE rank <= $3)
                   FOR UPDATE SKIP LOCKED
               )
               UPDATE inbound_message_buffer b SET
                   claimed_at = NOW(),
                   claimed_by = $1,
                   attempts   = attempts + 1
               FROM due
               WHERE b.id = due.id
               RETURNING b.id, b.seq, b.binding_id, b.loop_id, b.adapter_label,
                         b.message, b.attempts"#,
        )
        .bind(owner)
        .bind(bindings)
        .bind(i64::try_from(MAX_PARTS_PER_MESSAGE).unwrap_or(i64::MAX))
        .fetch_all(&self.pool)
        .await?;

        let mut claimed: Vec<ClaimedRow> = rows
            .into_iter()
            .map(|row| {
                Ok(ClaimedRow {
                    id: row.try_get("id")?,
                    seq: row.try_get("seq")?,
                    binding_id: row.try_get("binding_id")?,
                    loop_id: row.try_get("loop_id")?,
                    adapter_label: row.try_get("adapter_label")?,
                    message: row
                        .try_get::<sqlx::types::Json<InboundMessage>, _>("message")?
                        .0,
                    attempts: row.try_get("attempts")?,
                })
            })
            .collect::<Result<_, sqlx::Error>>()?;
        // `UPDATE ... RETURNING` has no order of its own, and the combined
        // message reads oldest first.
        claimed.sort_by(|a, b| (&a.binding_id, a.seq).cmp(&(&b.binding_id, b.seq)));

        let mut batches: Vec<BufferedBatch> = Vec::new();
        for row in claimed {
            match batches.last_mut() {
                Some(batch) if batch.binding_id == row.binding_id => {
                    batch.attempts = batch.attempts.max(row.attempts);
                    batch.row_ids.push(row.id);
                    batch.messages.push(row.message);
                }
                _ => batches.push(BufferedBatch {
                    row_ids: vec![row.id],
                    binding_id: row.binding_id,
                    loop_id: row.loop_id,
                    adapter_label: row.adapter_label,
                    messages: vec![row.message],
                    attempts: row.attempts,
                }),
            }
        }

        Ok(batches)
    }

    /// Drop the rows a flush delivered. Terminal — the combined turn is already
    /// on the task's transcript, and nothing deduplicates against these rows.
    pub(crate) async fn delete(&self, row_ids: &[String]) -> Result<(), sqlx::Error> {
        if row_ids.is_empty() {
            return Ok(());
        }
        sqlx::query("DELETE FROM inbound_message_buffer WHERE id = ANY($1)")
            .bind(row_ids)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Return a failed flush's rows to the pool behind [`RETRY_BACKOFF`].
    pub(crate) async fn reschedule(&self, row_ids: &[String]) -> Result<(), sqlx::Error> {
        if row_ids.is_empty() {
            return Ok(());
        }
        let flush_at = Utc::now() + delta(RETRY_BACKOFF);
        sqlx::query(
            r#"UPDATE inbound_message_buffer
               SET claimed_at = NULL, claimed_by = NULL, flush_at = $2
               WHERE id = ANY($1)"#,
        )
        .bind(row_ids)
        .bind(flush_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Return this owner's claims to the pool immediately, without waiting out
    /// the lease — the flusher calls it once at startup.
    ///
    /// The attempt already counted is deliberately not refunded, so a batch that
    /// crash-loops the backend is abandoned rather than retried forever.
    pub(crate) async fn release_claims_by_owner(&self, owner: &str) -> Result<u64, sqlx::Error> {
        let result = sqlx::query(
            r#"UPDATE inbound_message_buffer
               SET claimed_at = NULL, claimed_by = NULL
               WHERE claimed_at IS NOT NULL AND claimed_by = $1"#,
        )
        .bind(owner)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    /// Return claims whose lease has expired, covering the owner that never
    /// comes back: a pod replaced under a new name, or a wedged flusher.
    pub(crate) async fn release_expired_claims(&self) -> Result<u64, sqlx::Error> {
        let deadline = Utc::now() - delta(CLAIM_LEASE);
        let result = sqlx::query(
            r#"UPDATE inbound_message_buffer
               SET claimed_at = NULL, claimed_by = NULL
               WHERE claimed_at IS NOT NULL AND claimed_at < $1"#,
        )
        .bind(deadline)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }
}

#[cfg(all(test, feature = "db-tests"))]
mod tests {
    use moatless_types::adapter::{AdapterKind, Audience};
    use sqlx::PgPool;

    use super::*;

    const BINDING: &str = "tb_buffer_test";
    const OTHER_BINDING: &str = "tb_buffer_test_other";
    const LOOP_ID: &str = "lp_buffer_test";
    const OWNER: &str = "flusher-a";

    /// Seed the rows `inbound_message_buffer.binding_id` points at. The Binding
    /// carries the real foreign keys, so its connection comes first.
    async fn seed_bindings(pool: &PgPool) {
        sqlx::query(
            r#"INSERT INTO adapter_connections
                   (id, adapter_kind, connection_kind, external_account_id, created_by, scope)
               VALUES ('ac_buf', 'github_pr', 'app', 'acme', 'user-1', 'global')"#,
        )
        .execute(pool)
        .await
        .expect("connection seeds");
        for binding in [BINDING, OTHER_BINDING] {
            sqlx::query(
                r#"INSERT INTO task_bindings
                       (id, task_id, adapter_kind, subject_kind, subject_key, connection_id)
                   VALUES ($1, 'task-1', 'github_pr', 'pull_request', $1, 'ac_buf')"#,
            )
            .bind(binding)
            .execute(pool)
            .await
            .expect("binding seeds");
        }
    }

    fn message(text: &str) -> InboundMessage {
        InboundMessage {
            adapter_kind: AdapterKind::GithubPr,
            source_id: "acme/repo".to_string(),
            thread_id: None,
            message_thread_id: "pr:1".to_string(),
            user_id: "user-1".to_string(),
            author: Some("octocat".to_string()),
            sender_external_id: None,
            requester: None,
            text: text.to_string(),
            file_ids: vec![],
            attachments: Vec::new(),
            thread_properties: serde_json::Value::Null,
            adapter_context: serde_json::Value::Null,
            external_event_id: Some(format!("gh:{text}")),
            source_task_id: None,
            audience: Audience::Everyone,
            replies_to: Vec::new(),
            event: None,
        }
    }

    async fn store(pool: PgPool) -> MessageBufferStore {
        seed_bindings(&pool).await;
        MessageBufferStore::new(pool)
    }

    async fn push(store: &MessageBufferStore, binding: &str, text: &str, window: Duration) {
        store
            .push(binding, LOOP_ID, "github_pr", &message(text), window)
            .await
            .expect("push admits the event");
    }

    fn texts(batch: &BufferedBatch) -> Vec<&str> {
        batch.messages.iter().map(|m| m.text.as_str()).collect()
    }

    /// The feature itself: a burst on one Binding leaves as one batch, oldest
    /// first, and a second Binding's events stay their own.
    #[sqlx::test]
    async fn one_binding_s_due_events_claim_together_oldest_first(pool: PgPool) {
        let store = store(pool).await;
        push(&store, BINDING, "first", Duration::ZERO).await;
        push(&store, BINDING, "second", Duration::ZERO).await;
        push(&store, OTHER_BINDING, "elsewhere", Duration::ZERO).await;

        let batches = store.claim_due(OWNER, 10).await.expect("claim succeeds");

        assert_eq!(batches.len(), 2, "each binding is its own batch");
        let ours = batches
            .iter()
            .find(|b| b.binding_id == BINDING)
            .expect("the busy binding is claimed");
        assert_eq!(texts(ours), ["first", "second"]);
        assert_eq!(ours.attempts, 1, "claiming spends one attempt");
        assert_eq!(ours.loop_id, LOOP_ID);
    }

    /// Seconds from the window's opening event, and from its newest event, to
    /// each unclaimed row's deadline, oldest row first.
    async fn deadlines(pool: &PgPool) -> Vec<(f64, f64)> {
        sqlx::query_as(
            r#"SELECT EXTRACT(EPOCH FROM flush_at - MIN(created_at) OVER ())::float8,
                      EXTRACT(EPOCH FROM flush_at - MAX(created_at) OVER ())::float8
               FROM inbound_message_buffer
               WHERE binding_id = $1 AND claimed_at IS NULL
               ORDER BY seq"#,
        )
        .bind(BINDING)
        .fetch_all(pool)
        .await
        .expect("deadlines read")
    }

    /// Move the binding's rows `seconds` into the past, as if they had waited.
    async fn age(pool: &PgPool, seconds: f64) {
        sqlx::query(
            r#"UPDATE inbound_message_buffer
               SET created_at = created_at - make_interval(secs => $2),
                   flush_at = flush_at - make_interval(secs => $2)
               WHERE binding_id = $1"#,
        )
        .bind(BINDING)
        .bind(seconds)
        .execute(pool)
        .await
        .expect("rows age");
    }

    /// The point of the quiet period: a lone event is not held for the window.
    #[sqlx::test]
    async fn a_lone_event_waits_only_the_quiet_period(pool: PgPool) {
        let store = store(pool.clone()).await;
        push(&store, BINDING, "alone", Duration::from_mins(1)).await;

        let quiet = QUIET_PERIOD.as_secs_f64();
        assert_eq!(deadlines(&pool).await, [(quiet, quiet)]);
    }

    /// Each event of a burst restarts the quiet period for the whole window.
    #[sqlx::test]
    async fn a_later_event_extends_the_open_window(pool: PgPool) {
        let store = store(pool.clone()).await;
        push(&store, BINDING, "opens the window", Duration::from_mins(1)).await;
        age(&pool, 8.0).await;
        push(&store, BINDING, "extends it", Duration::from_mins(1)).await;

        let since_newest: Vec<f64> = deadlines(&pool).await.iter().map(|d| d.1).collect();
        assert_eq!(since_newest, [QUIET_PERIOD.as_secs_f64(); 2]);
    }

    /// The window caps the extension, so a stream of events cannot push the
    /// flush out indefinitely.
    #[sqlx::test]
    async fn the_window_caps_the_extension(pool: PgPool) {
        let store = store(pool.clone()).await;
        push(&store, BINDING, "opens the window", Duration::from_secs(30)).await;
        age(&pool, 25.0).await;
        push(&store, BINDING, "near the cap", Duration::from_secs(30)).await;

        let since_opened: Vec<f64> = deadlines(&pool).await.iter().map(|d| d.0).collect();
        assert_eq!(since_opened, [30.0, 30.0]);
    }

    /// A later event never brings the open deadline forward, so it cannot cut
    /// short a failed flush's backoff.
    #[sqlx::test]
    async fn a_later_event_never_shortens_the_open_window(pool: PgPool) {
        let store = store(pool).await;
        push(&store, BINDING, "opens the window", Duration::from_mins(10)).await;
        push(&store, BINDING, "would be due now", Duration::ZERO).await;

        assert!(
            store
                .claim_due(OWNER, 10)
                .await
                .expect("claim succeeds")
                .is_empty(),
            "the second event must wait out the window the first opened"
        );
    }

    /// An event arriving while the previous batch is mid-flush opens a fresh
    /// window. Reusing the in-flight rows' past `flush_at` would deliver it on
    /// its own, unbuffered.
    #[sqlx::test]
    async fn an_event_arriving_mid_flush_opens_a_new_window(pool: PgPool) {
        let store = store(pool).await;
        push(&store, BINDING, "first", Duration::ZERO).await;
        store.claim_due(OWNER, 10).await.expect("claim succeeds");

        push(&store, BINDING, "second", Duration::from_mins(10)).await;

        assert!(
            store
                .claim_due(OWNER, 10)
                .await
                .expect("claim succeeds")
                .is_empty(),
            "the new event owns a fresh window, not the claimed rows' deadline"
        );
    }

    /// Past the cap the surplus stays unclaimed, so the next poll delivers it as
    /// a second readable message.
    #[sqlx::test]
    async fn events_past_the_cap_are_left_for_the_next_claim(pool: PgPool) {
        let store = store(pool).await;
        for index in 0..MAX_PARTS_PER_MESSAGE + 3 {
            push(&store, BINDING, &format!("event {index}"), Duration::ZERO).await;
        }

        let first = store.claim_due(OWNER, 10).await.expect("claim succeeds");
        assert_eq!(first[0].messages.len(), MAX_PARTS_PER_MESSAGE);
        assert_eq!(first[0].row_ids.len(), MAX_PARTS_PER_MESSAGE);

        store
            .delete(&first[0].row_ids)
            .await
            .expect("flushed rows drop");
        let second = store.claim_due(OWNER, 10).await.expect("claim succeeds");
        assert_eq!(texts(&second[0]), ["event 25", "event 26", "event 27"]);
        assert_eq!(
            second[0].attempts, 1,
            "a row past the cap was never flushed, so it keeps its whole attempt budget"
        );
    }

    /// A failed flush spends attempts until the batch is abandoned, so a poison
    /// event cannot be retried forever.
    #[sqlx::test]
    async fn attempts_accumulate_across_rescheduled_flushes(pool: PgPool) {
        let store = store(pool).await;
        push(&store, BINDING, "poison", Duration::ZERO).await;

        for attempt in 1..=MAX_ATTEMPTS {
            let batch = store
                .claim_due(OWNER, 10)
                .await
                .expect("claim succeeds")
                .pop()
                .expect("the due binding is claimed");
            assert_eq!(batch.attempts, attempt);
            assert_eq!(batch.is_last_attempt(), attempt == MAX_ATTEMPTS);
            // Due again immediately, so the test does not wait out the backoff.
            store
                .release_claims_by_owner(OWNER)
                .await
                .expect("rows return");
        }
    }

    /// Startup recovery: rows a killed flusher still holds are claimable again
    /// under the same owner id, without waiting out the lease.
    #[sqlx::test]
    async fn a_flusher_recovers_the_claims_it_left_behind(pool: PgPool) {
        let store = store(pool).await;
        push(&store, BINDING, "in flight", Duration::ZERO).await;
        store.claim_due(OWNER, 10).await.expect("claim succeeds");

        assert_eq!(
            store
                .release_claims_by_owner(OWNER)
                .await
                .expect("recovery succeeds"),
            1
        );
        let batch = store.claim_due(OWNER, 10).await.expect("claim succeeds");
        assert_eq!(texts(&batch[0]), ["in flight"]);
        assert_eq!(
            batch[0].attempts, 2,
            "recovery must not refund the spent attempt"
        );
    }

    /// Deleting a Binding takes its buffered events with it, so a closed subject
    /// cannot leave rows the flusher retries against nothing.
    #[sqlx::test]
    async fn deleting_the_binding_drops_its_buffered_events(pool: PgPool) {
        let store = store(pool.clone()).await;
        push(&store, BINDING, "orphan", Duration::ZERO).await;

        sqlx::query("DELETE FROM task_bindings WHERE id = $1")
            .bind(BINDING)
            .execute(&pool)
            .await
            .expect("binding deletes");

        assert!(
            store
                .claim_due(OWNER, 10)
                .await
                .expect("claim succeeds")
                .is_empty()
        );
    }
}
