//! The durable ingress outbox: storage for admitted dispatch payloads.
//!
//! A record lives from durable admission until its consumer commits the message
//! it carries. See the module docs on [`super`] for where this sits in ADR-0006.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::db::DbPool;

/// Base delay before the relay's first re-publish of a record, and the factor
/// its backoff doubles from. Long enough that the ingress publish (and the
/// consumer's commit of it) normally wins the race and deletes the record
/// before the relay ever looks at it.
pub(super) const RETRY_BASE_SECS: i32 = 30;

/// Cap on the relay's per-record backoff. A record that has been failing for
/// hours still gets retried, just not hot.
pub(super) const RETRY_CAP_SECS: i32 = 15 * 60;

/// A dispatch payload durably admitted at the gRPC ingress.
///
/// Mirrors the fields the published event carries so the relay can reproduce
/// the original publication byte-for-byte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngressPayload {
    pub event_id: String,
    pub task_id: String,
    pub event_type: String,
    pub sub_type: String,
    pub turn_number: Option<i32>,
    pub payload: Value,
    pub timestamp: String,
}

/// A record the relay has claimed for re-publication.
#[derive(Debug, Clone)]
pub struct ClaimedPayload {
    pub payload: IngressPayload,
    /// Relay attempts made before this one (0 on the first claim).
    pub attempts: i32,
    /// When the ingress admitted the record — the age of an undelivered payload.
    pub admitted_at: DateTime<Utc>,
}

/// Persistence for the transactional ingress outbox.
///
/// Two consumers justify the seam: the gRPC ingress (admit) and the relay
/// (claim/republish), plus the event-handler consumer's delete. Unit tests of
/// admission and of the relay drive it through fakes.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait IngressOutbox: Send + Sync {
    /// Durably admit `payload`, scheduling its first relay attempt for
    /// `first_attempt_at`. Idempotent on `event_id`: a retried delivery of an
    /// already-admitted event neither duplicates the record nor resets its
    /// retry schedule.
    async fn admit(
        &self,
        payload: &IngressPayload,
        first_attempt_at: DateTime<Utc>,
    ) -> Result<(), sqlx::Error>;

    /// Drop the record for `event_id` because its content is now committed to
    /// its canonical table. The **only** delete: a record is removed on
    /// confirmed delivery and never on a publish that was merely attempted.
    /// Returns whether a record was present.
    async fn delete_delivered(&self, event_id: &str) -> Result<bool, sqlx::Error>;

    /// Claim up to `limit` records whose next attempt is due at `now`,
    /// rescheduling each with exponential backoff in the same statement so
    /// concurrent relays never claim the same record.
    async fn claim_due(
        &self,
        now: DateTime<Utc>,
        limit: i64,
    ) -> Result<Vec<ClaimedPayload>, sqlx::Error>;

    /// Records still awaiting confirmed delivery, and how many of those were
    /// admitted before `stale_before`. The relay's observability read.
    async fn pending_counts(&self, stale_before: DateTime<Utc>) -> Result<Pending, sqlx::Error>;
}

/// Outstanding-record counts reported by [`IngressOutbox::pending_counts`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Pending {
    pub total: i64,
    /// Of `total`, those admitted before the staleness threshold — payloads
    /// whose consumer has not committed them for far longer than one retry.
    pub stale: i64,
}

/// PostgreSQL implementation of [`IngressOutbox`].
pub struct PgIngressOutbox(pub DbPool);

/// Row shape shared by the claim query.
type ClaimedRow = (
    String,
    String,
    String,
    String,
    Option<i32>,
    Value,
    String,
    i32,
    DateTime<Utc>,
);

#[async_trait]
impl IngressOutbox for PgIngressOutbox {
    async fn admit(
        &self,
        payload: &IngressPayload,
        first_attempt_at: DateTime<Utc>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"INSERT INTO event_ingress_outbox (
                   event_id, task_id, event_type, sub_type,
                   turn_number, payload, timestamp, next_attempt_at
               ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
               ON CONFLICT (event_id) DO NOTHING"#,
        )
        .bind(&payload.event_id)
        .bind(&payload.task_id)
        .bind(&payload.event_type)
        .bind(&payload.sub_type)
        .bind(payload.turn_number)
        .bind(&payload.payload)
        .bind(&payload.timestamp)
        .bind(first_attempt_at)
        .execute(&self.0)
        .await?;
        Ok(())
    }

    async fn delete_delivered(&self, event_id: &str) -> Result<bool, sqlx::Error> {
        let deleted = sqlx::query("DELETE FROM event_ingress_outbox WHERE event_id = $1")
            .bind(event_id)
            .execute(&self.0)
            .await?
            .rows_affected();
        Ok(deleted > 0)
    }

    async fn claim_due(
        &self,
        now: DateTime<Utc>,
        limit: i64,
    ) -> Result<Vec<ClaimedPayload>, sqlx::Error> {
        // Claim-and-reschedule in one statement. `FOR UPDATE SKIP LOCKED` plus
        // the immediate `next_attempt_at` push means a second replica's relay
        // pass sees neither the locked rows nor the just-claimed ones.
        let rows: Vec<ClaimedRow> = sqlx::query_as(
            r#"UPDATE event_ingress_outbox o
                  SET attempts = o.attempts + 1,
                      next_attempt_at = $1 + make_interval(
                          secs => LEAST($2 * POWER(2, LEAST(o.attempts, 8)), $3)
                      )
                WHERE o.event_id IN (
                    SELECT event_id FROM event_ingress_outbox
                     WHERE next_attempt_at <= $1
                     ORDER BY next_attempt_at
                     LIMIT $4
                     FOR UPDATE SKIP LOCKED
                )
            RETURNING o.event_id, o.task_id, o.event_type, o.sub_type,
                      o.turn_number, o.payload, o.timestamp, o.attempts, o.admitted_at"#,
        )
        .bind(now)
        .bind(f64::from(RETRY_BASE_SECS))
        .bind(f64::from(RETRY_CAP_SECS))
        .bind(limit)
        .fetch_all(&self.0)
        .await?;

        Ok(rows
            .into_iter()
            .map(
                |(
                    event_id,
                    task_id,
                    event_type,
                    sub_type,
                    turn_number,
                    payload,
                    timestamp,
                    attempts,
                    admitted_at,
                )| ClaimedPayload {
                    payload: IngressPayload {
                        event_id,
                        task_id,
                        event_type,
                        sub_type,
                        turn_number,
                        payload,
                        timestamp,
                    },
                    // The UPDATE already counted this claim; report attempts
                    // made *before* it.
                    attempts: attempts.saturating_sub(1),
                    admitted_at,
                },
            )
            .collect())
    }

    async fn pending_counts(&self, stale_before: DateTime<Utc>) -> Result<Pending, sqlx::Error> {
        let (total, stale): (i64, i64) = sqlx::query_as(
            r#"SELECT COUNT(*), COUNT(*) FILTER (WHERE admitted_at < $1)
                 FROM event_ingress_outbox"#,
        )
        .bind(stale_before)
        .fetch_one(&self.0)
        .await?;
        Ok(Pending { total, stale })
    }
}
