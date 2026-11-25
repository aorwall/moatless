//! The scoped read projections a T3 answer is assembled from, as one handle.

use std::sync::Arc;

use crate::dao::ConversationDao;
use crate::poll_gate::PollGate;
use crate::workspace_listing::WorkspaceListing;
use moatless_core::change_feed::listing::ListingHead;
use moatless_ports::task::TaskReader;

/// The three read projections behind the UI-RPC handlers.
///
/// Every statement reachable from here is a read; a caller that has to write
/// asks through a port.
#[derive(Clone)]
pub struct T3Reads {
    conversations: Arc<dyn ConversationDao>,
    workspaces: Arc<dyn WorkspaceListing>,
    tasks: Arc<dyn TaskReader>,
    poll_gate: PollGate,
}

impl T3Reads {
    pub(crate) fn new(
        conversations: Arc<dyn ConversationDao>,
        workspaces: Arc<dyn WorkspaceListing>,
        tasks: Arc<dyn TaskReader>,
        poll_gate: PollGate,
    ) -> Self {
        Self {
            conversations,
            workspaces,
            tasks,
            poll_gate,
        }
    }

    /// The replica's bound on subscription polls and snapshot builds; shared
    /// by every clone of these reads.
    pub(crate) fn poll_gate(&self) -> &PollGate {
        &self.poll_gate
    }

    /// Where the shell change log stands, and which database allocated the
    /// positions on it.
    pub async fn listing_head(&self) -> Result<ListingHead, sqlx::Error> {
        self.conversations.listing_head().await
    }

    pub(crate) fn conversations(&self) -> &dyn ConversationDao {
        self.conversations.as_ref()
    }

    pub(crate) fn workspaces(&self) -> &dyn WorkspaceListing {
        self.workspaces.as_ref()
    }

    pub(crate) fn conversations_arc(&self) -> Arc<dyn ConversationDao> {
        self.conversations.clone()
    }

    pub(crate) fn tasks_arc(&self) -> Arc<dyn TaskReader> {
        self.tasks.clone()
    }

    /// The same reads over a different Task projection, for a caller asserting
    /// what one viewer is allowed to read.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn with_tasks(&self, tasks: Arc<dyn TaskReader>) -> Self {
        Self {
            tasks,
            ..self.clone()
        }
    }
}

#[cfg(feature = "test-support")]
mod refusal {
    use super::*;
    use crate::dao::{
        ArchiveCursor, Conversation, ConversationBrowse, ConversationMatch, InteractiveCall,
        ListingError,
    };
    use crate::transcript::Transcript;
    use async_trait::async_trait;
    use moatless_core::change_feed::conversation::ConversationHead;
    use moatless_core::change_feed::listing::ListingChange;
    use moatless_tasks::conversation::transcript::TranscriptEntry;
    use moatless_types::access_control::ScopeContext;
    use std::collections::HashMap;

    impl T3Reads {
        /// The same reads with the followed-row re-read refusing the way a pool
        /// that has gone away refuses it, for a caller asserting where a poll's
        /// failure lands.
        #[must_use]
        pub fn refusing_the_row_read(&self) -> Self {
            Self {
                conversations: Arc::new(RefusesTheRowRead(self.conversations.clone())),
                ..self.clone()
            }
        }

        /// The same reads with the workspace listing taken from `other`, for a
        /// caller that fails one half of a poll and not the other.
        #[must_use]
        pub fn with_workspaces_from(&self, other: &Self) -> Self {
            Self {
                workspaces: other.workspaces.clone(),
                ..self.clone()
            }
        }

        /// The ids of the conversations one viewer follows, off the query the
        /// shell listing is assembled from.
        pub async fn followed_task_ids(
            &self,
            viewer: &ScopeContext,
        ) -> Result<Vec<String>, ListingError> {
            Ok(self
                .conversations
                .list_followed(viewer)
                .await?
                .into_iter()
                .map(|conversation| conversation.task_id)
                .collect())
        }

        /// The shell change log after `after`, at most `limit` entries.
        pub async fn listing_changes_after(
            &self,
            after: i64,
            limit: i64,
        ) -> Result<Vec<ListingChange>, sqlx::Error> {
            self.conversations.listing_changes_after(after, limit).await
        }
    }

