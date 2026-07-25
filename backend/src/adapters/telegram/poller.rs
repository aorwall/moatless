//! Background Telegram long-polling — calls `getUpdates` in a loop when
//! the server cannot expose a webhook endpoint.

use std::sync::Arc;
use std::time::Duration;

use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::adapters::Adapters;
use crate::state::AppState;

use super::adapter::resolve_and_process_telegram_update;

const POLL_TIMEOUT_SECS: u32 = 30;
const INITIAL_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// Spawn the Telegram long-polling background task.
///
/// Follows the same `tokio::spawn` + `CancellationToken` pattern as the cron
/// scheduler in `cron/scheduler.rs`.
pub fn spawn_telegram_poller(state: AppState, providers: Arc<Adapters>, cancel: CancellationToken) {
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
                    backoff = INITIAL_BACKOFF;

                    for update in &updates {
                        // Advance offset so Telegram discards this update
                        if let Some(uid) =
                            update.get("update_id").and_then(serde_json::Value::as_i64)
                        {
                            offset = Some(uid + 1);
                        }

                        let body = match serde_json::to_vec(update) {
                            Ok(b) => b,
                            Err(e) => {
                                warn!(error = %e, "Telegram poller: failed to serialize update");
                                continue;
                            }
                        };

                        if let Err(e) =
                            resolve_and_process_telegram_update(&state, telegram_provider, &body)
                                .await
                        {
                            warn!(error = %e, "Telegram poller: failed to process update");
                        }
                    }
                }
                Err(e) => {
                    let is_conflict = e.to_string().contains("409 Conflict");
                    if is_conflict {
                        error!("Telegram poller: another bot instance is already polling — cannot run two polling instances simultaneously");
                    } else {
                        warn!(
                            error = %e,
                            backoff_secs = backoff.as_secs(),
                            "Telegram getUpdates failed, backing off"
                        );
                    }

                    tokio::select! {
                        () = cancel.cancelled() => {
                            info!("Telegram poller shutting down");
                            return;
                        }
                        () = tokio::time::sleep(backoff) => {}
                    }

                    backoff = (backoff * 2).min(MAX_BACKOFF);
                }
            }
        }
    });
}
