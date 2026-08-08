//! Outbound adapter notifications — event-driven replies and explicit sends.

use std::sync::Arc;

use tracing::warn;

use moatless_adapter_model::models::{OutboundFile, OutboundReply, TaskBinding};
use moatless_backend_core::scope::ScopeContext;
use moatless_core::config::{AuthConfig, S3Config};

use crate::adapters::dao::AdapterConnectionDao;
use crate::adapters::error::{AdapterError, SharedAdapterError};
use crate::adapters::{AdapterKind, Adapters};
use crate::task::binding_dao::TaskBindingDao;
use crate::bindings::metrics::record_outbound;
use crate::bindings::recently_sent::RecentlySent;
use crate::file_storage::dao::FileDao;
use crate::file_storage::s3::S3Storage;
use crate::loops::dao::LoopDao;
use crate::message::dao::MessageDao;
use crate::message::models::{MessageRole, MessageType, TaskMessage};
use crate::task::dao::TaskDao;
use crate::task_activities::mapping::ConversationWrite;
use crate::task_turn::dao::TaskTurnDao;

/// Binding-based outbound delivery, owning the ports the send path reads rather
/// than the whole `AppState`. Held by `ExternalReplyService`, which is the only
/// consumer; this is a module boundary made concrete (a plain struct, no trait,
/// no second implementation) rather than a speculative abstraction — what earns
/// it is the parameter list `send_to_binding` would otherwise carry.
pub(crate) struct OutboundSender {
    task_bindings: Arc<dyn TaskBindingDao>,
    adapter_connections: Arc<dyn AdapterConnectionDao>,
    loops: Arc<LoopDao>,
    tasks: Arc<dyn TaskDao>,
    recently_sent: Arc<RecentlySent>,
    messages: Arc<dyn MessageDao>,
    task_turns: Arc<dyn TaskTurnDao>,
    s3: Option<S3Storage>,
    auth_config: Arc<AuthConfig>,
    s3_config: Arc<S3Config>,
    files: Arc<dyn FileDao>,
}

impl OutboundSender {
    #[allow(clippy::too_many_arguments)]
    // reason: a composition-root constructor takes exactly the ports its struct
    // holds; the alternative is threading `AppState` back in, which is the edge
    // this slice removes.
    pub(crate) fn new(
        task_bindings: Arc<dyn TaskBindingDao>,
        adapter_connections: Arc<dyn AdapterConnectionDao>,
        loops: Arc<LoopDao>,
        tasks: Arc<dyn TaskDao>,
        recently_sent: Arc<RecentlySent>,
        messages: Arc<dyn MessageDao>,
        task_turns: Arc<dyn TaskTurnDao>,
        s3: Option<S3Storage>,
        auth_config: Arc<AuthConfig>,
        s3_config: Arc<S3Config>,
        files: Arc<dyn FileDao>,
    ) -> Self {
        Self {
            task_bindings,
            adapter_connections,
            loops,
            tasks,
            recently_sent,
            messages,
            task_turns,
            s3,
            auth_config,
            s3_config,
            files,
        }
    }

