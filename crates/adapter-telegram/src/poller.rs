//! Background Telegram long-polling — calls `getUpdates` in a loop when the
//! server cannot expose a webhook endpoint.

use std::sync::Arc;

use moatless_ports::identity::ProviderIdentityLink;
use std::time::Duration;

use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use moatless_adapter_model::ingress::IngressError;

use super::adapter::TelegramAdapter;
use super::ingress::{self, Transport};

const POLL_TIMEOUT_SECS: u32 = 30;
const INITIAL_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// Confirm an update to Telegram by asking for everything after it.
///
/// `getUpdates` treats the offset as the acknowledgment: once it moves past an
/// update, Telegram discards it and never sends it again.
fn advance(offset: &mut Option<i64>, update_id: Option<i64>) {
    if let Some(update_id) = update_id {
        *offset = Some(update_id + 1);
    }
}

/// Whether asking Telegram for this update again could produce a different
/// outcome.
///
/// A refused admission or database blip could; a missing-field error never
/// will, and treating it as retryable wedges the poller forever.
const fn is_worth_redelivering(error: &IngressError) -> bool {
    match error {
        IngressError::NotAdmitted(_) | IngressError::Ingress(_) => true,
        IngressError::Unidentifiable(_) => false,
    }
}

/// Wait out the current backoff, then double it up to [`MAX_BACKOFF`].
///
/// Returns `false` when shutdown was requested while waiting; the caller must
/// treat that as stop, not retry-now.
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
/// Registered as an optional worker: an early exit (unregistered provider,
/// rival poller) degrades Telegram alone and is logged, not failed as a process
/// error.
pub fn spawn_telegram_poller(
    telegram_provider: Arc<TelegramAdapter>,
    users: Arc<dyn ProviderIdentityLink>,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        info!("Telegram poller started");

        let api = match telegram_provider
            .app_api(moatless_adapter_model::apps::DEFAULT_APP_KEY)
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
                .app_api(moatless_adapter_model::apps::DEFAULT_APP_KEY)
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
                        match ingress::receive(
                            &telegram_provider,
                            users.as_ref(),
                            &body,
                            Transport::Poll,
                        )
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
                        // routing was never accepted would lose it.
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
        use moatless_adapter_model::delivery::AdmitError;
        use moatless_adapter_model::error::SharedAdapterError;

        assert!(is_worth_redelivering(&IngressError::NotAdmitted(
            AdmitError::AtCapacity { limit: 100 }
        )));
        assert!(is_worth_redelivering(&IngressError::Ingress(
            SharedAdapterError::Store(moatless_types::store::StoreError::with_source(
                "read the connection",
                sqlx::Error::PoolTimedOut,
            ))
            .into()
        )));
        assert!(is_worth_redelivering(&IngressError::Ingress(
            SharedAdapterError::Internal("sandbox refused".into()).into()
        )));
        assert!(!is_worth_redelivering(&IngressError::Unidentifiable(
            "Missing update_id".into()
        )));
    }
}
