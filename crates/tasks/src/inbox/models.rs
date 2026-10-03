//! Inbox items and the origin each one carries.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Who sent an item, as a sum type, so the `task_inbox_sender_check` CHECK is
/// never the first place a bad combination is caught.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InboxOrigin {
    User {
        sent_by: String,
    },
    Task {
        source_task_id: String,
        sent_by: Option<String>,
    },
    Adapter {
        channel: AdapterChannel,
        sender: ExternalSender,
        /// The Moatless user the external sender resolved to.
        sent_by: Option<String>,
    },
    /// A prompt the platform sent: a loop run, a first message with no sender.
    System,
    /// A prompt the Sandbox wrote to wake its own agent. Nobody said it.
    Sandbox,
    /// Delivered history whose sender did not record. Never submitted.
    Unknown,
}

/// The Adapter an item arrived through and its id there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterChannel {
    pub adapter: String,
    pub external_message_id: String,
    pub subject_key: Option<String>,
    pub source_name: Option<String>,
}

/// The external writer, in the Adapter's own terms.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExternalSender {
    /// The writer's id in the external system, e.g. a Slack `U0…`.
    pub id: Option<String>,
    pub author: Option<String>,
}

impl InboxOrigin {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::User { .. } => "user",
            Self::Task { .. } => "task",
            Self::Adapter { .. } => "adapter",
            Self::System => "system",
            Self::Sandbox => "sandbox",
            Self::Unknown => "unknown",
        }
    }

    pub fn sent_by(&self) -> Option<&str> {
        match self {
            Self::User { sent_by } => Some(sent_by),
            Self::Task { sent_by, .. } | Self::Adapter { sent_by, .. } => sent_by.as_deref(),
            Self::System | Self::Sandbox | Self::Unknown => None,
        }
    }

    pub fn source_task_id(&self) -> Option<&str> {
        match self {
            Self::Task { source_task_id, .. } => Some(source_task_id),
            _ => None,
        }
    }

    pub fn channel(&self) -> Option<&AdapterChannel> {
        match self {
            Self::Adapter { channel, .. } => Some(channel),
            _ => None,
        }
    }

    pub fn sender(&self) -> Option<&ExternalSender> {
        match self {
            Self::Adapter { sender, .. } => Some(sender),
            _ => None,
        }
    }
}

/// What the turn runner reads beside an item's text.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DeliveryOptions {
    pub model: Option<String>,
    pub agent_mode: Option<String>,
    pub effort: Option<String>,
    pub skills: Vec<String>,
    /// Continue the agent's current session; the last item of a delivery decides.
    pub resume: bool,
    pub start_fresh: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub contexts: Vec<InboxContext>,
    /// A tool response's structured answer, `{toolUseId, toolName, …}`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_response: Option<serde_json::Value>,
    /// This item re-runs, on the other agent, a turn its provider would not
    /// serve. A turn that delivered one never falls back again.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub agent_fallback: bool,
}

impl Default for DeliveryOptions {
    fn default() -> Self {
        Self {
            model: None,
            agent_mode: None,
            effort: None,
            skills: Vec::new(),
            resume: true,
            start_fresh: false,
            contexts: Vec::new(),
            tool_response: None,
            agent_fallback: false,
        }
    }
}

/// A preview URL, selected element or attached file sent with an item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InboxContext {
    #[serde(rename = "type")]
    pub context_type: String,
    pub reference_id: String,
    pub data: serde_json::Value,
}

