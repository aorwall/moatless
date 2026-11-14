//! A Sandbox's start as T3's worktree setup progress: `subscribeWorktreeSetup`
//! renders the start's steps as the card's stages, and `worktreeSetup.cancel`
//! stops the start.
//!
//! The steps are the Task's `sandbox` events from the newest acceptance on,
//! classed by the in-sandbox `reason` or the watcher's `lifecycleEvent`.

use async_trait::async_trait;
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use serde_json::{Value, json};

use moatless_tasks::task_event::operations::TaskEvents;

/// How many of a Task's newest `sandbox` events a snapshot reads. A start
/// publishes about ten.
pub(crate) const START_EVENT_WINDOW: i64 = 50;

/// How long a finished start stays reported. An older one answers `null`, so
/// a thread opened days later shows no card for a start nobody watched.
const FINISHED_START_SHOWN_FOR: Duration = Duration::minutes(10);

/// `WORKTREE_SETUP_DETAIL_MAX_LENGTH` in the contract's `worktreeSetup.ts`.
const DETAIL_MAX_CHARS: usize = 200;
/// `WORKTREE_SETUP_ERROR_MAX_LENGTH` in the same file.
const ERROR_MAX_CHARS: usize = 1000;

/// One persisted `sandbox` event of a Task.
#[derive(Debug, Clone)]
pub(crate) struct SandboxLogEvent {
    /// The `task_events` row id, which orders a Task's events.
    pub(crate) id: i64,
    pub(crate) sub_type: String,
    pub(crate) payload: Value,
    pub(crate) at: DateTime<Utc>,
}

/// The Task's `sandbox` events a setup snapshot is read from.
#[async_trait]
pub(crate) trait SandboxEventLog: Send + Sync {
    /// The newest `limit` events, oldest first.
    async fn recent_sandbox_events(
        &self,
        task_id: &str,
        limit: i64,
    ) -> Result<Vec<SandboxLogEvent>, sqlx::Error>;
}

#[async_trait]
impl SandboxEventLog for TaskEvents {
    async fn recent_sandbox_events(
        &self,
        task_id: &str,
        limit: i64,
    ) -> Result<Vec<SandboxLogEvent>, sqlx::Error> {
        let events = self
            .list_recent_events(task_id, &Some(vec!["sandbox".to_string()]), limit, 0)
            .await?;
        Ok(events
            .into_iter()
            .map(|event| SandboxLogEvent {
                id: event.id,
                sub_type: event.sub_type,
                payload: event.payload,
                at: event.created_at,
            })
            .collect())
    }
}

/// A fixed log, for tests.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct LoggedEvents(pub(crate) std::sync::Mutex<Vec<SandboxLogEvent>>);

#[cfg(test)]
#[async_trait]
impl SandboxEventLog for LoggedEvents {
    async fn recent_sandbox_events(
        &self,
        _task_id: &str,
        _limit: i64,
    ) -> Result<Vec<SandboxLogEvent>, sqlx::Error> {
        Ok(self.0.lock().expect("no poisoned lock").clone())
    }
}

/// The contract's `WorktreeSetupStreamEvent` for one Task: its newest start, or
/// `null` when it has none or that start finished before
/// [`FINISHED_START_SHOWN_FOR`].
pub(crate) fn snapshot(thread_id: &str, events: &[SandboxLogEvent], now: DateTime<Utc>) -> Value {
    let Some(start) = events.iter().rposition(is_acceptance) else {
        return Value::Null;
    };
    let accepted = &events[start];
    let start = Start::read(&events[start..]);
    let phase = start.phase();
    let ended_at = match phase {
        Phase::Running => None,
        Phase::Done => start.ready.max(start.setup_end.map(|(at, _)| at)),
        Phase::Failed | Phase::Cancelled => start.ended.as_ref().map(|(at, _)| *at),
    };
    if ended_at.is_some_and(|ended| now - ended > FINISHED_START_SHOWN_FOR) {
        return Value::Null;
    }
    json!({
        "threadId": thread_id,
        "phase": phase.as_str(),
        "startedAt": instant(accepted.at),
        "endedAt": ended_at.map(instant),
        "branch": Value::Null,
        "baseRef": Value::Null,
        "worktreePath": Value::Null,
        "setupScript": Value::Null,
        "stages": start.stages(accepted.at, phase),
        "error": match (phase, &start.ended) {
            (Phase::Failed, Some((_, message))) => json!(clip(message, ERROR_MAX_CHARS)),
            _ => Value::Null,
        },
        "sequence": events.last().map_or(0, |event| event.id.max(0)),
    })
}

