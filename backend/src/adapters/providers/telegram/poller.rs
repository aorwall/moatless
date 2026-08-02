//! Background Telegram long-polling — calls `getUpdates` in a loop when
//! the server cannot expose a webhook endpoint.

use std::sync::Arc;
use std::time::Duration;

use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::adapters::ingress::IngressError;
use crate::adapters::Adapters;
use crate::state::AppState;

use super::ingress::{self, Transport};

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
/// A refused admission or a database blip could; an update missing a field no
/// re-delivery supplies never will, and holding the offset back for one would
/// wedge the poller on it forever rather than delivering the updates queued
/// behind it.
///
/// This is the whole rule, and it is a match on [`IngressError`] rather than a
/// guess about which errors that path happens to raise — which is why the
/// variants were made to carry the distinction (spec Decision 6). Which class
/// a rejection lands in is decided where it is raised: see
/// `ingress::classify_normalization`.
const fn is_worth_redelivering(error: &IngressError) -> bool {
    match error {
        IngressError::NotAdmitted(_) | IngressError::Ingress(_) => true,
        IngressError::Unidentifiable(_) => false,
    }
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

        let api = match telegram_provider
            .app_api(crate::adapter_apps::DEFAULT_APP_KEY)
            .await
        {
            Ok(api) => api,
            Err(e) => {
                error!(error = %e, "Telegram poller: default app is not configured");
                return;
            }
        };

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
            let api = match telegram_provider
                .app_api(crate::adapter_apps::DEFAULT_APP_KEY)
                .await
            {
                Ok(api) => api,
                Err(e) => {
                    warn!(
                        error = %e,
                        backoff_secs = backoff.as_secs(),
                        "Telegram poller: default app lookup failed, backing off"
                    );
                    if !back_off(&cancel, &mut backoff).await {
                        info!("Telegram poller shutting down");
                        return;
                    }
                    continue;
                }
            };
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

                        // Long polling carries no secret, so there is nothing
                        // for this transport to verify.
                        match ingress::receive(&state, telegram_provider, &body, Transport::Poll)
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

    /// The offset *is* the acknowledgment, so it may only ever move to one past
    /// an update the poller is done with — and an update Telegram sent without
    /// an id cannot be acknowledged at all.
    #[test]
    fn the_offset_acknowledges_exactly_the_update_it_moves_past() {
        let mut offset = None;
        advance(&mut offset, Some(41));
        assert_eq!(offset, Some(42));

        advance(&mut offset, Some(50));
        assert_eq!(offset, Some(51));

        advance(&mut offset, None);
        assert_eq!(offset, Some(51), "an idless update acknowledges nothing");
    }

    /// The offset is held back only for failures a re-delivery could survive.
    /// An update no re-delivery can fix must be dropped instead, or the poller
    /// wedges on it and everything queued behind it stops.
    #[test]
    fn only_a_fixable_failure_holds_the_offset_back() {
        use crate::adapters::delivery::AdmitError;
        use crate::adapters::error::AdapterError;

        assert!(is_worth_redelivering(&IngressError::NotAdmitted(
            AdmitError::AtCapacity { limit: 100 }
        )));
        assert!(is_worth_redelivering(&IngressError::Ingress(
            AdapterError::Db(sqlx::Error::PoolTimedOut)
        )));
        assert!(is_worth_redelivering(&IngressError::Ingress(
            AdapterError::Internal("sandbox refused".into())
        )));
        assert!(!is_worth_redelivering(&IngressError::Unidentifiable(
            "Missing update_id".into()
        )));
    }
}
