//! Everything the Slack Adapter sends: thread replies, direct posts, DMs, and
//! the first message of a new thread.

use std::fmt::Write as _;

use tracing::warn;

use moatless_adapter_model::binding::BindingContext;
use moatless_adapter_model::capabilities::{
    AdapterBindingReply, AdapterDirectDestination, AdapterDirectMessage, AdapterMessageReply,
};
use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
use moatless_adapter_model::outbound::{DestinationPostResult, OutboundReply, TaskProvenance};
use moatless_types::adapter::AdapterConnection;
use moatless_types::adapter::TaskBinding;
use moatless_types::adapter::{OutboundPresentation, SlackBlock, SlackText};
use moatless_types::loops::LoopContext;
use moatless_types::loops::SubscriptionMatch;

use super::adapter::SlackAdapter;
use super::api::{PostMessage, SlackApi};
use super::config::SlackAdapterConfig;
use super::{subject, text};

/// Send each attachment's bytes to Slack before the reply is posted, so a file
/// Slack refuses fails the reply instead of going missing after the text.
///
/// Returns the `(slack file id, title)` pairs [`share_files`] takes.
async fn stage_files(
    api: &SlackApi,
    message: &OutboundReply,
) -> Result<Vec<(String, String)>, AdapterError> {
    let mut staged = Vec::with_capacity(message.files.len());
    for file in &message.files {
        let id = api.stage_file(&file.filename, file.data.clone()).await?;
        staged.push((id, file.filename.clone()));
    }
    Ok(staged)
}

/// Share what [`stage_files`] staged into `thread_ts`, best-effort: the text
/// has already landed, so failing here would make the caller resend it.
async fn share_files(
    api: &SlackApi,
    channel: &str,
    thread_ts: Option<&str>,
    staged: &[(String, String)],
) {
    if staged.is_empty() {
        return;
    }
    if let Err(e) = api.share_files(channel, thread_ts, staged).await {
        warn!(
            files = staged.len(),
            error = %e,
            "Failed to share file attachments into the Slack thread"
        );
    }
}

/// The root of the thread `ts` sits in, since Slack asks callers not to thread
/// under a reply. Falls back to `ts`.
async fn thread_root(api: &SlackApi, channel: &str, ts: &str) -> String {
    match api.get_thread_replies(channel, ts, 1, None).await {
        Ok(messages) => messages
            .into_iter()
            .next()
            .and_then(|m| m.ts)
            .unwrap_or_else(|| ts.to_string()),
        Err(e) => {
            warn!(error = %e, "Slack thread root lookup failed; replying under the message");
            ts.to_string()
        }
    }
}

/// The text to post: the reply, its provenance footer, and every `<@…>`
/// resolved to something Slack can address.
///
/// A linked login becomes that account's mention; a bare Slack id is left alone
/// (reachable even if unknown to Moatless); anything else stays plain text and
/// pings nobody.
async fn outbound_text(
    adapter: &SlackAdapter,
    message: &OutboundReply,
    provenance: Option<&TaskProvenance>,
) -> String {
    let text = text::with_task_link(&message.text, provenance);
    let Some(users) = adapter.users() else {
        return text;
    };
    moatless_adapter_model::mentions::render_mentions(
        users,
        super::delivery::ADAPTER,
        &text,
        super::auth::is_slack_user_id,
        |id| format!("<@{id}>"),
    )
    .await
}

/// The provenance to render, honouring the Loop's `task_link` setting.
///
/// The loops band assembles provenance for every reply; whether it's shown is
/// this adapter's call, since the setting lives in its own config.
fn wanted_provenance<'a>(
    message: &'a OutboundReply,
    config: &LoopContext,
) -> Option<&'a TaskProvenance> {
    message
        .provenance
        .as_ref()
        .filter(|_| SlackAdapterConfig::from_config(config).task_link)
}

/// The blocks to post for this reply, or `None` when it carries no Slack
/// presentation and the text path applies unchanged. Mentions resolve here the
/// way [`outbound_text`] resolves them in the body.
///
/// `provenance` is an argument, not read off `message`, so the decision sits
/// where [`text::with_task_link`] already makes it.
async fn reply_blocks(
    adapter: &SlackAdapter,
    message: &OutboundReply,
    provenance: Option<&TaskProvenance>,
) -> Option<Vec<SlackBlock>> {
    let OutboundPresentation::Slack { blocks } = message.presentation.as_ref()?;
    let mut blocks = blocks.clone();
    if let Some(users) = adapter.users() {
        blocks = super::presentation::render_block_mentions(users, blocks).await;
    }
    if let Some(p) = provenance {
        // The mrkdwn footer in `text` is not displayed once blocks are present,
        // so it is rendered again as the block Slack greys out.
        let mut line = format!("<{}|{}>", p.url, p.label());
        for detail in p.details() {
            let _ = write!(line, " · {detail}");
        }
        blocks.push(SlackBlock::Context {
            elements: vec![SlackText::Mrkdwn { text: line }],
        });
    }
    Some(blocks)
}