    /// Send a reply to the external target referenced by a task binding.
    ///
    /// Loads the binding, its adapter connection, and its optional
    /// subscription loop, then calls `send_reply_for_binding` on the matching
    /// adapter. Persists a visible assistant stub up front, tags it with the
    /// returned external id, populates the recently_sent race-window cache, and
    /// advances the binding cursor.
    pub(crate) async fn send_to_binding(
        &self,
        providers: &Adapters,
        task_id: &str,
        binding_id: &str,
        text: &str,
        file_ids: &[String],
    ) -> Result<(u32, Vec<String>), AdapterError> {
        let binding_repo = self.task_bindings.clone();
        let binding = binding_repo.find_by_id(binding_id).await?.ok_or_else(|| {
            SharedAdapterError::ConfigNotFound(format!("No task binding {binding_id}"))
        })?;

        if binding.task_id != task_id {
            return Err(SharedAdapterError::BadRequest(format!(
                "Binding {binding_id} does not belong to task {task_id}"
            ))
            .into());
        }

        #[allow(clippy::disallowed_methods)]
        // reason: outbound reply path runs as background system, no caller identity
        let sys = ScopeContext::system();
        let connection = self
            .adapter_connections
            .find_by_id(&binding.connection_id, &sys)
            .await?
            .ok_or_else(|| {
                SharedAdapterError::ConfigNotFound(format!(
                    "Provider connection {} not found",
                    binding.connection_id
                ))
            })?;

        let rule = if let Some(ref subscription_id) = binding.loop_id {
            self.loops.find_subscription_by_id(subscription_id).await?
        } else {
            None
        };

        let provider = providers.outbound_for_binding_reply_by_name(&binding.adapter_kind)?;
        let adapter_label = provider.provider_key();

        let binding_loop_id = binding.loop_id.as_deref().unwrap_or("");
        let files = self.resolve_files(file_ids).await;
        if !files.is_empty() && !provider.supports_files() {
            record_outbound(
                adapter_label,
                "error",
                "unsupported_feature",
                binding_loop_id,
                &binding.connection_id,
            );
            return Ok((
                0,
                vec![format!(
                    "Provider {:?} does not support file attachments",
                    provider.adapter_kind()
                )],
            ));
        }

        let footer = self
            .maybe_task_comment_footer(provider.adapter_kind(), task_id)
            .await;

        // Author the reply under the owning task's identity (preferred by the GitHub
        // adapter over binding/trigger-derived identities).
        #[allow(clippy::disallowed_methods)]
        // reason: outbound reply dispatched by background runtime, no caller identity
        let sys_author = ScopeContext::system();
        let author_user_id = self
            .tasks
            .find_by_id(task_id, &sys_author)
            .await
            .ok()
            .flatten()
            .and_then(|t| t.owner_user_id);

        let reply = OutboundReply {
            text: truncate_text(text, provider.max_message_length()),
            task_url: None,
            footer,
            author_user_id,
            is_error: false,
            files,
        };

        // Persist a visible stub up front so the row shows in the task timeline
        // and we have a uuid to tag with the outbound `external_event_id`.
        let stub_uuid = self
            .persist_outbound_stub_for_binding(task_id, &reply.text, &binding, adapter_label)
            .await;

        let external_id = match provider
            .send_reply_for_binding(&binding, &connection, rule.as_ref(), &reply)
            .await
        {
            Ok(id) => id,
            Err(e) => {
                record_outbound(
                    adapter_label,
                    "error",
                    "send_failed",
                    binding_loop_id,
                    &binding.connection_id,
                );
                return Ok((
                    0,
                    vec![format!(
                        "Failed to send reply via {}: {e}",
                        binding.adapter_kind
                    )],
                ));
            }
        };

        if let Some(ref ext_id) = external_id {
            self.recently_sent.note(task_id, ext_id);
            if let Some(ref msg_id) = stub_uuid {
                if let Err(e) = self
                    .messages
                    .set_external_source_metadata(
                        msg_id,
                        ext_id,
                        adapter_label,
                        Some(binding.subject_key.as_str()),
                    )
                    .await
                {
                    warn!(task_id = %task_id, error = %e, "Failed to tag outbound stub with external_event_id");
                }
            }
            if let Err(e) = binding_repo.touch_cursor(&binding.id, ext_id).await {
                warn!(
                    binding_id = %binding.id,
                    error = %e,
                    "Failed to advance binding cursor"
                );
            }
        }

        record_outbound(
            adapter_label,
            "success",
            "",
            binding_loop_id,
            &binding.connection_id,
        );
        Ok((1, vec![]))
    }

    /// The footer that GitHub/Linear comments end with — a link back to the
    /// moatless task. Other providers don't carry the footer, so this returns
    /// `None` for them (and when the task can't be resolved).
    pub(crate) async fn maybe_task_comment_footer(
        &self,
        adapter_kind: AdapterKind,
        task_id: &str,
    ) -> Option<String> {
        match adapter_kind {
            AdapterKind::GithubPr | AdapterKind::Linear => {
                crate::bindings::footer::task_comment_footer(
                    self.tasks.as_ref(),
                    &self.auth_config.frontend_url,
                    task_id,
                )
                .await
            }
            _ => None,
        }
    }

