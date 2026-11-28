//! Durable schedule-firing runner.

use std::sync::Arc;

use chrono::Utc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::loops::dao::LoopDao;
use crate::loops::firing::loop_firing::{Firing, FiringPorts};
use crate::loops::firing::metrics::record_schedule_firing;
use crate::loops::models::Schedule;
use crate::loops::schedule::next_fire_at;
use moatless_core::leader::LeaderGuard;

/// Durable schedule-firing runner: claims due schedules and fires each as an
/// inbound message, holding the ports it needs rather than the whole
/// `AppState`.
pub struct ScheduleRunner {
    loops: Arc<LoopDao>,
    ports: FiringPorts,
    worker_id: String,
}

impl ScheduleRunner {
    /// Assemble the runner from the ports it fires schedules through.
    pub(crate) fn new(loops: Arc<LoopDao>, ports: FiringPorts, worker_id: String) -> Self {
        Self {
            loops,
            ports,
            worker_id,
        }
    }

    /// Spawn the schedule-firing background task.
    ///
    /// The join handle is returned for the root supervisor to own — durable
    /// scheduled work stops firing if this loop ends.
    pub fn spawn(
        self,
        leader: Option<Arc<dyn LeaderGuard>>,
        cancel: CancellationToken,
    ) -> JoinHandle<()> {
        tokio::spawn(async move {
            info!("Schedule firing runner started");
            let worker_id = self.worker_id.clone();

            loop {
                tokio::select! {
                    () = cancel.cancelled() => {
                        info!("Schedule firing runner shutting down");
                        return;
                    }
                    () = tokio::time::sleep(std::time::Duration::from_secs(30)) => {
                        if let Some(ref guard) = leader {
                            match guard.try_acquire_or_renew().await {
                                Ok(true) => {}
                                Ok(false) => {
                                    debug!("Schedule firing runner not leader, skipping tick");
                                    record_schedule_firing("ignored", "not_leader", "");
                                    continue;
                                }
                                Err(e) => {
                                    warn!(error = %e, "Schedule firing leader election error, skipping tick");
                                    record_schedule_firing("error", "leader_election_failed", "");
                                    continue;
                                }
                            }
                        }

                        if let Err(e) = self.tick(&worker_id).await {
                            error!(error = %e, "Schedule firing tick failed");
                            record_schedule_firing("error", "tick_failed", "");
                        }
                    }
                }
            }
        })
    }

    async fn tick(&self, worker_id: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let locked_schedules = self.loops.claim_due_schedules(worker_id, 20).await?;

        for locked in locked_schedules {
            match self.execute_schedule_firing(&locked).await {
                Ok(()) => {
                    record_schedule_firing("success", "", &locked.id);
                }
                Err(e) => {
                    record_schedule_firing("error", "execution_failed", &locked.id);
                    warn!(schedule_id = %locked.id, error = %e, "Schedule firing execution failed");
                }
            }

            // Release the lock and reschedule this schedule. Contain failures per-schedule so a
            // single bad cron expression or DB hiccup doesn't strand the rest of the
            // already-locked batch until their 2-minute lock expiry.
            let next = match next_fire_at(&locked.cron_expression) {
                Ok(next) => next,
                Err(e) => {
                    record_schedule_firing("error", "next_fire_failed", &locked.id);
                    warn!(schedule_id = %locked.id, error = %format!("Invalid request: {e}"), "Failed to compute next fire time");
                    continue;
                }
            };
            if let Err(e) = self.loops.record_schedule_fire(&locked.id, next).await {
                record_schedule_firing("error", "record_fire_failed", &locked.id);
                warn!(schedule_id = %locked.id, error = %e, "Failed to record schedule fire");
            }
        }

        Ok(())
    }

    async fn execute_schedule_firing(
        &self,
        schedule: &Schedule,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let route = self.loops.find_loop_spawn_config(&schedule.loop_id).await?;
        let run_as_user_id = schedule
            .run_as_user_id
            .clone()
            .or_else(|| route.run_as_user_id.clone())
            .ok_or_else(|| {
                std::io::Error::other("claimed schedule Loop has no approved run-as user")
            })?;
        let firing = Firing {
            loop_id: schedule.id.clone(),
            text: schedule.message_template.clone(),
            run_as_user_id,
            requester: None,
            active: schedule.active,
            adapter_config: serde_json::json!({
                "message": schedule.message_template,
                "schedule": schedule.cron_expression,
                "timezone": schedule.timezone,
            }),
            event_id: format!("scheduled:{}:{}", schedule.id, Utc::now().timestamp()),
        };
        self.ports.fire(&route, &firing, &self.loops).await?;
        Ok(())
    }
}
