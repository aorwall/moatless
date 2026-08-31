//! Record, request, and outcome types for durable Adapter delivery.
//!
//! The tuning that governs *how* the seam retries and prunes —
//! `AdapterDeliveryPolicy` — belongs to the store and the worker that read it,
//! and stays with them in `moatless-loops`.

use moatless_types::store::StoreError;

/// One inbound delivery an Adapter hands over before acknowledging its sender.
///
/// `event_id` must be the identifier the *sender* assigned to the delivery.
/// A backend-generated id would make every redelivery a new record, which is
/// the duplicate-effect bug this seam exists to prevent.
///
/// One caller is deliberately outside that rule and says so on its own
/// interface: the generic webhook adapter's sender is an arbitrary client with
/// no retry contract, so it *may* supply a delivery identifier and gets
/// deduplication when it does, and gets a freshly minted one — and therefore
/// durable ownership without deduplication — when it does not.
#[derive(Debug, Clone)]
pub struct AdapterDeliveryRequest {
    /// The Adapter this delivery arrived on, used for metrics and operator
    /// queries. Never "provider": the glossary keeps that word as a qualifier
    /// only.
    pub adapter: &'static str,
    /// Names the registered handler that will execute this delivery. One
    /// sender delivery becomes one record per kind it fans into.
    pub kind: &'static str,
    /// The sender's delivery identifier.
    pub event_id: String,
    /// Everything the handler needs to redo this delivery's work from scratch.
    /// Must be self-contained: nothing held in memory survives a process kill.
    pub payload: serde_json::Value,
}

/// What admission did with a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission {
    /// A new durable record was written. The delivery is now owned by the
    /// backend.
    Admitted,
    /// This sender delivery was already admitted for this kind. Nothing was
    /// written and nothing extra will run.
    Duplicate,
}

impl Admission {
    /// Metric/log label.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Admitted => "admitted",
            Self::Duplicate => "duplicate",
        }
    }
}

/// Admission failed, so the sender must not be told the delivery was accepted.
#[derive(Debug, thiserror::Error)]
pub enum AdmitError {
    /// The unfinished backlog has reached its configured ceiling. Refusing here
    /// is what keeps ingress bounded: the sender retries the delivery later
    /// instead of the backend accepting one it cannot promise to execute.
    ///
    /// `limit` is the configured capacity, which is what an operator recognises
    /// — not the observed count, which may sit a few records above it while
    /// concurrent admissions commit. `AdapterDeliveryPolicy::backlog_capacity`
    /// in `moatless-loops` states the arithmetic.
    #[error("Adapter delivery backlog is at capacity ({limit} unfinished records)")]
    AtCapacity { limit: i64 },
    /// The durable write itself failed. [`StoreError`] names the operation and
    /// carries the driver error as its source, so this variant — which four
    /// provider crates match on and two construct in their own tests — names no
    /// database driver. The cause is retained as the source and redacted at the
    /// transport boundary.
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// One admitted record, as handed to its handler.
///
/// The row shape is contract, so every field is public whatever the feature
/// set. Only its *decoder* is a driver concern: `sqlx::FromRow` is derived
/// under `postgres`, so a provider crate that reads this type without a
/// database driver still sees the same fields.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "postgres", derive(sqlx::FromRow))]
pub struct AdapterDelivery {
    pub id: String,
    pub adapter: String,
    pub kind: String,
    pub event_id: String,
    pub payload: serde_json::Value,
    /// Attempt number this execution is, starting at 1.
    pub attempts: i32,
    /// Attempt limit for this record, fixed at admission.
    pub max_attempts: i32,
}

impl AdapterDelivery {
    /// Whether a failure of this attempt exhausts the record's attempt budget.
    pub fn is_last_attempt(&self) -> bool {
        self.attempts >= self.max_attempts
    }
}

/// Why a handler could not complete an Adapter delivery.
///
/// The distinction is the whole of the retry policy at the handler's end:
/// `Retryable` spends one attempt, `Permanent` ends the record immediately.
/// Both keep the causing error as their source.
#[derive(Debug, thiserror::Error)]
pub enum AdapterDeliveryFailure {
    /// A transient failure — a timeout, a sender 5xx, a lost connection.
    /// Retried with backoff until the attempt budget runs out.
    #[error("retryable Adapter delivery failure")]
    Retryable(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// The delivery can never succeed: a malformed payload, a deleted task, a
    /// revoked connection. Goes terminal on this attempt, no matter how much
    /// budget is left.
    #[error("permanent Adapter delivery failure")]
    Permanent(#[source] Box<dyn std::error::Error + Send + Sync>),
}

impl AdapterDeliveryFailure {
    /// Wrap a transient failure.
    pub fn retryable(source: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::Retryable(Box::new(source))
    }

    /// Wrap a failure that no number of retries can fix.
    pub fn permanent(source: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::Permanent(Box::new(source))
    }

    pub fn is_permanent(&self) -> bool {
        matches!(self, Self::Permanent(_))
    }
}
