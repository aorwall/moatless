//! Background Telegram long-polling — calls `getUpdates` in a loop when
//! the server cannot expose a webhook endpoint.

use std::sync::Arc;
use std::time::Duration;

use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::adapters::error::AdapterError;
use crate::adapters::Adapters;
use crate::state::AppState;

use super::adapter::resolve_and_process_telegram_update;

const POLL_TIMEOUT_SECS: u32 = 30;
const INITIAL_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// Confirm an update to Telegram by asking for everything after it.
///
/// `getUpdates` treats the offset as the acknowledgment: once it moves past an
/// update, Telegram discards it and will never send it again.
fn advance(offset: &mut Option<i64>, update_id: Option<i64>) {
    if let Some(update_id) = update_id {
        *offset = Some(update_id + 1);
    }
}

/// Whether asking Telegram for this update again could produce a different
/// outcome.
///
/// A refused admission or a database blip could; a malformed update never will,
/// and holding the offset back for one would wedge the poller on it forever
/// rather than delivering the updates queued behind it.
///
/// The narrow `false` arm is deliberate and depends on a fact about the path
/// behind it: everything `resolve_and_process_telegram_update` reaches raises
/// [`AdapterError::BadRequest`] for exactly the unfixable cases — a malformed
/// envelope — and answers an unmapped sender or a denied access decision with
/// `Ignore` rather than an error. This is narrower than the four variants
/// `inbound_delivery::classify` calls permanent, and that is correct *here* and not
/// in general. If a fifth error kind is ever raised on this path, check it
/// against this arm: an unfixable failure landing in the `true` branch wedges
/// the poller on one update and blocks every update queued behind it.
fn is_worth_redelivering(error: &AdapterError) -> bool {
    !matches!(error, AdapterError::BadRequest(_))
}

/// Wait out the current backoff, then double it up to [`MAX_BACKOFF`].
///
/// Returns `false` when shutdown was requested while waiting, which the caller
/// must treat as "stop", not "retry now".
///
/// Every failure that leaves work to re-attempt goes through here — a
/// `getUpdates` transport failure and a held-back offset alike. Keeping one
/// wait rather than one per failure kind is the point: a backoff that some
/// failure path skips is a backoff that does not exist on the path that needed
/// it most.
async fn back_off(cancel: &CancellationToken, backoff: &mut Duration) -> bool {
    tokio::select! {
        () = cancel.cancelled() => false,
        () = tokio::time::sleep(*backoff) => {
            *backoff = (*backoff * 2).min(MAX_BACKOFF);
            true
        }
    }
}

