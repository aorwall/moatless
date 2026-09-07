//! Record, request, and outcome types for durable Adapter delivery.
//!
//! The tuning that governs *how* the seam retries and prunes —
//! `AdapterDeliveryPolicy` — belongs to the store and the worker that read it,
//! and stays with them in `moatless-loops`.

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
