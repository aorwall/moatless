//! Ingress deduplication for sandbox-published events.
//!
//! The sandbox event outbox delivers at-least-once: on a failure — including a
//! response lost after the backend already admitted the event — it retries the
//! same event under the same `event_id` and `sequence`. This module makes that
//! retry converge instead of duplicating.
//!
//! Deduplication records **admission, not receipt** (the durable-ingress decision). A delivery
//! claims its event id, and the claim only becomes a lasting "already admitted"
//! once the complete payload is durably admitted; a failed admission releases
//! the claim so the retry gets a real second attempt. A duplicate that arrives
//! while the first delivery is still in flight is answered as retryable rather
//! than acknowledged, because the winner's admission may yet fail.
//!
//! # This cache is a suppressor, not the guarantee
//!
//! An entry expires after [`EVENT_ID_TTL`] and does not survive a backend
//! restart, while the sandbox retries indefinitely. A retry that outlives its
//! entry — a link down longer than the TTL, or a restarted backend — therefore
//! re-runs admission and fans the event out a second time.
//!
//! That is deliberate, because suppressing it here would buy nothing: Redis
//! Streams redeliver from the consumer's pending list after a disconnect, so
//! every consumer must already be idempotent under duplicate dispatch no matter
//! what this cache does. Convergence is owned durably downstream instead —
//! `ON CONFLICT (event_id) DO NOTHING` on `task_events`, `ON CONFLICT (uuid) DO
//! UPDATE` plus in-place block-index merge on `task_messages`, and the monotonic
//! `WHERE` guards on `task_turns`. This cache only keeps the common, cheap case
//! off the live stream twice.
//!
//! What *is* durable is admission itself: the complete payload is committed to
//! Postgres before the ack, so a re-run re-admits the same bytes and the event
//! cannot be lost.

use std::time::Duration;

use moka::future::Cache;

/// How long an admitted `event_id` is remembered. Comfortably longer than the
/// sandbox's retry backoff (capped at 30s) so the ordinary retry is recognised,
/// yet short enough to bound memory. Retries that outlive it re-admit — see the
/// module docs for why that converges rather than duplicating.
const EVENT_ID_TTL: Duration = Duration::from_mins(5);

/// Max distinct event ids retained across all tasks. LRU eviction past this.
const MAX_EVENT_IDS: u64 = 50_000;

/// What a delivery's `event_id` is currently known to be.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AdmissionState {
    /// A delivery holds the claim and has not finished admitting.
    InFlight { sequence: u64 },
    /// The complete payload was durably admitted.
    Admitted { sequence: u64 },
}

/// The caller's standing with respect to one `event_id`.
#[derive(Debug)]
pub enum Admission {
    /// This delivery owns admission. Report the outcome through the claim.
    Claimed(AdmissionClaim),
    /// The event was already admitted by an earlier delivery — acknowledge it
    /// without re-publishing.
    AlreadyAdmitted,
    /// A concurrent delivery owns admission and has not finished. Answer
    /// retryable; the retry converges on the winner's outcome.
    InFlight,
}

/// Ownership of one event id's admission, held for the duration of the attempt.
///
/// Exactly one of [`AdmissionClaim::admitted`] / [`AdmissionClaim::failed`] must
/// be called. Neither is a `Drop` impl because releasing the claim is async; a
/// leaked claim degrades to "retryable until the TTL expires", never to a lost
/// or duplicated event.
#[derive(Debug)]
pub struct AdmissionClaim {
    seen: Cache<String, AdmissionState>,
    key: String,
    sequence: u64,
}

impl AdmissionClaim {
    /// The complete payload is durably admitted: later deliveries of this id
    /// are duplicates.
    pub async fn admitted(self) {
        self.seen
            .insert(
                self.key,
                AdmissionState::Admitted {
                    sequence: self.sequence,
                },
            )
            .await;
    }

    /// Admission failed: release the claim so the sandbox's retry is a real
    /// attempt rather than a duplicate.
    pub async fn failed(self) {
        self.seen.invalidate(&self.key).await;
    }
}

/// Remembers which `event_id`s have been admitted, and which are being admitted.
pub struct EventDedup {
    seen: Cache<String, AdmissionState>,
    /// Last sequence observed per task, used only to log gaps/resets for
    /// observability — it never gates publishing.
    last_sequence: Cache<String, u64>,
}

impl EventDedup {
    pub fn new() -> Self {
        Self {
            seen: Cache::builder()
                .max_capacity(MAX_EVENT_IDS)
                .time_to_live(EVENT_ID_TTL)
                .build(),
            last_sequence: Cache::builder()
                .max_capacity(MAX_EVENT_IDS)
                .time_to_live(EVENT_ID_TTL)
                .build(),
        }
    }

