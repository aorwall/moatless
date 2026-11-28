//! Pure decisions the reaper takes per Task and per Sandbox; each reads only
//! its arguments, so `reaper::tests` drives them without a database.

use std::collections::HashMap;

use chrono::{DateTime, Utc};

/// Outcome of evaluating a sandbox for the idle-stop branch.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ReapDecision {
    /// Sandbox is idle past its TTL — stop it.
    Stop,
    /// Sandbox is still within its TTL — leave it running.
    Keep,
    /// Cannot decide (no activity timestamp, or TTL disabled). Reason is for
    /// logs.
    Skip(&'static str),
}

/// Whether an `agent` lifecycle `sub_type` marks the agent's run as ended.
///
/// The DAO query restricts to four sub-types, so anything not terminal here is
/// `started` (a running agent).
pub(crate) fn is_terminal_agent_sub_type(sub_type: &str) -> bool {
    matches!(sub_type, "completed" | "stopped" | "error")
}

/// Compute the idle-stop floor for a sandbox task, folding the latest agent
/// run-state into the raw activity floor.
///
/// A running agent floors to `now` (never reaped); a stopped agent floors to
/// `max(activity_floor, stop_time)` so the TTL counts from when it stopped, not
/// the last message; no agent event leaves `activity_floor` unchanged.
pub(crate) fn compute_reap_floor(
    agent_event: Option<&(String, DateTime<Utc>)>,
    activity_floor: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    match agent_event {
        Some((sub_type, _)) if !is_terminal_agent_sub_type(sub_type) => Some(now),
        Some((_, stopped_at)) => Some(match activity_floor {
            Some(existing) if existing > *stopped_at => existing,
            _ => *stopped_at,
        }),
        None => activity_floor,
    }
}

/// When a running agent last emitted an `agent` event, if that is more than
/// `threshold` before `now`; `None` for an agent that is not running.
///
/// Reads agent events alone, deliberately: a dead turn keeps receiving user
/// and child-task messages, and counting them would hide it.
pub(crate) fn silent_since(
    agent_event: Option<&(String, DateTime<Utc>)>,
    agent_activity: Option<DateTime<Utc>>,
    threshold: chrono::Duration,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    let (sub_type, started_at) = agent_event?;
    if is_terminal_agent_sub_type(sub_type) {
        return None;
    }
    let last = agent_activity.map_or(*started_at, |at| at.max(*started_at));
    (now - last > threshold).then_some(last)
}

/// The idle-stop floor for a whole Sandbox: the newest [`compute_reap_floor`]
/// across its Members.
///
/// A running Member contributes `now`, keeping the pod alive for every other
/// Member on it.
pub(crate) fn fold_reap_floor(
    members: &[String],
    agent_lifecycle: &HashMap<String, (String, DateTime<Utc>)>,
    latest_times: &HashMap<String, DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    members
        .iter()
        .filter_map(|member| {
            compute_reap_floor(
                agent_lifecycle.get(member),
                latest_times.get(member).copied(),
                now,
            )
        })
        .max()
}

/// The newest activity timestamp across a Sandbox's Members. Reported on the
/// stop, and never a decision input.
pub(crate) fn fold_latest_activity(
    members: &[String],
    latest_times: &HashMap<String, DateTime<Utc>>,
) -> Option<DateTime<Utc>> {
    members
        .iter()
        .filter_map(|member| latest_times.get(member).copied())
        .max()
}

/// The idle TTL for a whole Sandbox: the largest across its Members, and `0`
/// when any Member is pinned.
///
/// `decide_reap` reads `0` as indefinite, so a pinned Member pins the pod it
/// shares — matching the teardown guard in `run_remove`.
pub(crate) fn fold_ttl_minutes(
    members: &[String],
    ttl_map: &HashMap<String, i32>,
    default_ttl_minutes: i32,
) -> i32 {
    let mut folded = 0;
    for member in members {
        let ttl = ttl_map
            .get(member)
            .copied()
            .unwrap_or(default_ttl_minutes)
            .max(0);
        if ttl == 0 {
            return 0;
        }
        folded = folded.max(ttl);
    }
    folded
}