    /// Persist a visible assistant row for a binding-based outbound reply.
    pub(crate) async fn persist_outbound_stub_for_binding(
        &self,
        task_id: &str,
        text: &str,
        binding: &TaskBinding,
        adapter_label: &str,
    ) -> Option<String> {
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let uuid = uuid::Uuid::new_v4().to_string();

        let turn_number = match self.task_turns.latest_turn_number(task_id).await {
            Ok(t) => t,
            Err(e) => {
                warn!(task_id, error = %e, "Failed to read turn number for outbound stub");
                None
            }
        };

        let mut msg = TaskMessage::new_user_message(
            task_id.to_string(),
            uuid.clone(),
            text.to_string(),
            now.clone(),
        );
        msg.message_type = MessageType::System;
        msg.message_role = MessageRole::Assistant;
        msg.user_type = "assistant".to_string();
        msg.turn_number = turn_number;
        msg.external_adapter = Some(adapter_label.to_string());
        msg.external_source_name = Some(binding.subject_key.clone());

        match self
            .messages
            .append(&msg, &[], &ConversationWrite::default())
            .await
        {
            Ok(_) => Some(uuid),
            Err(e) => {
                warn!(task_id, error = %e, "Failed to persist outbound stub message");
                None
            }
        }
    }

    /// Resolve file IDs to [`OutboundFile`] structs by loading metadata + data
    /// from S3. Delegates to the shared [`resolve_files_with`] body so callers
    /// that hold the ports directly and this method never drift.
    pub(crate) async fn resolve_files(&self, file_ids: &[String]) -> Vec<OutboundFile> {
        resolve_files_with(&self.s3, &self.s3_config, self.files.as_ref(), file_ids).await
    }
}

/// Truncate text to a max byte length, appending "..." if truncated.
///
/// The cut is pulled back to the nearest UTF-8 char boundary so multibyte
/// input never panics (see PR #621).
pub(crate) fn truncate_text(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let cutoff = max.saturating_sub(3);
    let safe = text
        .char_indices()
        .map(|(i, _)| i)
        .take_while(|&i| i <= cutoff)
        .last()
        .unwrap_or(0);
    format!("{}...", &text[..safe])
}

// ---------------------------------------------------------------------------
// File attachment helpers
// ---------------------------------------------------------------------------

/// The file-resolution body, over the exact ports it reads.
/// [`OutboundSender::resolve_files`] and `adapters/handlers.rs` both forward
/// here, each passing the S3 and file ports it already holds.
pub(crate) async fn resolve_files_with(
    s3: &Option<S3Storage>,
    s3_config: &S3Config,
    files: &dyn FileDao,
    file_ids: &[String],
) -> Vec<OutboundFile> {
    if file_ids.is_empty() {
        return vec![];
    }

    let Some(s3_client) = s3.as_ref() else {
        warn!("S3 not configured, cannot resolve file attachments");
        return vec![];
    };
    let Some(bucket) = s3_config.bucket_name.as_ref() else {
        warn!("S3 bucket not configured, cannot resolve file attachments");
        return vec![];
    };

    let mut resolved = Vec::with_capacity(file_ids.len());
    for file_id in file_ids {
        let meta = match files.find_by_id(file_id).await {
            Ok(Some(m)) => m,
            Ok(None) => {
                warn!(file_id = %file_id, "File metadata not found for outbound attachment");
                continue;
            }
            Err(e) => {
                warn!(file_id = %file_id, error = %e, "Failed to load file metadata");
                continue;
            }
        };

        let s3_key = format!("{}/{}", s3_config.key_prefix, file_id);
        match crate::file_storage::s3::get_object(&s3_client.client, bucket, &s3_key).await {
            Ok((data, _content_type)) => {
                resolved.push(OutboundFile {
                    filename: meta.filename,
                    mime_type: meta.mime_type,
                    data,
                });
            }
            Err(e) => {
                warn!(file_id = %file_id, error = %e, "Failed to download file from S3");
            }
        }
    }

    resolved
}

#[cfg(test)]
mod tests {
    use super::truncate_text;

    #[test]
    fn truncate_under_max_returns_input() {
        assert_eq!(truncate_text("hello", 10), "hello");
    }

    #[test]
    fn truncate_over_max_appends_ellipsis() {
        assert_eq!(truncate_text("abcdefghij", 8), "abcde...");
    }

    #[test]
    fn truncate_respects_utf8_char_boundaries() {
        // Reproduces PR #621 panic: byte 197 fell inside '→' (3 bytes).
        let mut s = String::new();
        for _ in 0..195 {
            s.push('a');
        }
        s.push_str("→ main");
        let out = truncate_text(&s, 200);
        assert!(out.is_char_boundary(out.len() - 3));
        assert!(out.ends_with("..."));
    }

    #[test]
    fn truncate_at_exact_multibyte_boundary() {
        let s = "aaaa→bbbb";
        let out = truncate_text(s, 7);
        assert!(out.ends_with("..."));
        assert!(out.len() <= 7);
    }
}