    /// Claim admission for `(task_id, event_id)`.
    ///
    /// The claim is taken atomically per key, so two concurrent deliveries of
    /// one event id cannot both be told to admit it: exactly one gets
    /// [`Admission::Claimed`] and the other [`Admission::InFlight`].
    ///
    /// `sequence` is recorded with the claim purely to observe the sandbox's
    /// half of the contract — a retry must reuse the original id *and*
    /// sequence, and a mismatch is logged as the contract violation it is.
    pub async fn claim(&self, task_id: &str, event_id: &str, sequence: u64) -> Admission {
        let key = format!("{task_id}:{event_id}");
        let entry = self
            .seen
            .entry(key.clone())
            .or_insert_with(std::future::ready(AdmissionState::InFlight { sequence }))
            .await;

        if entry.is_fresh() {
            return Admission::Claimed(AdmissionClaim {
                seen: self.seen.clone(),
                key,
                sequence,
            });
        }

        let (state, prior_sequence) = match entry.into_value() {
            AdmissionState::Admitted { sequence } => ("admitted", sequence),
            AdmissionState::InFlight { sequence } => ("in_flight", sequence),
        };
        if prior_sequence != sequence {
            tracing::warn!(
                task_id,
                event_id,
                prior_sequence,
                sequence,
                "retried event reused its id under a different sequence"
            );
        }
        tracing::debug!(task_id, event_id, state, "duplicate delivery at ingress");
        if state == "admitted" {
            Admission::AlreadyAdmitted
        } else {
            Admission::InFlight
        }
    }

    /// Log a warning when a task's sequence skips forward (possible lost event)
    /// or moves backward (sandbox restart reset the counter). Best-effort
    /// observability only; `sequence == 0` means the sender didn't set it.
    pub async fn observe_sequence(&self, task_id: &str, sequence: u64) {
        if sequence == 0 {
            return;
        }
        if let Some(prev) = self.last_sequence.get(task_id).await {
            if sequence > prev + 1 {
                tracing::warn!(
                    task_id = %task_id,
                    prev_sequence = prev,
                    sequence,
                    "sandbox event sequence gap — a delivery may have been lost"
                );
            } else if sequence <= prev {
                tracing::debug!(
                    task_id = %task_id,
                    prev_sequence = prev,
                    sequence,
                    "sandbox event sequence did not advance (retry or restart)"
                );
            }
        }
        self.last_sequence
            .insert(task_id.to_string(), sequence)
            .await;
    }
}

impl Default for EventDedup {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Assert the claim's discriminant without requiring `PartialEq` on the
    /// claim (which owns a cache handle).
    fn kind(admission: &Admission) -> &'static str {
        match admission {
            Admission::Claimed(_) => "claimed",
            Admission::AlreadyAdmitted => "already",
            Admission::InFlight => "in_flight",
        }
    }

    #[tokio::test]
    async fn duplicate_after_admission_is_recognised() {
        let dedup = EventDedup::new();
        let Admission::Claimed(claim) = dedup.claim("task-1", "evt-1", 1).await else {
            panic!("first delivery must own admission");
        };
        claim.admitted().await;

        assert_eq!(kind(&dedup.claim("task-1", "evt-1", 1).await), "already");
        // Different id, and the same id under a different task, are both new.
        assert_eq!(kind(&dedup.claim("task-1", "evt-2", 2).await), "claimed");
        assert_eq!(kind(&dedup.claim("task-2", "evt-1", 1).await), "claimed");
    }

    #[tokio::test]
    async fn failed_admission_releases_the_claim() {
        let dedup = EventDedup::new();
        let Admission::Claimed(claim) = dedup.claim("task-1", "evt-1", 1).await else {
            panic!("first delivery must own admission");
        };
        claim.failed().await;

        // The retry gets a real second attempt, not a false "already admitted".
        assert_eq!(kind(&dedup.claim("task-1", "evt-1", 1).await), "claimed");
    }

    #[tokio::test]
    async fn concurrent_duplicate_is_told_to_retry() {
        let dedup = EventDedup::new();
        let Admission::Claimed(_claim) = dedup.claim("task-1", "evt-1", 1).await else {
            panic!("first delivery must own admission");
        };
        // The first delivery is still admitting: acknowledging now would ack an
        // admission that may still fail.
        assert_eq!(kind(&dedup.claim("task-1", "evt-1", 1).await), "in_flight");
    }

    #[tokio::test]
    async fn concurrent_claims_elect_exactly_one_admitter() {
        let dedup = std::sync::Arc::new(EventDedup::new());
        let mut handles = Vec::new();
        for _ in 0..16 {
            let dedup = dedup.clone();
            handles.push(tokio::spawn(async move {
                matches!(
                    dedup.claim("task-1", "evt-1", 7).await,
                    Admission::Claimed(_)
                )
            }));
        }
        let mut claimed = 0;
        for handle in handles {
            if handle.await.expect("task joined") {
                claimed += 1;
            }
        }
        assert_eq!(claimed, 1, "exactly one concurrent delivery may admit");
    }
}
