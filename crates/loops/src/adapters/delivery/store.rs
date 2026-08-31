//! Postgres persistence for admitted Adapter deliveries.
//!
//! Every state change a record can make lives here as one statement, so the
//! admission, claim, and recovery invariants are readable in one place rather
//! than spread across the worker loop.

use chrono::{DateTime, Utc};
use sqlx::Row;

use moatless_core::db::DbPool;
use moatless_types::store::StoreError;

use super::metrics::record_admission;
use super::models::AdapterDeliveryPolicy;
use moatless_adapter_model::delivery::{
    AdapterDelivery, AdapterDeliveryRequest, Admission, AdmitError, DeliveryAdmission,
};

/// Unfinished and finished record counts, for the worker's backlog gauges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Backlog {
    pub(crate) pending: i64,
    pub(crate) claimed: i64,
    pub(crate) failed: i64,
    /// Age of the oldest record still waiting, in seconds. Zero when nothing is
    /// waiting. This is the number that says "the worker is gone" when the
    /// worker's own liveness signal cannot.
    pub(crate) oldest_pending_secs: i64,
}

/// The durable Adapter delivery store.
///
/// Most webhook handlers reach this through `AppState::adapter_delivery` and
/// only ever call [`DeliveryAdmission::admit`]; the remaining operations belong
/// to the worker that owns admitted records.
///
/// INVARIANT — must stay a stateless wrapper over the pool. A provider adapter
/// (Telegram today, see `TelegramAdapter::with_ingress_ports`) may construct
/// its own instance at the composition root rather than share the one on
/// `AppState`, which is sound only because every instance is `pool` + a fixed
/// `policy` and admission's capacity guard rides the DB (`SELECT COUNT(*)`), so
/// two instances never diverge. Any in-memory state added here — a dedup cache,
/// a counter, a notify channel — breaks that: it would have to be built once
/// and shared, not rebuilt per provider.
pub struct AdapterDeliveryStore {
    pool: DbPool,
    policy: AdapterDeliveryPolicy,
}

impl AdapterDeliveryStore {
    /// `pub` so a composition root outside this crate can build one: an adapter
    /// that carries its own ingress ports (see
    /// `linear::adapter::LinearAdapter::with_ingress_ports`) is wired at the
    /// composition root, and the integration tests are composition roots too.
    /// Its sibling ports (`PgAdapterConnectionDao::new`, `LoopDao::new`) are
    /// already `pub` for the same reason.
    pub fn new(pool: DbPool) -> Self {
        Self {
            pool,
            policy: AdapterDeliveryPolicy::default(),
        }
    }

    /// Build a store with a non-default policy, so the DB tests can collapse
    /// the retry, lease, and capacity numbers instead of waiting ten minutes
    /// for the real ones. The running system always uses
    /// [`AdapterDeliveryPolicy::default`] — the policy is a decision, not a knob.
    #[cfg(all(test, feature = "db-tests"))]
    pub(crate) fn with_policy(pool: DbPool, policy: AdapterDeliveryPolicy) -> Self {
        Self { pool, policy }
    }

    pub(crate) fn policy(&self) -> &AdapterDeliveryPolicy {
        &self.policy
    }

