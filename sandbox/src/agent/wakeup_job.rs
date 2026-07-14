//! One-shot timer for the Claude Code `ScheduleWakeup` tool.
//!
//! When the agent calls `ScheduleWakeup`, the actor parses the tool input into
//! a `WakeupJob` and registers it against the shared `ScheduledJobRegistry`
//! once the turn ends successfully (latest-wins replaces any prior wakeup).
//! When the timer fires, the job hands the agent-supplied prompt back to the
//! actor via `ctx.injector.inject(...)`, which goes through `AgentHandle::send`
//! → mpsc → `queued_sends`. (One extra hop vs the previous in-loop push, but
//! functionally identical: if `Stop` is in flight the synthetic send is
//! dropped, same as today's behaviour.)

use std::time::Duration;

use serde_json::Value;
use tokio::time::Instant;
use tracing::debug;

use super::scheduled_job::{JobContext, JobKind, JobSnapshot, ScheduledJob, WakeupSnapshot};

/// One-shot wakeup job armed at the end of a successful turn.
pub struct WakeupJob {
    id: String,
    reason: String,
    prompt: String,
    wakeup_at: Instant,
    wakeup_at_unix_ms: i64,
    registered_at_unix_ms: i64,
}

impl WakeupJob {
    /// Parse a `ScheduleWakeup` tool input map. Returns `None` when any
    /// required field is missing.
    pub fn from_tool_use(id: &str, input: &Value) -> Option<Self> {
        let delay_seconds = input.get("delaySeconds")?.as_u64()?;
        let prompt = input.get("prompt")?.as_str()?.to_string();
        let reason = input.get("reason")?.as_str()?.to_string();

        let now_unix_ms = unix_now_ms();
        let wakeup_at_unix_ms = now_unix_ms + (delay_seconds as i64) * 1000;
        let wakeup_at = Instant::now() + Duration::from_secs(delay_seconds);

        Some(Self {
            id: id.to_string(),
            reason,
            prompt,
            wakeup_at,
            wakeup_at_unix_ms,
            registered_at_unix_ms: now_unix_ms,
        })
    }

    /// Test-only constructor that lets callers control the firing instant
    /// without having to fake the system clock.
    #[cfg(test)]
    pub(crate) fn new_for_test(
        id: String,
        reason: String,
        prompt: String,
        wakeup_at: Instant,
    ) -> Self {
        let now_unix_ms = unix_now_ms();
        Self {
            id,
            reason,
            prompt,
            wakeup_at,
            wakeup_at_unix_ms: now_unix_ms,
            registered_at_unix_ms: now_unix_ms,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn reason(&self) -> &str {
        &self.reason
    }

    pub fn wakeup_at(&self) -> Instant {
        self.wakeup_at
    }
}

#[async_trait::async_trait]
impl ScheduledJob for WakeupJob {
    fn id(&self) -> &str {
        &self.id
    }

    fn kind(&self) -> JobKind {
        JobKind::Wakeup
    }

    fn snapshot(&self) -> JobSnapshot {
        let prompt_preview: String = self.prompt.chars().take(200).collect();
        JobSnapshot::Wakeup(WakeupSnapshot {
            id: self.id.clone(),
            registered_at_unix_ms: self.registered_at_unix_ms,
            reason: self.reason.clone(),
            prompt_preview,
            wakeup_at_unix_ms: self.wakeup_at_unix_ms,
        })
    }

    async fn run(self: Box<Self>, ctx: JobContext) {
        tokio::select! {
            () = ctx.job_cancel.cancelled() => {
                debug!(id = %self.id, "Wakeup cancelled (job token)");
            }
            () = ctx.run_cancel.cancelled() => {
                debug!(id = %self.id, "Wakeup cancelled (run token)");
            }
            () = tokio::time::sleep_until(self.wakeup_at) => {
                ctx.injector.inject(self.prompt);
            }
        }
    }
}

fn unix_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_tool_use_parses_required_fields() {
        let input = serde_json::json!({
            "delaySeconds": 120,
            "prompt": "check build status",
            "reason": "waiting for build to complete"
        });
        let job = WakeupJob::from_tool_use("toolu_123", &input).expect("parse");
        assert_eq!(job.id(), "toolu_123");
        assert_eq!(job.reason, "waiting for build to complete");
        assert_eq!(job.prompt, "check build status");
        let JobSnapshot::Wakeup(snap) = job.snapshot() else {
            panic!("expected wakeup snapshot");
        };
        assert_eq!(snap.prompt_preview, "check build status");
        assert!(snap.wakeup_at_unix_ms > snap.registered_at_unix_ms);
    }

    #[test]
    fn from_tool_use_requires_all_fields() {
        let missing_prompt = serde_json::json!({
            "delaySeconds": 60,
            "reason": "x"
        });
        assert!(WakeupJob::from_tool_use("toolu_x", &missing_prompt).is_none());

        let missing_reason = serde_json::json!({
            "delaySeconds": 60,
            "prompt": "p"
        });
        assert!(WakeupJob::from_tool_use("toolu_x", &missing_reason).is_none());

        let missing_delay = serde_json::json!({
            "prompt": "p",
            "reason": "r"
        });
        assert!(WakeupJob::from_tool_use("toolu_x", &missing_delay).is_none());
    }
}