#[async_trait::async_trait]
impl AdapterBindingReply for SlackAdapter {
    async fn send_reply(
        &self,
        thread: &BindingContext,
        config: &LoopContext,
        message: &OutboundReply,
    ) -> Result<Option<String>, AdapterError> {
        let app = self
            .app(moatless_adapter_model::apps::DEFAULT_APP_KEY)
            .await?;
        let coordinates = subject::thread_coordinates(&thread.destination_config);
        let channel = coordinates.channel.ok_or_else(|| {
            SharedAdapterError::Internal("No channel in thread properties".into())
        })?;
        // A room is the whole channel, so its replies post top-level.
        let thread_ts = match coordinates.thread_ts {
            None if thread.subject_kind == subject::room_subject_kind() => None,
            ts => Some(ts.ok_or_else(|| {
                SharedAdapterError::Internal("No thread_ts in thread properties".into())
            })?),
        };

        let provenance = wanted_provenance(message, config);
        let text = outbound_text(self, message, provenance).await;
        let blocks = reply_blocks(self, message, provenance).await;

        let staged = stage_files(&app.api, message).await?;
        let ts = app
            .api
            .post_message_with(PostMessage {
                channel,
                text: &text,
                thread_ts,
                blocks: blocks.as_deref(),
            })
            .await?;

        share_files(&app.api, channel, thread_ts, &staged).await;

        Ok(ts.map(|t| super::external_id::message_ts(&t)))
    }

    async fn send_reply_for_binding(
        &self,
        binding: &TaskBinding,
        connection: &AdapterConnection,
        rule: Option<&SubscriptionMatch>,
        message: &OutboundReply,
    ) -> Result<Option<String>, AdapterError> {
        self.reply_on_binding(binding, connection, rule, message, None)
            .await
    }
}

impl SlackAdapter {
    /// Post `message` on `binding`. In a Task Room, `under` is the `ts` the
    /// post threads beneath; without one it posts top-level.
    async fn reply_on_binding(
        &self,
        binding: &TaskBinding,
        _connection: &AdapterConnection,
        rule: Option<&SubscriptionMatch>,
        message: &OutboundReply,
        under: Option<&str>,
    ) -> Result<Option<String>, AdapterError> {
        let app_key = moatless_adapter_model::apps::app_key_from_transport_config(
            &_connection.transport_config,
        );
        let app = self.app(app_key).await?;
        let thread = moatless_adapter_model::binding::binding_context(binding);
        let config = rule.map_or_else(
            || LoopContext {
                id: binding.loop_id.clone().unwrap_or_default(),
                adapter_kind: "slack".to_string(),
                source_id: subject::thread_coordinates(&binding.destination_config)
                    .channel
                    .unwrap_or_default()
                    .to_string(),
                source_name: None,
                workspace_id: String::new(),
                routing_mode: moatless_types::loops::RoutingMode::BySubject,
                prompt_template: None,
                active: true,
                allow_direct_send: false,
                adapter_config: None,
                ongoing_task_id: None,
                scope: moatless_types::access_control::Scope::Global,
                task_name: None,
                tag_ids: vec![],
                memory_preload: Vec::new(),
                skills: Vec::new(),
                run_as_user_id: None,
                agent_profile: moatless_types::agent::AgentProfile::ClaudeCode,
                model: None,
                close_after_settled_days: None,
                settle_after_hours: None,
                created_by: "system".to_string(),
                created_at: binding.created_at,
                updated_at: binding.last_event_at,
                node_selector: None,
                stop_when_done: false,
                room: None,
            },
            moatless_adapter_model::loops::subscription_context,
        );
        let coordinates = subject::thread_coordinates(&thread.destination_config);
        let channel = coordinates.channel.ok_or_else(|| {
            SharedAdapterError::Internal("No channel in thread properties".into())
        })?;
        let room_thread = match under {
            Some(ts) if binding.subject_kind == subject::room_subject_kind() => {
                Some(thread_root(&app.api, channel, ts).await)
            }
            _ => None,
        };
        let thread_ts = match coordinates.thread_ts {
            None if binding.subject_kind == subject::room_subject_kind() => room_thread.as_deref(),
            ts => Some(ts.ok_or_else(|| {
                SharedAdapterError::Internal("No thread_ts in thread properties".into())
            })?),
        };

        let provenance = wanted_provenance(message, &config);
        let text = outbound_text(self, message, provenance).await;
        let blocks = reply_blocks(self, message, provenance).await;
        let staged = stage_files(&app.api, message).await?;
        let ts = app
            .api
            .post_message_with(PostMessage {
                channel,
                text: &text,
                thread_ts,
                blocks: blocks.as_deref(),
            })
            .await?;
        share_files(&app.api, channel, thread_ts, &staged).await;

        Ok(ts.map(|t| super::external_id::message_ts(&t)))
    }
}

#[async_trait::async_trait]
impl AdapterMessageReply for SlackAdapter {
    async fn reply_to_message(
        &self,
        binding: &TaskBinding,
        connection: &AdapterConnection,
        rule: Option<&SubscriptionMatch>,
        target_external_id: &str,
        message: &OutboundReply,
    ) -> Result<Option<String>, AdapterError> {
        let under = super::external_id::ts_from_message_id(target_external_id);
        self.reply_on_binding(binding, connection, rule, message, under)
            .await
    }
}