/// Pure decision function: decide whether to reap a single sandbox.
///
/// `ttl_minutes == 0` means pinned indefinitely — never reap; a missing
/// `latest_at` (no message, sandbox-start, or creation timestamp) skips rather
/// than risking a brand-new sandbox.
pub(crate) fn decide_reap(
    ttl_minutes: i32,
    latest_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> ReapDecision {
    if ttl_minutes <= 0 {
        return ReapDecision::Skip("ttl_minutes is 0 (indefinite)");
    }
    let Some(latest_at) = latest_at else {
        return ReapDecision::Skip("no activity timestamp");
    };
    let cutoff = now - chrono::Duration::minutes(ttl_minutes as i64);
    if latest_at < cutoff {
        ReapDecision::Stop
    } else {
        ReapDecision::Keep
    }
}

/// How long past a Sandbox's reap floor preview traffic alone can hold it.
///
/// A tab left open that polls its server would otherwise keep the pod up for
/// as long as the browser stays open.
pub(crate) const PREVIEW_HOLD_CAP_HOURS: i64 = 8;

/// Whether someone using a Script's preview defers an idle stop the floor
/// already called for.
///
/// Holds while the last proxied request is inside the idle TTL, and for at most
/// [`PREVIEW_HOLD_CAP_HOURS`] past `reap_floor`. A Script nobody has loaded
/// holds nothing, so a running Script alone never keeps a pod up.
pub(crate) fn preview_holds_the_sandbox(
    last_preview_request: Option<DateTime<Utc>>,
    reap_floor: Option<DateTime<Utc>>,
    ttl_minutes: i32,
    now: DateTime<Utc>,
) -> bool {
    let (Some(last_request), Some(floor)) = (last_preview_request, reap_floor) else {
        return false;
    };
    now - last_request < chrono::Duration::minutes(i64::from(ttl_minutes))
        && now - floor < chrono::Duration::hours(PREVIEW_HOLD_CAP_HOURS)
}

/// Outcome of evaluating an Open task for the auto-close branch.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CloseDecision {
    /// Task was settled longer ago than its TTL — close it.
    Close,
    /// Task is still within its TTL — leave it open.
    Keep,
    /// Cannot decide (not settled, or TTL disabled).
    Skip(&'static str),
}

/// Pure decision function: decide whether to auto-close a single Open task.
pub(crate) fn decide_close(
    close_after_settled_days: i32,
    settled_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> CloseDecision {
    if close_after_settled_days <= 0 {
        return CloseDecision::Skip("close_after_settled_days is 0 (indefinite)");
    }
    let Some(settled_at) = settled_at else {
        return CloseDecision::Skip("not settled");
    };
    let cutoff = now - chrono::Duration::days(close_after_settled_days as i64);
    if settled_at < cutoff {
        CloseDecision::Close
    } else {
        CloseDecision::Keep
    }
}

/// Whether an unsettled Open task has been idle long enough to settle.
/// `settle_after_hours <= 0` never settles, and neither does a task with no
/// activity timestamp.
pub(crate) fn decide_settle(
    settle_after_hours: i32,
    latest_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> bool {
    settle_after_hours > 0
        && latest_at
            .is_some_and(|at| now - at >= chrono::Duration::hours(i64::from(settle_after_hours)))
}

/// Whether a stopped sandbox has been stopped long enough to reclaim its
/// storage.
///
/// `cold_after_hours <= 0` disables the branch (`0` means indefinite, like
/// the other TTLs); `stopped_at` also anchors the pre-stop archive push, so
/// it drives both this clock and the freshness proof downstream.
pub(crate) fn decide_cold(
    cold_after_hours: i32,
    stopped_at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> bool {
    cold_after_hours > 0 && now - stopped_at >= chrono::Duration::hours(i64::from(cold_after_hours))
}
