//! Background cron scheduler — polls active cron configs and triggers tasks.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use cron::Schedule;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::leader::LeaderGuard;
use crate::state::AppState;

/// Spawn the cron scheduler background task.
///
/// Checks active cron configs every 60 seconds and fires any overdue schedules.
pub fn spawn_cron_scheduler(
    state: AppState,
    leader: Option<Arc<dyn LeaderGuard>>,
    cancel: CancellationToken,
) {
    tokio::spawn(async move {
        info!("Cron scheduler started");

        // Track last fire time per config_id to avoid double-firing
        let mut last_fired: HashMap<String, chrono::DateTime<Utc>> = HashMap::new();

        loop {
            tokio::select! {
                () = cancel.cancelled() => {
                    info!("Cron scheduler shutting down");
                    return;
                }
                () = tokio::time::sleep(std::time::Duration::from_secs(60)) => {
                    // When a leader guard is present, only tick if we are the leader.
                    if let Some(ref guard) = leader {
                        match guard.try_acquire_or_renew().await {
                            Ok(true) => { /* we are leader, proceed */ }
                            Ok(false) => {
                                debug!("Cron scheduler not leader, skipping tick");
                                continue;
                            }
                            Err(e) => {
                                warn!(error = %e, "Cron leader election error, skipping tick");
                                continue;
                            }
                        }
                    }

                    if let Err(e) = tick(&state, &mut last_fired).await {
                        error!(error = %e, "Cron scheduler tick failed");
                    }
                }
            }
        }
    });
}

async fn tick(
    state: &AppState,
    last_fired: &mut HashMap<String, chrono::DateTime<Utc>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Load active cron configs
    let configs = state
        .channel_configs
        .list(Some("cron"), None, true)
        .await?;

    if configs.is_empty() {
        return Ok(());
    }

    let now = Utc::now();
    debug!(config_count = configs.len(), now = %now, "Cron scheduler tick");

    for config in &configs {
        let schedule_str = config
            .provider_config
            .as_ref()
            .and_then(|c| c.get("schedule"))
            .and_then(|v| v.as_str());

        let Some(schedule_str) = schedule_str else {
            debug!(config_id = %config.id, "Cron config has no schedule, skipping");
            continue;
        };

        let schedule: Schedule = match schedule_str.parse() {
            Ok(s) => s,
            Err(e) => {
                warn!(config_id = %config.id, schedule = %schedule_str, error = %e, "Invalid cron schedule");
                continue;
            }
        };

        // Check if schedule should have fired since last check.
        // For new configs, seed last_fired to now so the first fire happens
        // at the next scheduled occurrence (not retroactively).
        let last = *last_fired.entry(config.id.clone()).or_insert(now);
        let is_new = last == now;
        let next = schedule.after(&last).next();

        debug!(
            config_id = %config.id,
            schedule = %schedule_str,
            is_new,
            last = %last,
            next = ?next.map(|t| t.to_string()),
            now = %now,
            "Cron schedule check"
        );

        if let Some(next_fire) = next {
            if next_fire <= now {
                info!(config_id = %config.id, schedule = %schedule_str, "Cron schedule fired");

                let msg =
                    super::provider::CronProvider::generate_message(config);

                let state_clone = state.clone();
                tokio::spawn(async move {
                    if let Err(e) =
                        crate::channel::inbound::process_inbound(
                            &state_clone,
                            msg,
                        )
                        .await
                    {
                        warn!(error = %e, "Failed to process cron-triggered message");
                    }
                });

                last_fired.insert(config.id.clone(), now);
            }
        }
    }

    Ok(())
}