#[async_trait::async_trait]
impl AdapterDirectDestination for SlackAdapter {
    async fn post_to_destination(
        &self,
        destination: &str,
        message: &OutboundReply,
    ) -> Result<DestinationPostResult, AdapterError> {
        let app = self
            .app(moatless_adapter_model::apps::DEFAULT_APP_KEY)
            .await?;
        let workspace_id = self.ensure_workspace_id().await?;
        let (channel, thread_ts) = subject::parse_destination(destination)?;

        // No `LoopContext` on this path, so the `task_link` toggle is not
        // readable; the caller decides by setting `provenance` or not.
        let provenance = message.provenance.as_ref();
        let text = outbound_text(self, message, provenance).await;
        let blocks = reply_blocks(self, message, provenance).await;
        let staged = stage_files(&app.api, message).await?;
        let ts = app
            .api
            .post_message_with(PostMessage {
                channel: &channel,
                text: &text,
                thread_ts: thread_ts.as_deref(),
                blocks: blocks.as_deref(),
            })
            .await?
            .ok_or_else(|| {
                SharedAdapterError::Internal("Slack did not return message ts".into())
            })?;

        // Files share into the same thread (or under the new post when there's no thread).
        let upload_thread_ts = thread_ts.clone().unwrap_or_else(|| ts.clone());
        share_files(&app.api, &channel, Some(&upload_thread_ts), &staged).await;

        // For top-level posts the binding's `thread_ts` IS the new `ts`.
        // For thread replies the binding's `thread_ts` is the ORIGINAL thread anchor
        // (so future replies thread under the same parent, not under our new message).
        let binding_thread_ts = thread_ts.unwrap_or_else(|| ts.clone());

        let (subject_key, destination_config) =
            subject::direct_destination(&workspace_id, &channel, &binding_thread_ts, &ts);
        Ok(DestinationPostResult {
            message_external_id: ts,
            subject_kind: subject::subject_kind(),
            subject_key,
            destination_config,
        })
    }
}

#[async_trait::async_trait]
impl AdapterDirectMessage for SlackAdapter {
    /// Slack has to ask its API for the DM channel that pairs the bot with the
    /// user before anything can be posted there.
    async fn dm_destination(&self, provider_user_id: &str) -> Result<Option<String>, AdapterError> {
        let app = self
            .app(moatless_adapter_model::apps::DEFAULT_APP_KEY)
            .await?;
        let dm_channel = app.api.open_dm(provider_user_id).await?;
        Ok(Some(format!("dm:{dm_channel}")))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, OnceLock};

    use moatless_adapter_model::binding::BindingContext;
    use moatless_adapter_model::capabilities::{
        AdapterBindingReply, AdapterDirectDestination, AdapterDirectMessage, AdapterMessageReply,
    };
    use moatless_adapter_model::outbound::{
        OutboundFile, OutboundReply, TaskOrigin, TaskProvenance,
    };
    use moatless_adapter_model::test_support::ProviderStub;
    use moatless_types::access_control::Scope;
    use moatless_types::adapter::{
        OutboundPresentation, SlackBlock, SlackPlainText, SlackRichTextElement,
        SlackRichTextSection, SlackTableCell, SlackText,
    };
    use moatless_types::loops::{LoopContext, RoutingMode};

    use crate::adapter::SlackAdapter;
    use crate::api::SlackApi;

    /// A stub that answers every send endpoint the outbound paths use.
    async fn slack_stub() -> ProviderStub {
        ProviderStub::start(|path| match path {
            "/chat.postMessage" => serde_json::json!({"ok": true, "ts": "1700000000.000900"}),
            "/auth.test" => serde_json::json!({
                "ok": true, "user_id": "U0BOT", "team_id": "T1",
            }),
            "/conversations.open" => {
                serde_json::json!({"ok": true, "channel": {"id": "D123"}})
            }
            "/files.getUploadURLExternal" => serde_json::json!({
                "ok": true, "upload_url": "http://127.0.0.1:1/upload", "file_id": "F1",
            }),
            "/files.completeUploadExternal" => serde_json::json!({"ok": true}),
            "/conversations.replies" => serde_json::json!({
                "ok": true, "messages": [{"ts": "1700000000.000100"}],
            }),
            other => serde_json::json!({"ok": false, "error": format!("unexpected {other}")}),
        })
        .await
    }

    fn adapter(stub: &ProviderStub) -> SlackAdapter {
        SlackAdapter::with_api(
            SlackApi::with_base_url("xoxb-test", stub.base_url()),
            "secret",
            None,
        )
    }

    fn reply(text: &str, task_url: Option<&str>) -> OutboundReply {
        OutboundReply {
            text: text.to_string(),
            provenance: task_url.map(|u| TaskProvenance {
                url: u.to_string(),
                task_name: None,
                workspace_name: None,
                origin: TaskOrigin::Unknown,
            }),
            footer: None,
            author_user_id: None,
            is_error: false,
            files: vec![],
            presentation: None,
        }
    }

