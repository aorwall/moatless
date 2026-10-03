//! The cursor an archived listing page resumes from.

use chrono::{DateTime, Utc};

use crate::dao::Conversation;

/// Where an archive page resumes: the sort key of the last row the page
/// before it returned, in the listing's `updated_at DESC NULLS LAST, task_id`
/// order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveCursor {
    pub(crate) updated_at: Option<DateTime<Utc>>,
    pub(crate) task_id: String,
}

impl ArchiveCursor {
    /// The cursor resuming after `conversation`.
    pub fn after(conversation: &Conversation) -> Self {
        Self {
            updated_at: conversation.updated_at,
            task_id: conversation.task_id.clone(),
        }
    }

    /// The opaque text a client echoes back: `<updated_at>|<task_id>`, the
    /// time empty when absent. Microsecond precision, since that is what
    /// Postgres stores and a coarser time would skip or repeat rows.
    pub fn encode(&self) -> String {
        let updated_at = self
            .updated_at
            .map(|at| at.to_rfc3339_opts(chrono::SecondsFormat::Micros, true))
            .unwrap_or_default();
        format!("{updated_at}|{}", self.task_id)
    }

    /// `None` for text [`Self::encode`] did not produce.
    pub fn decode(text: &str) -> Option<Self> {
        let (updated_at, task_id) = text.split_once('|')?;
        if task_id.is_empty() {
            return None;
        }
        let updated_at = match updated_at {
            "" => None,
            at => Some(DateTime::parse_from_rfc3339(at).ok()?.with_timezone(&Utc)),
        };
        Some(Self {
            updated_at,
            task_id: task_id.to_owned(),
        })
    }
}