/// Whether a stage may be cancelled: the start is still running.
pub(crate) fn is_running(snapshot: &Value) -> bool {
    snapshot.get("phase").and_then(Value::as_str) == Some(Phase::Running.as_str())
}

/// The lifecycle's acceptance of a start: `initializing` with neither an
/// in-sandbox `reason` nor a watcher `lifecycleEvent`.
fn is_acceptance(event: &SandboxLogEvent) -> bool {
    event.sub_type == "initializing"
        && text(&event.payload, "reason").is_none()
        && text(&event.payload, "lifecycleEvent").is_none()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Running,
    Done,
    Failed,
    Cancelled,
}

impl Phase {
    fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

type Step = (DateTime<Utc>, Option<String>);

/// What one start's events said, each step at its first report.
#[derive(Default)]
struct Start {
    scheduled: Option<Step>,
    credentials: Option<Step>,
    checkout: Option<Step>,
    plugins: Option<Step>,
    ready: Option<DateTime<Utc>>,
    setup_running: Option<Step>,
    /// The end of the setup commands, and whether one failed.
    setup_end: Option<(DateTime<Utc>, bool)>,
    /// Which stage failed, by [`STAGES`] index, when one did.
    failed_stage: Option<usize>,
    /// A stop or failure before the Sandbox was ready, and what it said.
    ended: Option<(DateTime<Utc>, String)>,
    cancelled: bool,
}

/// The stages in the order a Moatless start reaches them: the agent is ready
/// before the setup commands run.
const STAGES: [&str; 5] = ["fetch", "checkout", "submodules", "agent", "setup-script"];

impl Start {
    fn read(events: &[SandboxLogEvent]) -> Self {
        let mut start = Self::default();
        for event in events {
            if start.ended.is_some() {
                break;
            }
            let message = text(&event.payload, "message").map(str::to_string);
            let step = (event.at, message.clone());
            match (
                event.sub_type.as_str(),
                text(&event.payload, "reason"),
                text(&event.payload, "lifecycleEvent"),
            ) {
                ("initializing", Some("CredentialsFetched"), _) => {
                    start.credentials.get_or_insert(step);
                }
                ("initializing", Some("CredentialsFailed"), _) => start.fail(0, step),
                ("initializing", Some("RepositoryRestored" | "RepositoryCloned"), _) => {
                    start.checkout.get_or_insert(step);
                }
                ("initializing", Some("RepositoryCloneFailed"), _) => start.fail(1, step),
                ("initializing", Some("PluginsCloned"), _) => {
                    start.plugins.get_or_insert(step);
                }
                ("initializing", Some("SetupRunning"), _) => {
                    start.setup_running.get_or_insert(step);
                }
                ("initializing", Some("SetupFailed"), _) => {
                    start.setup_running.get_or_insert(step);
                    start.setup_end.get_or_insert((event.at, true));
                }
                ("initializing", Some("WorkspaceSetupComplete"), _) => {
                    start.setup_end.get_or_insert((event.at, false));
                }
                ("initializing", None, Some("pod_scheduled")) => {
                    start.scheduled.get_or_insert(step);
                }
                ("ready", _, _) => {
                    start.ready.get_or_insert(event.at);
                }
                ("error", _, _) if start.ready.is_none() => {
                    start.ended = Some((event.at, message.unwrap_or_default()));
                }
                ("stopped", _, _) if start.ready.is_none() => {
                    start.cancelled = true;
                    start.ended = Some((event.at, message.unwrap_or_default()));
                }
                _ => {}
            }
        }
        start
    }

    /// A step reported its own failure. Credentials and the clone end the
    /// start; a failed setup command does not, and is a warning instead.
    fn fail(&mut self, stage: usize, (at, message): Step) {
        self.failed_stage.get_or_insert(stage);
        self.ended.get_or_insert((at, message.unwrap_or_default()));
    }

    fn phase(&self) -> Phase {
        match (&self.ended, self.cancelled) {
            (Some(_), true) => Phase::Cancelled,
            (Some(_), false) => Phase::Failed,
            (None, _) if self.ready.is_some() && self.setup_end_or_none() => Phase::Done,
            (None, _) => Phase::Running,
        }
    }