    fn binding_context(destination_config: serde_json::Value) -> BindingContext {
        let now = chrono::Utc::now();
        BindingContext {
            binding_id: "tb_1".into(),
            loop_id: None,
            task_id: "task-1".into(),
            subject_kind: "slack_thread".into(),
            subject_key: "T1:C1:1700000000.000100".into(),
            destination_config,
            metadata: serde_json::json!({}),
            last_external_event_id: None,
            last_message_at: now,
            created_at: now,
        }
    }

    fn loop_context(source_id: &str) -> LoopContext {
        let now = chrono::Utc::now();
        LoopContext {
            id: "tt_1".into(),
            adapter_kind: "slack".to_string(),
            source_id: source_id.into(),
            source_name: None,
            workspace_id: "ws-1".into(),
            routing_mode: RoutingMode::BySubject,
            prompt_template: None,
            active: true,
            allow_direct_send: true,
            adapter_config: None,
            ongoing_task_id: None,
            scope: Scope::Global,
            task_name: None,
            tag_ids: vec![],
            memory_preload: Vec::new(),
            skills: Vec::new(),
            run_as_user_id: None,
            agent_profile: moatless_types::agent::AgentProfile::ClaudeCode,
            model: None,
            close_after_settled_days: None,
            settle_after_hours: None,
            created_by: "user-1".into(),
            created_at: now,
            updated_at: now,
            node_selector: None,
            stop_when_done: false,
            room: None,
        }
    }

    fn body(request: &moatless_adapter_model::test_support::RecordedRequest) -> serde_json::Value {
        serde_json::from_str(&request.body).expect("stub recorded a JSON body")
    }

    /// The budget: one `chat.postMessage`, nothing else. The reply threads
    /// under the binding's anchor and carries the task link.
    #[tokio::test]
    async fn a_thread_reply_costs_one_post_and_renders_the_task_link() {
        let stub = slack_stub().await;
        let binding = binding_context(serde_json::json!({
            "channel": "C1",
            "thread_ts": "1700000000.000100",
        }));

        let external_id = adapter(&stub)
            .send_reply(
                &binding,
                &loop_context("C1"),
                &reply("all done", Some("https://example.test/t/1")),
            )
            .await
            .expect("the stub accepts the post");

        assert_eq!(
            external_id.as_deref(),
            Some("slack:ts:1700000000.000900"),
            "the returned id is namespaced"
        );
        assert_eq!(stub.paths(), vec!["/chat.postMessage".to_string()]);
        let posted = body(&stub.requests()[0]);
        assert_eq!(posted["channel"], "C1");
        assert_eq!(posted["thread_ts"], "1700000000.000100");
        assert_eq!(
            posted["text"],
            "all done\n<https://example.test/t/1|View task>"
        );
    }

    #[tokio::test]
    async fn a_reply_to_a_binding_without_coordinates_is_refused_before_any_call() {
        let stub = slack_stub().await;
        let no_channel = binding_context(serde_json::json!({"thread_ts": "1700000000.000100"}));
        let no_thread = binding_context(serde_json::json!({"channel": "C1"}));
        let adapter = adapter(&stub);

        assert_eq!(
            adapter
                .send_reply(&no_channel, &loop_context("C1"), &reply("hi", None))
                .await
                .unwrap_err()
                .to_string(),
            "Internal error: No channel in thread properties"
        );
        assert_eq!(
            adapter
                .send_reply(&no_thread, &loop_context("C1"), &reply("hi", None))
                .await
                .unwrap_err()
                .to_string(),
            "Internal error: No thread_ts in thread properties"
        );
        assert!(stub.paths().is_empty());
    }

    fn with_files(text: &str, names: &[&str]) -> OutboundReply {
        let mut message = reply(text, None);
        message.files = names
            .iter()
            .map(|name| OutboundFile {
                filename: (*name).into(),
                mime_type: "text/plain".into(),
                data: b"contents".to_vec(),
            })
            .collect();
        message
    }

    fn thread_binding() -> BindingContext {
        binding_context(serde_json::json!({
            "channel": "C1",
            "thread_ts": "1700000000.000100",
        }))
    }

    /// Each file is staged before the text is posted, and every staged file is
    /// shared into the reply's thread in one call after it.
    #[tokio::test]
    async fn attachments_are_staged_before_the_post_and_shared_after_it_in_one_call() {
        let base = Arc::new(OnceLock::<String>::new());
        let upload_base = base.clone();
        let stub = ProviderStub::start(move |path| match path {
            "/chat.postMessage" => serde_json::json!({"ok": true, "ts": "1700000000.000900"}),
            "/files.getUploadURLExternal" => serde_json::json!({
                "ok": true,
                "upload_url": format!("{}/upload", upload_base.get().expect("base is set")),
                "file_id": "F1",
            }),
            _ => serde_json::json!({"ok": true}),
        })
        .await;
        base.set(stub.base_url().to_string()).expect("set once");

        adapter(&stub)
            .send_reply(
                &thread_binding(),
                &loop_context("C1"),
                &with_files("with files", &["a.txt", "b.txt"]),
            )
            .await
            .expect("the stub accepts every call");

        assert_eq!(
            stub.paths(),
            [
                "/files.getUploadURLExternal",
                "/upload",
                "/files.getUploadURLExternal",
                "/upload",
                "/chat.postMessage",
                "/files.completeUploadExternal",
            ]
            .map(String::from)
            .to_vec(),
        );
        let shared = body(stub.requests().last().expect("a share call"));
        assert_eq!(shared["channel_id"], "C1");
        assert_eq!(shared["thread_ts"], "1700000000.000100");
        assert_eq!(
            shared["files"],
            serde_json::json!([
                {"id": "F1", "title": "a.txt"},
                {"id": "F1", "title": "b.txt"},
            ])
        );
    }

