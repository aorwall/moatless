//! The agent's reply to an `artifact` notice, saved as a record its page reads.

use std::sync::Arc;

use moatless_types::access_control::ScopeContext;

use super::error::ArtifactError;
use super::operations::Artifacts;
use super::records::write_as_user;
use super::schemas::{ArtifactRecordResponse, WriteArtifactRecordRequest};
use crate::task::operations::TaskCommands;

/// Where the reply to the notice `message_id` is saved. A page shows replies by
/// subscribing to the `agent-reply/` prefix.
pub fn agent_reply_key(message_id: &str) -> String {
    format!("agent-reply/{message_id}")
}

/// Saves the agent's replies to `artifact` notices as records.
pub struct NoticeReplies {
    tasks: Arc<TaskCommands>,
    artifacts: Arc<Artifacts>,
}

impl NoticeReplies {
    pub fn new(tasks: Arc<TaskCommands>, artifacts: Arc<Artifacts>) -> Self {
        Self { tasks, artifacts }
    }

    /// Save `text` under [`agent_reply_key`] on the Artifact `slug` of
    /// `task_id`, the notice's `subject_key`, as `scope` and past the record
    /// rules. `NotFound` when the Task has no Artifact `slug`.
    pub async fn reply(
        &self,
        scope: &ScopeContext,
        task_id: &str,
        slug: &str,
        message_id: &str,
        text: &str,
    ) -> Result<ArtifactRecordResponse, ArtifactError> {
        let row = self
            .artifacts
            .list_for_task(task_id)
            .await?
            .into_iter()
            .find(|row| row.slug == slug)
            .ok_or_else(|| ArtifactError::NotFound(slug.to_string()))?;
        let req = WriteArtifactRecordRequest::Save {
            key: agent_reply_key(message_id),
            data: serde_json::json!({ "text": text, "messageId": message_id }),
        };
        write_as_user(&self.tasks, &self.artifacts, scope, &row.artifact_id, req).await
    }
}
