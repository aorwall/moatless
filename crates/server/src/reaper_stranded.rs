//! The reaper's stranded-inbox pass: re-wake an open Task holding a message that
//! nothing is set to deliver.
//!
//! The Sandbox reconciler re-notifies a pending wake only while the message
//! supersedes the Sandbox's intent, so one a later idle stop overtook waits
//! for the next send; this pass is the backstop for that and any lost wake.

use std::collections::HashMap;
use std::sync::Mutex;

use chrono::{DateTime, Duration, Utc};
use moatless_ports::sandbox::SandboxWaker;
use moatless_tasks::message::operations::Messages;
use tracing::warn;

/// How long a pending message may wait with no turn open before it counts as
/// stranded. Above a cold start's provision and restore time.
const STRANDED_AFTER_MINUTES: i64 = 10;

/// The least time between two re-wakes of one Task, so a pod that keeps
/// refusing the delivery is woken and logged on this cadence, not each cycle.
const REWAKE_SPACING_MINUTES: i64 = 15;

/// Wake every Task whose inbox is stranded, once per [`REWAKE_SPACING_MINUTES`],
/// and log and count each wake. `rewoken` is the pass's memory across cycles,
/// pruned to the Tasks still stranded.
pub async fn rewake_stranded_inboxes(
    messages: &Messages,
    waker: &dyn SandboxWaker,
    rewoken: &Mutex<HashMap<String, DateTime<Utc>>>,
    now: DateTime<Utc>,
) {
    let stranded = match messages
        .stranded_inboxes(now - Duration::minutes(STRANDED_AFTER_MINUTES))
        .await
    {
        Ok(stranded) => stranded,
        Err(e) => {
            warn!(error = %e, "stranded-inbox: failed to list stranded inboxes");
            return;
        }
    };

    let due: Vec<(String, DateTime<Utc>)> = {
        let mut rewoken = rewoken
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        rewoken.retain(|task_id, _| stranded.iter().any(|(id, _)| id == task_id));
        stranded
            .into_iter()
            .filter(|(task_id, _)| {
                let due = rewoken.get(task_id).is_none_or(|at| {
                    now.signed_duration_since(*at) >= Duration::minutes(REWAKE_SPACING_MINUTES)
                });
                if due {
                    rewoken.insert(task_id.clone(), now);
                }
                due
            })
            .collect()
    };

    for (task_id, oldest_pending_at) in due {
        let waiting_minutes = now.signed_duration_since(oldest_pending_at).num_minutes();
        warn!(
            task_id = %task_id,
            oldest_pending_at = %oldest_pending_at,
            waiting_minutes,
            "stranded-inbox: a pending message has had no turn; re-waking the task"
        );
        metrics::counter!("inbox_stranded_rewakes_total").increment(1);
        if let Err(e) = waker.wake(&task_id).await {
            warn!(task_id = %task_id, error = %e, "stranded-inbox: re-wake failed");
        }
    }
}