    async fn insert(&self, request: AdapterDeliveryRequest) -> Result<Admission, AdmitError> {
        let id = format!("ad_{}", uuid::Uuid::new_v4());
        // The capacity predicate is evaluated against this statement's own READ
        // COMMITTED snapshot, so concurrent admissions can each see the same
        // below-capacity backlog and all insert. That is deliberate, and the
        // resulting bound is still hard rather than approximate — see
        // `AdapterDeliveryPolicy::backlog_capacity` for the arithmetic and for why
        // serialising the decision would cost more than the overshoot does.
        let inserted = sqlx::query(
            r#"INSERT INTO adapter_delivery
                   (id, adapter, kind, event_id, payload, status, attempts, max_attempts,
                    next_attempt_at, admitted_at, updated_at)
               SELECT $1, $2, $3, $4, $5, 'pending', 0, $6, NOW(), NOW(), NOW()
               WHERE (
                   SELECT COUNT(*) FROM adapter_delivery
                   WHERE status IN ('pending', 'claimed')
               ) < $7
               ON CONFLICT (adapter, kind, event_id) DO NOTHING
               RETURNING id"#,
        )
        .bind(&id)
        .bind(request.adapter)
        .bind(request.kind)
        .bind(&request.event_id)
        .bind(&request.payload)
        .bind(self.policy.max_attempts)
        .bind(self.policy.backlog_capacity)
        .fetch_optional(&self.pool)
        .await
        .map_err(admission_failed)?;

        if inserted.is_some() {
            return Ok(Admission::Admitted);
        }

        // Nothing was written: either the event is already admitted, or the
        // capacity guard rejected it. Only this rare path pays a second query.
        let duplicate = sqlx::query(
            r#"SELECT 1 FROM adapter_delivery
               WHERE adapter = $1 AND kind = $2 AND event_id = $3"#,
        )
        .bind(request.adapter)
        .bind(request.kind)
        .bind(&request.event_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(admission_failed)?;

        if duplicate.is_some() {
            Ok(Admission::Duplicate)
        } else {
            Err(AdmitError::AtCapacity {
                limit: self.policy.backlog_capacity,
            })
        }
    }

    /// Claim up to `batch_size` due records for `owner`, counting the attempt.
    ///
    /// Only records whose `kind` has a handler in this process are claimed. A
    /// replica running an older build therefore leaves deliveries it cannot execute
    /// for one that can, instead of burning its attempt budget during a rolling
    /// deploy. `SKIP LOCKED` keeps concurrent replicas off each other's rows.
    pub(crate) async fn claim_due(
        &self,
        owner: &str,
        kinds: &[&'static str],
    ) -> Result<Vec<AdapterDelivery>, sqlx::Error> {
        if kinds.is_empty() {
            return Ok(Vec::new());
        }
        let kinds: Vec<String> = kinds.iter().map(|k| (*k).to_string()).collect();
        sqlx::query_as::<_, AdapterDelivery>(
            r#"UPDATE adapter_delivery SET
                   status      = 'claimed',
                   attempts    = attempts + 1,
                   claimed_at  = NOW(),
                   claimed_by  = $1,
                   updated_at  = NOW()
               WHERE id IN (
                   SELECT id FROM adapter_delivery
                   WHERE status = 'pending'
                     AND next_attempt_at <= NOW()
                     AND kind = ANY($2)
                   ORDER BY next_attempt_at
                   FOR UPDATE SKIP LOCKED
                   LIMIT $3
               )
               RETURNING id, adapter, kind, event_id, payload, attempts, max_attempts"#,
        )
        .bind(owner)
        .bind(&kinds)
        .bind(self.policy.batch_size)
        .fetch_all(&self.pool)
        .await
    }

    /// Mark a record done. Terminal, and the row is kept so a late duplicate
    /// delivery still deduplicates against it.
    pub(crate) async fn mark_succeeded(&self, id: &str) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"UPDATE adapter_delivery SET
                   status     = 'succeeded',
                   claimed_at = NULL,
                   claimed_by = NULL,
                   last_error = NULL,
                   updated_at = NOW()
               WHERE id = $1"#,
        )
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Record a failed attempt.
    ///
    /// `terminal` ends the record now — the handler called the failure
    /// permanent, or this attempt spent the last of the budget. Otherwise the
    /// record returns to `pending` behind its backoff. Either way `last_error`
    /// is retained: a terminal record has to say why it was abandoned.
    pub(crate) async fn mark_attempt_failed(
        &self,
        id: &str,
        terminal: bool,
        attempts: i32,
        error: &str,
    ) -> Result<(), sqlx::Error> {
        let next_attempt_at = Utc::now()
            + chrono::Duration::from_std(self.policy.backoff(attempts))
                .unwrap_or(chrono::Duration::zero());
        sqlx::query(
            r#"UPDATE adapter_delivery SET
                   status          = CASE WHEN $2 THEN 'failed' ELSE 'pending' END,
                   next_attempt_at = $3,
                   claimed_at      = NULL,
                   claimed_by      = NULL,
                   last_error      = $4,
                   updated_at      = NOW()
               WHERE id = $1"#,
        )
        .bind(id)
        .bind(terminal)
        .bind(next_attempt_at)
        .bind(truncate_error(error))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Return this owner's claims to `pending` immediately.
    ///
    /// Called once at worker startup. A process that was killed mid-execution
    /// left claims behind under the same stable owner id, so its work resumes
    /// on restart without waiting out the lease.
    ///
    /// The attempt the claim already counted is deliberately not refunded: a
    /// record whose execution kills the process still spends its budget, so a
    /// payload that crash-loops the backend goes terminal instead of restarting
    /// the process forever.
    pub(crate) async fn release_claims_by_owner(&self, owner: &str) -> Result<u64, sqlx::Error> {
        let result = sqlx::query(
            r#"UPDATE adapter_delivery SET
                   status     = 'pending',
                   claimed_at = NULL,
                   claimed_by = NULL,
                   updated_at = NOW()
               WHERE status = 'claimed' AND claimed_by = $1"#,
        )
        .bind(owner)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    /// Return claims whose lease has expired to `pending`.
    ///
    /// Covers the owner that never comes back — a pod replaced under a new
    /// name, or a worker wedged past its lease.
    pub(crate) async fn release_expired_claims(&self) -> Result<u64, sqlx::Error> {
        let deadline = Utc::now()
            - chrono::Duration::from_std(self.policy.claim_lease)
                .unwrap_or(chrono::Duration::zero());
        let result = sqlx::query(
            r#"UPDATE adapter_delivery SET
                   status     = 'pending',
                   claimed_at = NULL,
                   claimed_by = NULL,
                   updated_at = NOW()
               WHERE status = 'claimed' AND claimed_at < $1"#,
        )
        .bind(deadline)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    /// Delete finished records past their retention window.
    ///
    /// Bounded per call so a large backlog is drained over several maintenance
    /// ticks instead of one long-running delete.
    pub(crate) async fn prune_finished(&self) -> Result<u64, sqlx::Error> {
        let succeeded_before = Utc::now()
            - chrono::Duration::from_std(self.policy.succeeded_retention)
                .unwrap_or(chrono::Duration::zero());
        let failed_before = Utc::now()
            - chrono::Duration::from_std(self.policy.failed_retention)
                .unwrap_or(chrono::Duration::zero());
        let result = sqlx::query(
            r#"DELETE FROM adapter_delivery WHERE id IN (
                   SELECT id FROM adapter_delivery
                   WHERE (status = 'succeeded' AND updated_at < $1)
                      OR (status = 'failed'    AND updated_at < $2)
                   LIMIT 1000
               )"#,
        )
        .bind(succeeded_before)
        .bind(failed_before)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    /// Read the counts behind the backlog gauges.
    pub(crate) async fn backlog(&self) -> Result<Backlog, sqlx::Error> {
        let row = sqlx::query(
            r#"SELECT
                   COUNT(*) FILTER (WHERE status = 'pending')   AS pending,
                   COUNT(*) FILTER (WHERE status = 'claimed')   AS claimed,
                   COUNT(*) FILTER (WHERE status = 'failed')    AS failed,
                   MIN(admitted_at) FILTER (WHERE status = 'pending') AS oldest_pending
               FROM adapter_delivery"#,
        )
        .fetch_one(&self.pool)
        .await?;

        let oldest_pending: Option<DateTime<Utc>> = row.try_get("oldest_pending")?;
        Ok(Backlog {
            pending: row.try_get::<Option<i64>, _>("pending")?.unwrap_or(0),
            claimed: row.try_get::<Option<i64>, _>("claimed")?.unwrap_or(0),
            failed: row.try_get::<Option<i64>, _>("failed")?.unwrap_or(0),
            oldest_pending_secs: oldest_pending
                .map_or(0, |admitted| (Utc::now() - admitted).num_seconds().max(0)),
        })
    }

    /// Read one record's status and last error. Only the tests below and
    /// operator queries need this; the worker drives records forward blind.
    #[cfg(all(test, feature = "db-tests"))]
    pub(crate) async fn peek(
        &self,
        adapter: &str,
        kind: &str,
        event_id: &str,
    ) -> Result<Option<(String, i32, Option<String>)>, sqlx::Error> {
        let row = sqlx::query(
            r#"SELECT status, attempts, last_error FROM adapter_delivery
               WHERE adapter = $1 AND kind = $2 AND event_id = $3"#,
        )
        .bind(adapter)
        .bind(kind)
        .bind(event_id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| {
            Ok((
                row.try_get("status")?,
                row.try_get("attempts")?,
                row.try_get("last_error")?,
            ))
        })
        .transpose()
    }
}

#[async_trait::async_trait]
impl DeliveryAdmission for AdapterDeliveryStore {
    /// Take durable ownership of one inbound Adapter delivery.
    ///
    /// Returns once the record is committed, which is the point the caller may
    /// acknowledge its sender. A repeat delivery of the same
    /// `(sender, kind, event_id)` returns [`Admission::Duplicate`] without
    /// writing anything, so acknowledging a duplicate costs one insert attempt
    /// and produces no second effect.
    ///
    /// Refuses with [`AdmitError::AtCapacity`] once the unfinished backlog
    /// reaches `AdapterDeliveryPolicy::backlog_capacity`. The capacity test rides
    /// along inside the insert, so admission stays a single round trip on the
    /// sender's response deadline. See that field for the exact bound this
    /// enforces — it is a ceiling, not an exact cutoff.
    ///
    /// `DeliveryAdmission` is the store's public admission surface; the
    /// worker-only operations stay inherent on the concrete
    /// [`AdapterDeliveryStore`].
    async fn admit(&self, request: AdapterDeliveryRequest) -> Result<Admission, AdmitError> {
        let (adapter, kind) = (request.adapter, request.kind);
        let result = self.insert(request).await;
        let outcome = match &result {
            Ok(admission) => admission.as_str(),
            Err(AdmitError::AtCapacity { .. }) => "at_capacity",
            Err(AdmitError::Store(_)) => "error",
        };
        record_admission(adapter, kind, outcome);
        result
    }
}

/// Wrap a failed statement on the admission path.
///
/// Named once so both statements — the insert and the duplicate check behind it
/// — report the same operation, and applied here rather than at the transport
/// boundary so `AdmitError` never carries a `sqlx::Error` of its own. The driver
/// error survives as the [`StoreError`]'s source.
fn admission_failed(source: sqlx::Error) -> AdmitError {
    AdmitError::Store(StoreError::with_source(
        "persist the Adapter delivery record",
        source,
    ))
}

/// Keep a stored failure readable without letting a pathological error message
/// bloat the row.
fn truncate_error(error: &str) -> String {
    const MAX: usize = 2000;
    if error.len() <= MAX {
        return error.to_string();
    }
    let mut end = MAX;
    while end > 0 && !error.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &error[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_truncation_keeps_short_messages_and_splits_on_char_boundaries() {
        assert_eq!(truncate_error("boom"), "boom");
        let long = "é".repeat(3000);
        let truncated = truncate_error(&long);
        assert!(truncated.ends_with('…'));
        assert!(truncated.len() <= 2001 + '…'.len_utf8());
    }
}
