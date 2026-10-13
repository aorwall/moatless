//! Opening one conversation: the Task band's transcript rows, with where each
//! Message came from and what its calls settled resolved beside them.

use std::collections::{HashMap, HashSet};
use std::ops::{Deref, DerefMut};

use moatless_ports::activity::{ActivitySource, SubagentUsage};
use moatless_tasks::conversation::transcript::TranscriptWindow;
use moatless_types::activity::{Activity, ActivityKind, ActivityView};
use serde_json::Value;

pub use moatless_tasks::conversation::transcript::{RewoundTurn, TranscriptEntry, TurnOutcome};

use crate::UiRpcDbPool;
use crate::dao::store_read;

/// One Message of a Task's transcript, with the names its origin resolves to.
///
/// Derefs to its [`TranscriptEntry`] so the projection reads one row; the
/// origin is joined from tables the Task band does not own.
#[derive(Debug, Clone)]
pub struct TranscriptMessage {
    pub entry: TranscriptEntry,
    /// The web address of the Binding this Message's subject key resolves to.
    /// `None` when no Binding matches or it records no url.
    pub binding_url: Option<String>,
    /// What the Binding calls its subject — a Linear issue identifier such as
    /// `SP-2454`. Preferred over `external_source_name`, which for Linear is
    /// the subscription's team id rather than anything a reader would
    /// recognise.
    pub binding_label: Option<String>,
    /// The name of the Loop a `scheduled` Message was fired by; `None` once
    /// the Loop is deleted.
    pub loop_name: Option<String>,
    /// The display name of the Moatless user `sent_by` resolved to. The `user`
    /// fallback where no external author was recorded.
    pub sent_by_name: Option<String>,
}

impl Deref for TranscriptMessage {
    type Target = TranscriptEntry;

    fn deref(&self) -> &TranscriptEntry {
        &self.entry
    }
}

impl DerefMut for TranscriptMessage {
    fn deref_mut(&mut self) -> &mut TranscriptEntry {
        &mut self.entry
    }
}

/// The names a Message's origin resolves to, as [`TranscriptMessage`] carries
/// them.
#[derive(Debug, Clone, Default, sqlx::FromRow)]
pub(crate) struct JoinedOrigin {
    pub uuid: String,
    pub binding_url: Option<String>,
    pub binding_label: Option<String>,
    pub loop_name: Option<String>,
    pub sent_by_name: Option<String>,
}

/// A Task's transcript, and whether it is all of it.
#[derive(Default)]
pub struct Transcript {
    /// Oldest first — the order a transcript is read in.
    pub messages: Vec<TranscriptMessage>,
    /// See [`TranscriptWindow::activities`].
    pub activities: Vec<Activity>,
    /// See [`TranscriptWindow::omitted`].
    pub omitted: i64,
    /// See [`TranscriptWindow::turns`]. Empty from
    /// [`crate::dao::ConversationDao::changed_messages`], whose caller projects
    /// no checkpoints.
    pub turns: Vec<i32>,
    /// See [`TranscriptWindow::turn_outcomes`].
    pub turn_outcomes: Vec<TurnOutcome>,
    /// See [`TranscriptWindow::rewound_turns`].
    pub rewound_turns: Vec<RewoundTurn>,
    /// Of the requests these rows ask, the ones whose call has come back.
    ///
    /// A panel row sits *beside* the call rather than being it — the call's
    /// status is the whole answer — and a resumed session can rewrite the panel
    /// onto a different Message than the call's, so this is looked up by
    /// request id rather than read out of `activities`.
    pub settled_requests: HashSet<String>,
    /// What each subagent these rows spawned has spent, by its call's
    /// `tool_use_id`.
    ///
    /// Looked up rather than read out of the rows: a subagent's own Messages
    /// carry the spawning id as `parent_uuid` and are never among `activities`.
    pub subagent_usage: HashMap<String, SubagentUsage>,
}

/// `window` as a [`Transcript`]: its Messages' origins joined, and the settled
/// requests and subagent usage its activities name.
pub(crate) async fn resolve(
    pool: &UiRpcDbPool,
    activity_source: &dyn ActivitySource,
    task_id: &str,
    window: TranscriptWindow,
) -> Result<Transcript, sqlx::Error> {
    let TranscriptWindow {
        entries,
        activities,
        omitted,
        turns,
        turn_outcomes,
        rewound_turns,
    } = window;
    let mut origins = crate::dao::transcript_origins(pool, task_id, &entries).await?;
    let messages = entries
        .into_iter()
        .map(|entry| {
            let origin = origins.remove(&entry.uuid).unwrap_or_default();
            TranscriptMessage {
                entry,
                binding_url: origin.binding_url,
                binding_label: origin.binding_label,
                loop_name: origin.loop_name,
                sent_by_name: origin.sent_by_name,
            }
        })
        .collect();
    let settled_requests = read_settled_requests(activity_source, task_id, &activities).await?;
    let subagent_usage = read_subagent_usage(activity_source, task_id, &activities).await?;
    Ok(Transcript {
        messages,
        activities,
        omitted,
        turns,
        turn_outcomes,
        rewound_turns,
        settled_requests,
        subagent_usage,
    })
}

/// Of the requests `activities` ask, the ones whose call has come back.
///
/// See [`Transcript::settled_requests`] for why the call isn't found among the
/// rows themselves.
async fn read_settled_requests(
    activity_source: &dyn ActivitySource,
    task_id: &str,
    activities: &[Activity],
) -> Result<HashSet<String>, sqlx::Error> {
    let asked: Vec<String> = activities
        .iter()
        .filter(|activity| activity.kind == ActivityKind::UserInputRequested)
        .filter_map(|activity| activity.payload.get("requestId").and_then(Value::as_str))
        .map(str::to_string)
        .collect();
    if asked.is_empty() {
        return Ok(HashSet::new());
    }
    activity_source
        .settled_calls(task_id, &asked)
        .await
        .map_err(store_read)
}

/// What each subagent these rows spawned has spent, keyed by the spawning call.
async fn read_subagent_usage(
    activity_source: &dyn ActivitySource,
    task_id: &str,
    activities: &[Activity],
) -> Result<HashMap<String, SubagentUsage>, sqlx::Error> {
    let spawned: Vec<String> = activities
        .iter()
        .filter(|activity| crate::activity::is_subagent_call(&ActivityView::new(activity)))
        .filter_map(|activity| activity.tool_use_id.clone())
        .collect();
    if spawned.is_empty() {
        return Ok(HashMap::new());
    }
    Ok(activity_source
        .subagent_usage(task_id, &spawned)
        .await
        .map_err(store_read)?
        .into_iter()
        .map(|usage| (usage.tool_use_id.clone(), usage))
        .collect())
}