    /// A file Slack will not take fails the reply before its text is posted,
    /// so the caller sees the error and a resend does not post the text twice.
    #[tokio::test]
    async fn a_file_that_cannot_be_staged_fails_the_reply_before_the_post() {
        let stub = slack_stub().await;

        let error = adapter(&stub)
            .send_reply(
                &thread_binding(),
                &loop_context("C1"),
                &with_files("with a file", &["log.txt"]),
            )
            .await
            .expect_err("the presigned URL is unreachable");

        assert!(
            error.to_string().contains("File upload to presigned URL"),
            "{error}"
        );
        assert_eq!(
            stub.paths(),
            vec!["/files.getUploadURLExternal".to_string()]
        );
    }

    /// The budget: one `auth.test` (memoized) and one `chat.postMessage`. A
    /// top-level direct send binds under its own `ts` and renders the task
    /// link, because a channel post is often the only message a reader sees.
    fn room_binding() -> moatless_types::adapter::TaskBinding {
        let now = chrono::Utc::now();
        moatless_types::adapter::TaskBinding {
            id: "tb_room".into(),
            task_id: "task-1".into(),
            adapter_kind: moatless_types::adapter::AdapterKind::Slack,
            subject_kind: "slack_channel".into(),
            subject_key: "T1:C1".into(),
            connection_id: "pc_slack".into(),
            loop_id: None,
            destination_config: serde_json::json!({"channel": "C1", "team_id": "T1"}),
            metadata: serde_json::json!({}),
            last_external_event_id: None,
            origin: "room".into(),
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

    fn connection() -> moatless_types::adapter::AdapterConnection {
        let now = chrono::Utc::now();
        moatless_types::adapter::AdapterConnection {
            id: "pc_slack".into(),
            adapter_kind: moatless_types::adapter::AdapterKind::Slack,
            connection_kind: "workspace".into(),
            scope_value: "T1".into(),
            credential_ref: None,
            webhook_secret: None,
            transport_config: serde_json::json!({}),
            created_by: "alice".into(),
            scope: Scope::Global,
            owner_user_id: None,
            created_at: now,
            updated_at: now,
        }
    }

    #[tokio::test]
    async fn a_room_reply_threads_under_the_root_of_the_message_it_answers() {
        let stub = slack_stub().await;

        adapter(&stub)
            .reply_to_message(
                &room_binding(),
                &connection(),
                None,
                "slack:ts:1700000000.000500",
                &reply("on it", None),
            )
            .await
            .expect("the stub accepts the post");

        let requests = stub.requests();
        assert_eq!(requests[0].path, "/conversations.replies");
        assert!(requests[0].query.contains("ts=1700000000.000500"));
        assert_eq!(
            body(&requests[1])["thread_ts"],
            "1700000000.000100",
            "the reply joins the thread, not a thread under a reply"
        );
    }

    #[tokio::test]
    async fn a_room_reply_whose_thread_cannot_be_read_threads_under_the_message() {
        let stub = ProviderStub::start(|path| match path {
            "/chat.postMessage" => serde_json::json!({"ok": true, "ts": "1700000000.000900"}),
            _ => serde_json::json!({"ok": false, "error": "channel_not_found"}),
        })
        .await;

        adapter(&stub)
            .reply_to_message(
                &room_binding(),
                &connection(),
                None,
                "slack:ts:1700000000.000500",
                &reply("on it", None),
            )
            .await
            .expect("the stub accepts the post");

        assert_eq!(body(&stub.requests()[1])["thread_ts"], "1700000000.000500");
    }

    #[tokio::test]
    async fn a_room_post_with_no_message_to_answer_is_top_level() {
        let stub = slack_stub().await;

        adapter(&stub)
            .send_reply_for_binding(&room_binding(), &connection(), None, &reply("done", None))
            .await
            .expect("the stub accepts the post");

        assert_eq!(stub.paths(), vec!["/chat.postMessage".to_string()]);
        assert!(body(&stub.requests()[0]).get("thread_ts").is_none());
    }

    #[tokio::test]
    async fn a_direct_send_costs_one_auth_test_and_one_post() {
        let stub = slack_stub().await;
        let adapter = adapter(&stub);

        let result = adapter
            .post_to_destination(
                "channel:C1",
                &reply("hello", Some("https://example.test/t/1")),
            )
            .await
            .expect("the stub accepts the post");

        assert_eq!(result.message_external_id, "1700000000.000900");
        assert_eq!(result.subject_kind, "slack_thread");
        assert_eq!(result.subject_key, "T1:C1:1700000000.000900");
        assert_eq!(
            result.destination_config,
            serde_json::json!({
                "channel": "C1",
                "ts": "1700000000.000900",
                "thread_ts": "1700000000.000900",
            })
        );
        assert_eq!(
            stub.paths(),
            vec!["/auth.test".to_string(), "/chat.postMessage".to_string()]
        );
        assert_eq!(
            body(&stub.requests()[1])["text"],
            "hello\n<https://example.test/t/1|View task>",
            "a direct send renders the task link, like a reply does"
        );

        adapter
            .post_to_destination("channel:C1", &reply("again", None))
            .await
            .expect("the stub accepts the post");
        assert_eq!(
            stub.paths(),
            vec![
                "/auth.test".to_string(),
                "/chat.postMessage".to_string(),
                "/chat.postMessage".to_string(),
            ],
            "the workspace id is memoized after the first send"
        );
    }

    #[tokio::test]
    async fn a_direct_send_into_a_thread_binds_under_the_parent() {
        let stub = slack_stub().await;

        let result = adapter(&stub)
            .post_to_destination("thread:C1/1700000000.000100", &reply("hello", None))
            .await
            .expect("the stub accepts the post");

        assert_eq!(result.subject_key, "T1:C1:1700000000.000100");
        assert_eq!(result.destination_config["ts"], "1700000000.000900");
        assert_eq!(body(&stub.requests()[1])["thread_ts"], "1700000000.000100");
    }

    #[tokio::test]
    async fn a_dm_destination_costs_one_conversations_open() {
        let stub = slack_stub().await;

        assert_eq!(
            adapter(&stub)
                .dm_destination("U1")
                .await
                .expect("the stub opens the DM"),
            Some("dm:D123".to_string())
        );
        assert_eq!(stub.paths(), vec!["/conversations.open".to_string()]);
    }

    fn reply_with_blocks(
        text: &str,
        task_url: Option<&str>,
        blocks: Vec<SlackBlock>,
    ) -> OutboundReply {
        OutboundReply {
            presentation: Some(OutboundPresentation::Slack { blocks }),
            ..reply(text, task_url)
        }
    }

    /// The text path is load-bearing for every reply that sends no blocks, so
    /// it has to keep posting exactly the body it posted before.
    #[tokio::test]
    async fn a_reply_without_a_presentation_posts_no_blocks_key() {
        let stub = slack_stub().await;
        let binding = binding_context(serde_json::json!({
            "channel": "C1",
            "thread_ts": "1700000000.000100",
        }));

        adapter(&stub)
            .send_reply(&binding, &loop_context("C1"), &reply("all done", None))
            .await
            .expect("the stub accepts the post");

        let posted = body(&stub.requests()[0]);
        assert_eq!(posted.get("blocks"), None);
        assert_eq!(posted["text"], "all done");
    }

    /// This adapter has no directory, so the blocks carry no mention to resolve
    /// and go out as written. The task link moves into a block — the mrkdwn link
    /// in `text` is not displayed once blocks are present.
    #[tokio::test]
    async fn a_presentation_posts_its_blocks_and_appends_the_task_link_as_a_block() {
        let stub = slack_stub().await;
        let binding = binding_context(serde_json::json!({
            "channel": "C1",
            "thread_ts": "1700000000.000100",
        }));
        let blocks = vec![SlackBlock::Header {
            text: SlackPlainText::PlainText {
                text: "Prod release".into(),
                emoji: None,
            },
        }];

        adapter(&stub)
            .send_reply(
                &binding,
                &loop_context("C1"),
                &reply_with_blocks("7 apps", Some("https://example.test/t/1"), blocks),
            )
            .await
            .expect("the stub accepts the post");

        let posted = body(&stub.requests()[0]);
        assert_eq!(
            posted["blocks"],
            serde_json::json!([
                { "type": "header", "text": { "type": "plain_text", "text": "Prod release" } },
                {
                    "type": "context",
                    "elements": [
                        { "type": "mrkdwn", "text": "<https://example.test/t/1|View task>" }
                    ]
                }
            ])
        );
        // `text` still carries the whole message: Slack notifies on it and the
        // binding layer persists it as the task-timeline entry.
        assert_eq!(
            posted["text"],
            "7 apps\n<https://example.test/t/1|View task>"
        );
    }

    /// The footer is rendered again as a block, because Slack stops displaying
    /// `text` once blocks are present.
    #[tokio::test]
    async fn a_direct_send_appends_a_task_link_block() {
        let stub = slack_stub().await;

        adapter(&stub)
            .post_to_destination(
                "channel:C1",
                &reply_with_blocks(
                    "hi",
                    Some("https://example.test/t/1"),
                    vec![SlackBlock::Divider],
                ),
            )
            .await
            .expect("the stub accepts the post");

        let posted = body(
            stub.requests()
                .iter()
                .find(|r| r.path == "/chat.postMessage")
                .expect("a message was posted"),
        );
        assert_eq!(
            posted["blocks"],
            serde_json::json!([
                { "type": "divider" },
                { "type": "context", "elements": [
                    { "type": "mrkdwn", "text": "<https://example.test/t/1|View task>" }
                ]},
            ])
        );
    }

    /// A send with no task named — a DM, or `moat send` without `--task` —
    /// still posts exactly what it was given.
    #[tokio::test]
    async fn a_direct_send_without_provenance_renders_nothing_extra() {
        let stub = slack_stub().await;

        adapter(&stub)
            .post_to_destination(
                "channel:C1",
                &reply_with_blocks("hi", None, vec![SlackBlock::Divider]),
            )
            .await
            .expect("the stub accepts the post");

        let posted = body(
            stub.requests()
                .iter()
                .find(|r| r.path == "/chat.postMessage")
                .expect("a message was posted"),
        );
        assert_eq!(posted["text"], "hi");
        assert_eq!(posted["blocks"], serde_json::json!([{ "type": "divider" }]));
    }

    /// A block Slack dislikes costs the message its formatting, not its
    /// existence.
    #[tokio::test]
    async fn slack_rejecting_the_blocks_reposts_the_message_without_them() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        let seen = Arc::new(AtomicUsize::new(0));
        let counter = seen.clone();
        let stub = ProviderStub::start(move |path| match path {
            "/chat.postMessage" => {
                if counter.fetch_add(1, Ordering::SeqCst) == 0 {
                    serde_json::json!({"ok": false, "error": "invalid_blocks"})
                } else {
                    serde_json::json!({"ok": true, "ts": "1700000000.000900"})
                }
            }
            "/auth.test" => serde_json::json!({
                "ok": true, "user_id": "U0BOT", "team_id": "T1",
            }),
            other => serde_json::json!({"ok": false, "error": format!("unexpected {other}")}),
        })
        .await;

        let binding = binding_context(serde_json::json!({
            "channel": "C1",
            "thread_ts": "1700000000.000100",
        }));

        let external_id = adapter(&stub)
            .send_reply(
                &binding,
                &loop_context("C1"),
                &reply_with_blocks("all done", None, vec![SlackBlock::Divider]),
            )
            .await
            .expect("the retry lands");

        assert_eq!(external_id.as_deref(), Some("slack:ts:1700000000.000900"));

        let posts: Vec<_> = stub
            .requests()
            .into_iter()
            .filter(|r| r.path == "/chat.postMessage")
            .collect();
        assert_eq!(posts.len(), 2, "one rejected post, one retry");
        assert!(body(&posts[0]).get("blocks").is_some());
        assert_eq!(body(&posts[1]).get("blocks"), None);
        assert_eq!(body(&posts[1])["text"], "all done");
    }

    fn reply_with_provenance(text: &str, blocks: Vec<SlackBlock>) -> OutboundReply {
        OutboundReply {
            presentation: Some(OutboundPresentation::Slack { blocks }),
            provenance: Some(TaskProvenance {
                url: "https://example.test/t/1".into(),
                task_name: Some("prod-release — 7 apps".into()),
                workspace_name: Some("soap-app-gitops".into()),
                origin: TaskOrigin::Loop {
                    name: "prod-release".into(),
                },
            }),
            ..reply(text, None)
        }
    }

    fn loop_context_with(adapter_config: serde_json::Value) -> LoopContext {
        LoopContext {
            adapter_config: Some(adapter_config),
            ..loop_context("C1")
        }
    }

    /// The footer is repeated as a block because the mrkdwn one in `text` is
    /// not displayed once blocks are present.
    #[tokio::test]
    async fn provenance_renders_as_a_trailing_context_block() {
        let stub = slack_stub().await;
        let binding = binding_context(serde_json::json!({
            "channel": "C1",
            "thread_ts": "1700000000.000100",
        }));

        adapter(&stub)
            .send_reply(
                &binding,
                &loop_context("C1"),
                &reply_with_provenance("7 apps", vec![SlackBlock::Divider]),
            )
            .await
            .expect("the stub accepts the post");

        let posted = body(&stub.requests()[0]);
        assert_eq!(
            posted["blocks"],
            serde_json::json!([
                { "type": "divider" },
                {
                    "type": "context",
                    "elements": [{
                        "type": "mrkdwn",
                        "text": "<https://example.test/t/1|prod-release — 7 apps> · soap-app-gitops · loop: prod-release"
                    }]
                }
            ])
        );
        assert!(
            posted["text"]
                .as_str()
                .is_some_and(|t| t.contains("loop: prod-release")),
            "text carries the footer too, for the notification and the timeline"
        );
    }

    /// `task_link: false` on the Loop suppresses the footer on both paths.
    #[tokio::test]
    async fn the_loop_can_turn_the_footer_off() {
        let stub = slack_stub().await;
        let binding = binding_context(serde_json::json!({
            "channel": "C1",
            "thread_ts": "1700000000.000100",
        }));

        adapter(&stub)
            .send_reply(
                &binding,
                &loop_context_with(serde_json::json!({ "task_link": false })),
                &reply_with_provenance("7 apps", vec![SlackBlock::Divider]),
            )
            .await
            .expect("the stub accepts the post");

        let posted = body(&stub.requests()[0]);
        assert_eq!(
            posted["blocks"],
            serde_json::json!([{ "type": "divider" }]),
            "no context block"
        );
        assert_eq!(posted["text"], "7 apps", "and no footer in the text either");
    }

    /// Absent config means on: a Loop that says nothing still gets a way home.
    #[tokio::test]
    async fn the_footer_is_on_when_the_loop_says_nothing() {
        let stub = slack_stub().await;
        let binding = binding_context(serde_json::json!({
            "channel": "C1",
            "thread_ts": "1700000000.000100",
        }));

        adapter(&stub)
            .send_reply(
                &binding,
                &loop_context_with(serde_json::json!({ "trigger": "all_messages" })),
                &reply_with_provenance("7 apps", vec![SlackBlock::Divider]),
            )
            .await
            .expect("the stub accepts the post");

        let posted = body(&stub.requests()[0]);
        assert_eq!(posted["blocks"].as_array().map(Vec::len), Some(2));
    }

    /// A directory that links the login `aorwall` to `U0AAQ`, and nobody else.
    struct LinkedLogins;

    #[async_trait::async_trait]
    impl moatless_ports::identity::ProviderIdentityLink for LinkedLogins {
        async fn provider_id_for_login(
            &self,
            provider: &str,
            login: &str,
        ) -> Result<Option<String>, moatless_types::store::StoreError> {
            Ok((provider == "slack" && login == "aorwall").then(|| "U0AAQ".to_string()))
        }
    }

    /// All three mention outcomes, in the request Slack actually receives.
    #[tokio::test]
    async fn mentions_are_resolved_in_the_posted_text() {
        let stub = slack_stub().await;

        adapter(&stub)
            .with_users(std::sync::Arc::new(LinkedLogins))
            .post_to_destination(
                "channel:C1",
                &reply("<@aorwall> and <@martin> and <@U0NEVERSEEN>", None),
            )
            .await
            .expect("the stub accepts the post");

        let posted = body(
            stub.requests()
                .iter()
                .find(|r| r.path == "/chat.postMessage")
                .expect("a message was posted"),
        );
        assert_eq!(
            posted["text"],
            // A linked login pings; an unlinked one names without pinging; a
            // raw Slack id is passed through so anyone stays reachable.
            "<@U0AAQ> and @martin and <@U0NEVERSEEN>"
        );
    }

    /// Without a directory — an adapter built with no pool — a reply still
    /// posts, and a mention it cannot resolve is left exactly as written.
    #[tokio::test]
    async fn a_send_with_no_directory_posts_the_text_unchanged() {
        let stub = slack_stub().await;

        adapter(&stub)
            .post_to_destination("channel:C1", &reply("<@aorwall> take a look", None))
            .await
            .expect("the stub accepts the post");

        let posted = body(
            stub.requests()
                .iter()
                .find(|r| r.path == "/chat.postMessage")
                .expect("a message was posted"),
        );
        assert_eq!(posted["text"], "<@aorwall> take a look");
    }

    /// The wiring, on the request Slack receives: the presentation is resolved
    /// through the directory on the way out, and the footer is appended after,
    /// so the block the adapter writes itself is never walked for mentions.
    #[tokio::test]
    async fn a_presentations_mentions_are_resolved_before_the_footer_is_appended() {
        let stub = slack_stub().await;
        let blocks = vec![
            SlackBlock::Section {
                text: Some(SlackText::Mrkdwn {
                    text: "<@aorwall> owns it".into(),
                }),
                fields: None,
            },
            SlackBlock::Table {
                rows: vec![vec![SlackTableCell::RichText {
                    elements: vec![SlackRichTextSection::RichTextSection {
                        elements: vec![SlackRichTextElement::User {
                            user_id: "aorwall".into(),
                        }],
                    }],
                }]],
            },
        ];

        adapter(&stub)
            .with_users(std::sync::Arc::new(LinkedLogins))
            .post_to_destination(
                "channel:C1",
                &reply_with_blocks("owned", Some("https://example.test/t/1"), blocks),
            )
            .await
            .expect("the stub accepts the post");

        let posted = body(
            stub.requests()
                .iter()
                .find(|r| r.path == "/chat.postMessage")
                .expect("a message was posted"),
        );
        assert_eq!(
            posted["blocks"],
            serde_json::json!([
                { "type": "section", "text": { "type": "mrkdwn", "text": "<@U0AAQ> owns it" } },
                {
                    "type": "table",
                    "rows": [[{
                        "type": "rich_text",
                        "elements": [{
                            "type": "rich_text_section",
                            "elements": [{ "type": "user", "user_id": "U0AAQ" }],
                        }],
                    }]],
                },
                {
                    "type": "context",
                    "elements": [
                        { "type": "mrkdwn", "text": "<https://example.test/t/1|View task>" }
                    ]
                }
            ])
        );
    }
}