    /// The setup commands ended, or none started by the time the agent was
    /// ready. A command that starts later turns the start back to running.
    fn setup_end_or_none(&self) -> bool {
        self.setup_running.is_none() || self.setup_end.is_some()
    }

    /// Each stage's end, by [`STAGES`] index: `Some` once it finished or a
    /// later stage reported, which skips it.
    fn ends(&self) -> [Option<DateTime<Utc>>; 5] {
        [
            self.credentials.as_ref().map(|(at, _)| *at),
            self.checkout.as_ref().map(|(at, _)| *at),
            self.plugins.as_ref().map(|(at, _)| *at),
            self.ready,
            self.setup_end.map(|(at, _)| at),
        ]
    }

    fn stages(&self, accepted_at: DateTime<Utc>, phase: Phase) -> Vec<Value> {
        let ends = self.ends();
        // When the next stage could have begun: the end of the last one that
        // reported one.
        let mut began = accepted_at;
        let mut stages = Vec::with_capacity(STAGES.len());
        for (index, id) in STAGES.into_iter().enumerate() {
            let reached_later = ends[index + 1..].iter().any(Option::is_some);
            let (status, ended_at, detail) = self.stage(index, ends[index], reached_later, phase);
            let started_at = match status {
                "pending" | "skipped" => None,
                _ if index == 4 => self.setup_running.as_ref().map(|(at, _)| *at),
                _ => Some(began),
            };
            if let Some(ended) = ended_at {
                began = ended;
            }
            stages.push(json!({
                "id": id,
                "status": status,
                "startedAt": started_at.map(instant),
                "endedAt": ended_at.map(instant),
                "percent": Value::Null,
                "detail": detail.map(|detail| clip(&detail, DETAIL_MAX_CHARS)),
                "tail": [],
            }));
        }
        stages
    }

    /// One stage's `(status, endedAt, detail)`, by [`STAGES`] index.
    fn stage(
        &self,
        index: usize,
        end: Option<DateTime<Utc>>,
        reached_later: bool,
        phase: Phase,
    ) -> (&'static str, Option<DateTime<Utc>>, Option<String>) {
        let detail = |step: &Option<Step>| step.as_ref().and_then(|(_, message)| message.clone());
        let failure = || {
            (
                "failed",
                self.ended.as_ref().map(|(at, _)| *at),
                self.ended.as_ref().map(|(_, message)| message.clone()),
            )
        };
        if self.failed_stage == Some(index) {
            return failure();
        }
        if index == 4 {
            return match (&self.setup_running, self.setup_end) {
                (Some(_), Some((at, true))) => ("warning", Some(at), detail(&self.setup_running)),
                (Some(_), Some((at, false))) => ("done", Some(at), detail(&self.setup_running)),
                (Some(_), None) if phase == Phase::Running => {
                    ("running", None, detail(&self.setup_running))
                }
                (None, _) if phase == Phase::Running && self.ready.is_none() => {
                    ("pending", None, None)
                }
                _ => ("skipped", None, None),
            };
        }
        // Every stage before this one has ended or been passed.
        let current = index == 0 || self.ends()[index - 1].is_some();
        match (end, reached_later) {
            (Some(at), _) => (
                "done",
                Some(at),
                match index {
                    0 => detail(&self.credentials).or_else(|| detail(&self.scheduled)),
                    1 => detail(&self.checkout),
                    2 => detail(&self.plugins),
                    _ => None,
                },
            ),
            // Fetching credentials is never skipped: a later step means it
            // happened unreported.
            (None, true) if index == 0 => ("done", None, detail(&self.scheduled)),
            (None, true) => ("skipped", None, None),
            (None, false) if !current => match phase {
                Phase::Running => ("pending", None, None),
                _ => ("skipped", None, None),
            },
            (None, false) => match phase {
                Phase::Running => (
                    "running",
                    None,
                    (index == 0).then(|| detail(&self.scheduled)).flatten(),
                ),
                // No step owned the failure, so the one in flight did.
                Phase::Failed if self.failed_stage.is_none() => failure(),
                _ => ("skipped", None, None),
            },
        }
    }
}

fn text<'a>(payload: &'a Value, key: &str) -> Option<&'a str> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn clip(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn instant(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Millis, true)
}

#[cfg(test)]
#[path = "sandbox_setup_tests.rs"]
mod tests;