/// Spawn the Telegram long-polling background task.
///
/// Follows the same `tokio::spawn` + `CancellationToken` pattern as the cron
/// scheduler in `cron/scheduler.rs`. The join handle is returned for the root
/// supervisor to own; the poller is registered as an optional worker, so its
/// early exit (an unregistered provider, a rival polling instance) degrades
/// Telegram alone and is logged rather than failing the process.
pub fn spawn_telegram_poller(
    state: AppState,
    providers: Arc<Adapters>,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        info!("Telegram poller started");

        let Some(telegram_provider) = providers.telegram() else {
            error!("Telegram poller: provider not registered");
            return;
        };

        let api = telegram_provider.api();

        // Delete any existing webhook so getUpdates works
        match api.delete_webhook().await {
            Ok(()) => info!("Telegram poller: cleared existing webhook"),
            Err(e) => {
                warn!(error = %e, "Telegram poller: failed to delete webhook, continuing anyway");
            }
        }

        let mut offset: Option<i64> = None;
        let mut backoff = INITIAL_BACKOFF;

        loop {
            let result = tokio::select! {
                () = cancel.cancelled() => {
                    info!("Telegram poller shutting down");
                    return;
                }
                result = api.get_updates(offset, POLL_TIMEOUT_SECS) => result,
            };

            match result {
                Ok(updates) => {
                    let mut held_back = false;

                    for update in &updates {
                        let update_id = update.get("update_id").and_then(serde_json::Value::as_i64);

                        let body = match serde_json::to_vec(update) {
                            Ok(b) => b,
                            Err(e) => {
                                warn!(error = %e, "Telegram poller: failed to serialize update");
                                advance(&mut offset, update_id);
                                continue;
                            }
                        };

                        match resolve_and_process_telegram_update(&state, telegram_provider, &body)
                            .await
                        {
                            Ok(_) => {}
                            Err(e) if is_worth_redelivering(&e) => {
                                warn!(
                                    error = %e,
                                    update_id,
                                    backoff_secs = backoff.as_secs(),
                                    "Telegram poller: update not accepted, leaving the offset so it is delivered again"
                                );
                                held_back = true;
                                break;
                            }
                            Err(e) => warn!(
                                error = %e,
                                update_id,
                                "Telegram poller: dropping an update no re-delivery can fix"
                            ),
                        }

                        // Only now: the offset is this transport's
                        // acknowledgment, and advancing it past an update whose
                        // routing was never accepted is how the polling path
                        // used to lose one.
                        advance(&mut offset, update_id);
                    }

                    if held_back {
                        // A held-back update is still pending, so the next
                        // `getUpdates` returns it immediately instead of
                        // long-polling for 30s. Re-attempting it at that rate
                        // would hammer whatever refused it — and a saturated
                        // backlog, one of the conditions that holds an update
                        // back, is precisely the persistent state such a spin
                        // would feed. So this failure waits like any other.
                        if !back_off(&cancel, &mut backoff).await {
                            info!("Telegram poller shutting down");
                            return;
                        }
                    } else {
                        // Reset only on a batch that got all the way through:
                        // every spin iteration is an `Ok`, so resetting on
                        // `Ok` alone is what would defeat the backoff above.
                        backoff = INITIAL_BACKOFF;
                    }
                }
                Err(e) => {
                    let is_conflict = e.to_string().contains("409 Conflict");
                    if is_conflict {
                        error!(
                            "Telegram poller: another bot instance is already polling — cannot run two polling instances simultaneously"
                        );
                    } else {
                        warn!(
                            error = %e,
                            backoff_secs = backoff.as_secs(),
                            "Telegram getUpdates failed, backing off"
                        );
                    }

                    if !back_off(&cancel, &mut backoff).await {
                        info!("Telegram poller shutting down");
                        return;
                    }
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The backoff has to actually grow, or the "wait like any other failure"
    /// above degrades into a fixed-rate retry against something that is still
    /// failing.
    #[tokio::test(start_paused = true)]
    async fn backing_off_doubles_up_to_the_ceiling() {
        let cancel = CancellationToken::new();
        let mut backoff = INITIAL_BACKOFF;

        assert!(back_off(&cancel, &mut backoff).await);
        assert_eq!(backoff, INITIAL_BACKOFF * 2);

        for _ in 0..10 {
            assert!(back_off(&cancel, &mut backoff).await);
        }
        assert_eq!(backoff, MAX_BACKOFF, "growth stops at the ceiling");
    }

    /// Shutdown during the wait means stop, not retry immediately — both call
    /// sites return on `false`.
    #[tokio::test(start_paused = true)]
    async fn a_cancelled_wait_says_stop_rather_than_retry_now() {
        let cancel = CancellationToken::new();
        cancel.cancel();

        let mut backoff = INITIAL_BACKOFF;
        assert!(!back_off(&cancel, &mut backoff).await);
        assert_eq!(
            backoff, INITIAL_BACKOFF,
            "a wait that never completed did not spend a backoff step"
        );
    }

    /// The offset is held back only for failures a re-delivery could survive.
    /// A malformed update must be dropped instead, or the poller wedges on it
    /// and everything queued behind it stops.
    #[test]
    fn only_a_fixable_failure_holds_the_offset_back() {
        assert!(is_worth_redelivering(&AdapterError::Overloaded));
        assert!(is_worth_redelivering(&AdapterError::Db(
            sqlx::Error::PoolTimedOut
        )));
        assert!(is_worth_redelivering(&AdapterError::Internal(
            "sandbox refused".into()
        )));
        assert!(!is_worth_redelivering(&AdapterError::BadRequest(
            "Missing update_id".into()
        )));
    }
}