    /// Answers the shell feed's cursor reads and refuses the one that comes
    /// after them: the database is there and the connection is not.
    struct RefusesTheRowRead(Arc<dyn ConversationDao>);

    #[async_trait]
    impl ConversationDao for RefusesTheRowRead {
        async fn list_followed(
            &self,
            viewer: &ScopeContext,
        ) -> Result<Vec<Conversation>, ListingError> {
            self.0.list_followed(viewer).await
        }

        async fn list_followed_closed(
            &self,
            viewer: &ScopeContext,
            before: Option<ArchiveCursor>,
        ) -> Result<Vec<Conversation>, ListingError> {
            self.0.list_followed_closed(viewer, before).await
        }

        async fn list_browsable(
            &self,
            viewer: &ScopeContext,
            filter: &ConversationBrowse,
        ) -> Result<Vec<Conversation>, ListingError> {
            self.0.list_browsable(viewer, filter).await
        }

        async fn list_followed_by_ids(
            &self,
            _viewer: &ScopeContext,
            _task_ids: &[String],
        ) -> Result<Vec<Conversation>, sqlx::Error> {
            Err(sqlx::Error::PoolTimedOut)
        }

        async fn readable_by_id(
            &self,
            viewer: &ScopeContext,
            task_id: &str,
        ) -> Result<Option<Conversation>, sqlx::Error> {
            self.0.readable_by_id(viewer, task_id).await
        }

        async fn readable_by_ids(
            &self,
            viewer: &ScopeContext,
            task_ids: &[String],
        ) -> Result<Vec<Conversation>, sqlx::Error> {
            self.0.readable_by_ids(viewer, task_ids).await
        }

        async fn search_conversations(
            &self,
            viewer: &ScopeContext,
            query: &str,
            limit: i64,
        ) -> Result<Vec<ConversationMatch>, sqlx::Error> {
            self.0.search_conversations(viewer, query, limit).await
        }

        async fn can_read_conversation(
            &self,
            viewer: &ScopeContext,
            task_id: &str,
        ) -> Result<bool, sqlx::Error> {
            self.0.can_read_conversation(viewer, task_id).await
        }

        async fn writes_conversation(
            &self,
            viewer: &ScopeContext,
            task_id: &str,
        ) -> Result<bool, sqlx::Error> {
            self.0.writes_conversation(viewer, task_id).await
        }

        async fn owns_conversation(
            &self,
            viewer: &ScopeContext,
            task_id: &str,
        ) -> Result<bool, sqlx::Error> {
            self.0.owns_conversation(viewer, task_id).await
        }

        async fn transcript(&self, task_id: &str) -> Result<Transcript, sqlx::Error> {
            self.0.transcript(task_id).await
        }

        async fn messages_after(
            &self,
            task_id: &str,
            after: Option<i32>,
            limit: i64,
        ) -> Result<Vec<TranscriptEntry>, sqlx::Error> {
            self.0.messages_after(task_id, after, limit).await
        }

        async fn source_task_names(
            &self,
            viewer: &ScopeContext,
            task_ids: &[String],
        ) -> Result<HashMap<String, String>, sqlx::Error> {
            self.0.source_task_names(viewer, task_ids).await
        }

        async fn conversation_head(&self, task_id: &str) -> Result<ConversationHead, sqlx::Error> {
            self.0.conversation_head(task_id).await
        }

        async fn conversation_heads(
            &self,
            task_ids: &[String],
        ) -> Result<Vec<(String, ConversationHead)>, sqlx::Error> {
            self.0.conversation_heads(task_ids).await
        }

        async fn interactive_call(
            &self,
            task_id: &str,
            tool_use_id: &str,
        ) -> Result<Option<InteractiveCall>, sqlx::Error> {
            self.0.interactive_call(task_id, tool_use_id).await
        }

        async fn listing_head(&self) -> Result<ListingHead, sqlx::Error> {
            self.0.listing_head().await
        }

        async fn listing_changes_after(
            &self,
            after: i64,
            limit: i64,
        ) -> Result<Vec<ListingChange>, sqlx::Error> {
            self.0.listing_changes_after(after, limit).await
        }

        async fn pull_request_host_changes(&self) -> Result<i64, sqlx::Error> {
            self.0.pull_request_host_changes().await
        }
    }
}