/// An item to submit. `id` is derived by a send path that must be idempotent.
#[derive(Debug, Clone)]
pub struct NewInboxItem {
    pub id: Uuid,
    pub task_id: String,
    pub text: String,
    pub file_ids: Vec<String>,
    pub options: DeliveryOptions,
    /// The parked `AskUserQuestion`/`ExitPlanMode` call this item answers.
    pub answers_tool_use_id: Option<String>,
    pub origin: InboxOrigin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InboxStatus {
    Pending,
    Delivered,
    Cancelled,
    /// Delivered, then cut by a rewind; kept while a fork's Message reads it.
    Rewound,
}

/// A stored item.
#[derive(Debug, Clone)]
pub struct InboxItem {
    pub id: Uuid,
    pub task_id: String,
    pub created_at: DateTime<Utc>,
    pub text: String,
    pub file_ids: Vec<String>,
    pub options: DeliveryOptions,
    pub answers_tool_use_id: Option<String>,
    pub origin: InboxOrigin,
    pub status: InboxStatus,
    pub delivered_at: Option<DateTime<Utc>>,
    pub run_id: Option<String>,
    pub turn_number: Option<i32>,
    pub session_id: Option<String>,
}

/// The run a delivery joined, stamped on each item it delivers.
#[derive(Debug, Clone)]
pub struct RunStamp {
    pub run_id: Option<String>,
    pub session_id: String,
    pub turn_number: i32,
}

/// A `task_inbox` row as stored.
#[derive(sqlx::FromRow)]
pub(crate) struct InboxRow {
    pub id: Uuid,
    pub task_id: String,
    pub created_at: DateTime<Utc>,
    pub text: String,
    pub file_ids: Vec<String>,
    pub delivery_options: Option<serde_json::Value>,
    pub answers_tool_use_id: Option<String>,
    pub origin: String,
    pub sent_by: Option<String>,
    pub source_task_id: Option<String>,
    pub external_sender_id: Option<String>,
    pub external_author: Option<String>,
    pub adapter: Option<String>,
    pub external_message_id: Option<String>,
    pub external_subject_key: Option<String>,
    pub external_source_name: Option<String>,
    pub status: String,
    pub delivered_at: Option<DateTime<Utc>>,
    pub run_id: Option<String>,
    pub turn_number: Option<i32>,
    pub session_id: Option<String>,
}

impl TryFrom<InboxRow> for InboxItem {
    type Error = sqlx::Error;

    fn try_from(row: InboxRow) -> Result<Self, Self::Error> {
        let decode = |column: &str, value: &str| sqlx::Error::ColumnDecode {
            index: column.to_string(),
            source: format!("unexpected value {value:?}").into(),
        };
        let origin = match row.origin.as_str() {
            "user" => InboxOrigin::User {
                sent_by: row.sent_by.ok_or_else(|| decode("sent_by", "NULL"))?,
            },
            "task" => InboxOrigin::Task {
                source_task_id: row
                    .source_task_id
                    .ok_or_else(|| decode("source_task_id", "NULL"))?,
                sent_by: row.sent_by,
            },
            "adapter" => InboxOrigin::Adapter {
                channel: AdapterChannel {
                    adapter: row.adapter.ok_or_else(|| decode("adapter", "NULL"))?,
                    external_message_id: row
                        .external_message_id
                        .ok_or_else(|| decode("external_message_id", "NULL"))?,
                    subject_key: row.external_subject_key,
                    source_name: row.external_source_name,
                },
                sender: ExternalSender {
                    id: row.external_sender_id,
                    author: row.external_author,
                },
                sent_by: row.sent_by,
            },
            "system" => InboxOrigin::System,
            "sandbox" => InboxOrigin::Sandbox,
            "unknown" => InboxOrigin::Unknown,
            other => return Err(decode("origin", other)),
        };
        let status = match row.status.as_str() {
            "pending" => InboxStatus::Pending,
            "delivered" => InboxStatus::Delivered,
            "cancelled" => InboxStatus::Cancelled,
            "rewound" => InboxStatus::Rewound,
            other => return Err(decode("status", other)),
        };
        let options = row
            .delivery_options
            .map(serde_json::from_value)
            .transpose()
            .map_err(|e| sqlx::Error::ColumnDecode {
                index: "delivery_options".to_string(),
                source: Box::new(e),
            })?
            .unwrap_or_default();
        Ok(Self {
            id: row.id,
            task_id: row.task_id,
            created_at: row.created_at,
            text: row.text,
            file_ids: row.file_ids,
            options,
            answers_tool_use_id: row.answers_tool_use_id,
            origin,
            status,
            delivered_at: row.delivered_at,
            run_id: row.run_id,
            turn_number: row.turn_number,
            session_id: row.session_id,
        })
    }
}
