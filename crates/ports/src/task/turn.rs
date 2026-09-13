use async_trait::async_trait;
use chrono::DateTime;
use chrono::Utc;
use moatless_types::store::StoreError;
use moatless_types::task::TurnStatus;

/// The Turn a row appended now belongs to.
#[cfg_attr(feature = "test-support", mockall::automock)]
#[async_trait]
pub trait TurnCounter: Send + Sync {
    async fn latest_turn_number(&self, task_id: &str) -> Result<Option<i32>, StoreError>;
}

/// The Task band's half of [`crate::running::derive_running`], for one Task.
///
/// A peer already holds the Sandbox half (intent, stop reason, heartbeat);
/// `None` here means *this reader cannot say*, never *this did not happen*.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TurnSignals {
    pub turn_status: Option<TurnStatus>,
    pub turn_ended_at: Option<DateTime<Utc>>,
    pub latest_user_message_at: Option<DateTime<Utc>>,
}

/// The durable turn state behind the running rule.
///
/// Narrower than [`TurnCounter`]: that answers "which turn does a new row
/// belong to", this answers "is a turn in flight" — kept apart since this is
/// read on a status path that must not grow a write-path dependency.
#[cfg_attr(feature = "test-support", mockall::automock)]
#[async_trait]
pub trait TurnStateReader: Send + Sync {
    async fn turn_signals(&self, task_id: &str) -> Result<TurnSignals, StoreError>;
}
