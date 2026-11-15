//! Outbound adapter notifications — event-driven replies and explicit sends.

use std::sync::Arc;

use tracing::warn;

use moatless_adapter_model::outbound::{OutboundFile, OutboundReply, TaskOrigin, TaskProvenance};
use moatless_types::access_control::ScopeContext;
use moatless_types::adapter::TaskBinding;
use moatless_types::adapter::{AdapterKind, OutboundPresentation};
use moatless_types::auth::AuthConfig;
use moatless_types::loops::SubscriptionMatch;

use crate::bindings::metrics::record_outbound;
use crate::bindings::operations::Bindings;
use crate::bindings::own_events::OwnEvents;
use crate::loops::dao::LoopDao;
use moatless_adapter_model::connections::ConnectionRouting;
use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
use moatless_adapter_model::registry::Adapters;
use moatless_core::file_storage::Files;
use moatless_ports::identity::UserDisplay;
use moatless_ports::task::{
    ExternalEvent, TaskReader, TranscriptAppend, TranscriptEntry, TranscriptEntryKind, TurnCounter,
};
use moatless_workspace_ports::WorkspaceResolver;

/// Whether this thread has already carried a provenance footer.
///
/// The footer names the thread, not the message, so it renders once and the
/// lookups behind it are paid once; the binding is already loaded at every send
/// site, so asking costs nothing.
pub fn task_link_already_sent(binding: &TaskBinding) -> bool {
    binding
        .metadata
        .get("task_link_sent")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

/// Reject a presentation addressed to a different adapter than the send
/// resolves to. Dropping it silently would post the plain-text fallback and
/// report success, which reads as "Slack ignored my blocks".
pub fn checked_presentation(
    presentation: Option<OutboundPresentation>,
    adapter: AdapterKind,
) -> Result<Option<OutboundPresentation>, AdapterError> {
    match presentation {
        Some(p) if p.kind() != adapter => Err(SharedAdapterError::BadRequest(format!(
            "presentation kind `{}` does not match the destination adapter `{}`",
            p.kind().as_str(),
            adapter.as_str()
        ))
        .into()),
        other => Ok(other),
    }
}

/// Binding-based outbound delivery, owning the ports the send path reads rather
/// than the whole `AppState`. Held by `ExternalReplyService`, its only
/// consumer.
pub struct OutboundSender {
    task_bindings: Arc<Bindings>,
    adapter_connections: Arc<ConnectionRouting>,
    loops: Arc<LoopDao>,
    tasks: Arc<dyn TaskReader>,
    own_events: Arc<OwnEvents>,
    messages: Arc<dyn TranscriptAppend>,
    task_turns: Arc<dyn TurnCounter>,
    auth_config: Arc<AuthConfig>,
    files: Arc<Files>,
    workspaces: Arc<dyn WorkspaceResolver>,
    users: Arc<dyn UserDisplay>,
}

impl OutboundSender {
    #[allow(clippy::too_many_arguments)]
    // reason: a composition-root constructor takes exactly the ports its struct
    // holds; the alternative is threading `AppState` back in, which is the edge
    // this slice removes.
    pub(crate) fn new(
        task_bindings: Arc<Bindings>,
        adapter_connections: Arc<ConnectionRouting>,
        loops: Arc<LoopDao>,
        tasks: Arc<dyn TaskReader>,
        own_events: Arc<OwnEvents>,
        messages: Arc<dyn TranscriptAppend>,
        task_turns: Arc<dyn TurnCounter>,
        auth_config: Arc<AuthConfig>,
        files: Arc<Files>,
        workspaces: Arc<dyn WorkspaceResolver>,
        users: Arc<dyn UserDisplay>,
    ) -> Self {
        Self {
            task_bindings,
            adapter_connections,
            loops,
            tasks,
            own_events,
            messages,
            task_turns,
            auth_config,
            files,
            workspaces,
            users,
        }
    }

    /// Send a reply to the external target referenced by a task binding.
    ///
    /// Loads the binding, connection, and optional subscription loop, then
    /// calls `send_reply_for_binding`; persists a visible assistant stub up
    /// front, tags it with the returned external id, populates the
    /// `own_events` race window, and advances the binding cursor.
    pub(crate) async fn send_to_binding(
        &self,
        providers: &Adapters,
        task_id: &str,
        binding_id: &str,
        text: &str,
        file_ids: &[String],
        presentation: Option<OutboundPresentation>,
    ) -> Result<(u32, Vec<String>), AdapterError> {
        let binding_repo = self.task_bindings.clone();
        let binding = binding_repo.get(binding_id).await?.ok_or_else(|| {
            SharedAdapterError::ConfigNotFound(format!("No task binding {binding_id}"))
        })?;

        if binding.task_id != task_id {
            return Err(SharedAdapterError::BadRequest(format!(
                "Binding {binding_id} does not belong to task {task_id}"
            ))
            .into());
        }

        let connection = self
            .adapter_connections
            .by_id_trusted(&binding.connection_id)
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

        let provider =
            providers.outbound_for_binding_reply_by_name(binding.adapter_kind.as_str())?;
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
            .describe(task_id, &sys_author)
            .await
            .ok()
            .flatten()
            .and_then(|t| t.owner_user_id);

        let presentation = checked_presentation(presentation, provider.adapter_kind())?;
        let provenance = if task_link_already_sent(&binding) {
            None
        } else {
            self.maybe_task_provenance(provider.adapter_kind(), task_id, rule.as_ref())
                .await
        };
        let sent_provenance = provenance.is_some();
        let reply = OutboundReply {
            text: truncate_text(text, provider.max_message_length()),
            provenance,
            footer,
            author_user_id,
            is_error: false,
            files,
            presentation,
        };

        let external_id = match provider
            .send_reply_for_binding(&binding, &connection, rule.as_ref(), &reply)
            .await
        {
            Ok(id) => {
                if sent_provenance {
                    // Best effort: a failure re-renders the footer once, which
                    // beats failing a reply that already landed.
                    if let Err(e) = self.task_bindings.mark_task_link_sent(&binding.id).await {
                        warn!(binding_id = %binding.id, error = %e,
                              "Failed to record that the task link was sent");
                    }
                }
                id
            }
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

        self.persist_outbound_stub_for_binding(
            task_id,
            &reply.text,
            &binding,
            adapter_label,
            external_id.as_deref(),
        )
        .await;
        if let Some(ref ext_id) = external_id {
            self.own_events.note(task_id, ext_id);
            if let Err(e) = binding_repo.advance_cursor(&binding.id, ext_id).await {
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

    /// The provenance footer for adapters that render the task link themselves.
    ///
    /// `None` for Markdown-comment adapters (link is in `footer` instead) or
    /// when no frontend URL is configured; best-effort, so a failed lookup
    /// drops its own segment, not the reply, and `rule` names the firing Loop
    /// else falls back to who started the task.
    pub async fn maybe_task_provenance(
        &self,
        adapter_kind: AdapterKind,
        task_id: &str,
        rule: Option<&SubscriptionMatch>,
    ) -> Option<TaskProvenance> {
        if adapter_kind != AdapterKind::Slack {
            return None;
        }
        let base = self.auth_config.frontend_url.trim_end_matches('/');
        if base.is_empty() {
            return None;
        }
        let url = format!("{base}/tasks/{task_id}");

        #[expect(
            clippy::disallowed_methods,
            reason = "a formatting lookup on an already-authorised reply, with no caller identity"
        )]
        let sys = ScopeContext::system();

        let task = match self.tasks.describe(task_id, &sys).await {
            Ok(t) => t,
            Err(e) => {
                warn!(task_id, error = %e, "Failed to load task for provenance footer");
                None
            }
        };

        let mut workspace_name = None;
        if let Some(workspace_id) = task.as_ref().and_then(|t| t.workspace_id.as_deref()) {
            match self.workspaces.resolve(workspace_id).await {
                Ok(Some(w)) => workspace_name = Some(w.name),
                Ok(None) => {}
                Err(e) => {
                    warn!(task_id, error = %e, "Failed to load workspace for provenance footer");
                }
            }
        }

        // A Loop that fired the task names itself; otherwise the person who
        // started it does.
        let origin = match rule {
            Some(r) => TaskOrigin::Loop {
                name: r.name.clone(),
            },
            None => {
                self.started_by(task.as_ref().and_then(|t| t.created_by.as_deref()))
                    .await
            }
        };

        Some(TaskProvenance {
            url,
            task_name: task.and_then(|t| t.name),
            workspace_name,
            origin,
        })
    }

    /// Resolve a `created_by` user id to a login, best effort.
    async fn started_by(&self, created_by: Option<&str>) -> TaskOrigin {
        let Some(raw) = created_by else {
            return TaskOrigin::Unknown;
        };
        let Ok(uuid) = uuid::Uuid::parse_str(raw) else {
            return TaskOrigin::Unknown;
        };
        match self.users.name(&uuid).await {
            Ok(Some(login)) => TaskOrigin::StartedBy { login },
            Ok(None) => TaskOrigin::Unknown,
            Err(e) => {
                warn!(error = %e, "Failed to load user for provenance footer");
                TaskOrigin::Unknown
            }
        }
    }

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

    /// Record a binding-based outbound reply once sent, with the external
    /// event it became, which the echo check reads.
    pub(crate) async fn persist_outbound_stub_for_binding(
        &self,
        task_id: &str,
        text: &str,
        binding: &TaskBinding,
        adapter_label: &str,
        external_id: Option<&str>,
    ) {
        let turn_number = match self.task_turns.latest_turn_number(task_id).await {
            Ok(t) => t,
            Err(e) => {
                warn!(task_id, error = %e, "Failed to read turn number for outbound stub");
                None
            }
        };

        if let Err(e) = self
            .messages
            .append(&TranscriptEntry {
                task_id: task_id.to_string(),
                uuid: uuid::Uuid::new_v4().to_string(),
                text: text.to_string(),
                kind: TranscriptEntryKind::OutboundStub,
                turn_number,
                external: external_id.map(|external_id| ExternalEvent {
                    external_id: external_id.to_string(),
                    provider: adapter_label.to_string(),
                    source_name: Some(binding.subject_key.clone()),
                }),
            })
            .await
        {
            warn!(task_id, error = %e, "Failed to persist outbound stub message");
        }
    }

    /// The uploads a reply carries, as the adapter kernel names them.
    pub(crate) async fn resolve_files(&self, file_ids: &[String]) -> Vec<OutboundFile> {
        outbound_files(&self.files, file_ids).await
    }
}

/// Truncate text to a max byte length, appending "..." if truncated.
///
/// The cut is pulled back to the nearest UTF-8 char boundary so multibyte input
/// never panics (see PR #621).
pub fn truncate_text(text: &str, max: usize) -> String {
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

/// The uploads `file_ids` names, as [`OutboundFile`]s.
///
/// [`Files`] owns the lookup and the warn-and-skip policy; this only renames
/// the result into the adapter kernel's vocabulary, which `moatless-core` sits
/// below and cannot name.
pub async fn outbound_files(files: &Files, file_ids: &[String]) -> Vec<OutboundFile> {
    files
        .resolve_attachments(file_ids)
        .await
        .into_iter()
        .map(|file| OutboundFile {
            filename: file.filename,
            mime_type: file.mime_type,
            data: file.data,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{task_link_already_sent, truncate_text};
    use moatless_types::adapter::TaskBinding;

    fn binding(metadata: serde_json::Value) -> TaskBinding {
        let now = chrono::Utc::now();
        TaskBinding {
            id: "tb_1".into(),
            task_id: "task-1".into(),
            adapter_kind: moatless_types::adapter::AdapterKind::Slack,
            subject_kind: "thread".into(),
            subject_key: "subject-1".into(),
            connection_id: "conn-1".into(),
            loop_id: None,
            destination_config: serde_json::json!({}),
            metadata,
            last_external_event_id: None,
            origin: "inbound".into(),
            created_at: now,
            last_event_at: now,
            external_status: None,
            external_state_kind: None,
            external_status_raw: None,
            external_review_decision: None,
            external_mergeable_state: None,
            external_status_updated_at: None,
            active: true,
            deactivated_at: None,
            watching: true,
        }
    }

    /// The footer names the thread, so a fresh thread wants one and a thread
    /// that already carried one does not.
    #[test]
    fn the_footer_is_owed_once_per_thread() {
        assert!(!task_link_already_sent(&binding(serde_json::json!({}))));
        assert!(task_link_already_sent(&binding(
            serde_json::json!({"task_link_sent": true})
        )));
    }

    /// Unrelated metadata must not read as "already sent" — that would silently
    /// suppress the footer on every thread carrying any other key.
    #[test]
    fn unrelated_metadata_does_not_suppress_the_footer() {
        assert!(!task_link_already_sent(&binding(
            serde_json::json!({"something_else": true})
        )));
        assert!(!task_link_already_sent(&binding(
            serde_json::json!({"task_link_sent": "yes"})
        )));
    }

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
