//! Everything the Slack Adapter sends: thread replies, direct posts, DMs, and
//! the first message of a new thread.

use std::fmt::Write as _;

use tracing::warn;

use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
use moatless_adapter_model::models::{AdapterConnection, SubscriptionMatch, TaskBinding};
use moatless_types::adapter::{
    BindingContext, DestinationPostResult, OutboundPresentation, OutboundReply, SlackBlock,
    SlackText, TaskProvenance,
};
use moatless_types::loops::LoopContext;
use moatless_adapter_model::capabilities::{AdapterBindingReply, AdapterDirectDestination, AdapterDirectMessage, AdapterMessageReply};

use super::adapter::SlackAdapter;
use super::api::{PostMessage, SlackApi};
use super::config::SlackAdapterConfig;
use super::{subject, text};

/// Upload each attachment into `thread_ts`, best-effort: a failed upload is
/// logged and the reply still counts as sent.
async fn upload_files(api: &SlackApi, channel: &str, thread_ts: &str, message: &OutboundReply) {
    for file in &message.files {
        if let Err(e) = api
            .upload_file(channel, Some(thread_ts), &file.filename, file.data.clone())
            .await
        {
            warn!(
                filename = %file.filename,
                error = %e,
                "Failed to upload file attachment to Slack"
            );
        }
    }
}

/// The text to post: the reply, its provenance footer, and every `<@…>`
/// resolved to something Slack can address.
///
/// A Moatless login linked to a Slack account becomes that account's mention; a
/// Slack id is left alone, so anyone in the workspace is reachable whether or
/// not Moatless knows them; anything else stays plain text and pings nobody.
async fn outbound_text(
    adapter: &SlackAdapter,
    message: &OutboundReply,
    provenance: Option<&TaskProvenance>,
) -> String {
    let text = text::with_task_link(&message.text, provenance);
    let Some(users) = adapter.users() else {
        return text;
    };
    moatless_loops::adapters::mentions::render_mentions(
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
/// The loops band assembles provenance for every Slack reply; whether it is
/// shown is this adapter's call, because the setting lives in this adapter's
/// config.
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
/// presentation and the text path applies unchanged.
///
/// `provenance` is taken as an argument rather than read off `message` so the
/// decision sits where [`text::with_task_link`] already makes it.
fn reply_blocks(
    message: &OutboundReply,
    provenance: Option<&TaskProvenance>,
) -> Option<Vec<SlackBlock>> {
    let OutboundPresentation::Slack { blocks } = message.presentation.as_ref()?;
    let mut blocks = blocks.clone();
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
        let app = self.app(moatless_adapter_model::apps::DEFAULT_APP_KEY).await?;
        let coordinates = subject::thread_coordinates(&thread.destination_config);
        let channel = coordinates.channel.ok_or_else(|| {
            SharedAdapterError::Internal("No channel in thread properties".into())
        })?;
        let thread_ts = coordinates.thread_ts.ok_or_else(|| {
            SharedAdapterError::Internal("No thread_ts in thread properties".into())
        })?;

        let provenance = wanted_provenance(message, config);
        let text = outbound_text(self, message, provenance).await;
        let blocks = reply_blocks(message, provenance);

        let ts = app
            .api
            .post_message_with(PostMessage {
                channel,
                text: &text,
                thread_ts: Some(thread_ts),
                blocks: blocks.as_deref(),
            })
            .await?;

        upload_files(&app.api, channel, thread_ts, message).await;

        Ok(ts.map(|t| super::external_id::message_ts(&t)))
    }

    async fn send_reply_for_binding(
        &self,
        binding: &TaskBinding,
        _connection: &AdapterConnection,
        rule: Option<&SubscriptionMatch>,
        message: &OutboundReply,
    ) -> Result<Option<String>, AdapterError> {
        let app_key =
            moatless_adapter_model::apps::app_key_from_transport_config(&_connection.transport_config);
        let app = self.app(app_key).await?;
        let thread = moatless_adapter_model::models::binding_context(binding);
        let config = rule.map_or_else(
            || LoopContext {
                id: binding.loop_id.clone().unwrap_or_default(),
                adapter_kind: "slack".to_string(),
                source_id: subject::thread_coordinates(&binding.destination_config)
                    .channel
                    .unwrap_or_default()
                    .to_string(),
                source_name: None,
                repository_id: String::new(),
                routing_mode: moatless_types::loops::RoutingMode::BySubject,
                prompt_template: None,
                active: true,
                allow_direct_send: false,
                adapter_config: None,
                ongoing_task_id: None,
                scope: moatless_types::access_control::Scope::Global,
                task_name: None,
                tag_ids: vec![],
                skills: Vec::new(),
                run_as_user_id: None,
                agent_type: "claude-code".into(),
                inactive_ttl_days: None,
                created_by: "system".to_string(),
                created_at: binding.created_at,
                updated_at: binding.last_event_at,
            },
            moatless_adapter_model::models::subscription_context,
        );
        let coordinates = subject::thread_coordinates(&thread.destination_config);
        let channel = coordinates.channel.ok_or_else(|| {
            SharedAdapterError::Internal("No channel in thread properties".into())
        })?;
        let thread_ts = coordinates.thread_ts.ok_or_else(|| {
            SharedAdapterError::Internal("No thread_ts in thread properties".into())
        })?;

        let provenance = wanted_provenance(message, &config);
        let text = outbound_text(self, message, provenance).await;
        let blocks = reply_blocks(message, provenance);
        let ts = app
            .api
            .post_message_with(PostMessage {
                channel,
                text: &text,
                thread_ts: Some(thread_ts),
                blocks: blocks.as_deref(),
            })
            .await?;
        upload_files(&app.api, channel, thread_ts, message).await;

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
        _target_external_id: &str,
        message: &OutboundReply,
    ) -> Result<Option<String>, AdapterError> {
        self.send_reply_for_binding(binding, connection, rule, message)
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
        let app = self.app(moatless_adapter_model::apps::DEFAULT_APP_KEY).await?;
        let workspace_id = self.ensure_workspace_id().await?;
        let (channel, thread_ts) = subject::parse_destination(destination)?;

        // No `LoopContext` on this path, so the `task_link` toggle is not
        // readable; the caller decides by setting `provenance` or not.
        let provenance = message.provenance.as_ref();
        let text = outbound_text(self, message, provenance).await;
        let blocks = reply_blocks(message, provenance);
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

        // Files upload into the same thread (or top-level when there's no thread).
        let upload_thread_ts = thread_ts.clone().unwrap_or_else(|| ts.clone());
        upload_files(&app.api, &channel, &upload_thread_ts, message).await;

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
        let app = self.app(moatless_adapter_model::apps::DEFAULT_APP_KEY).await?;
        let dm_channel = app.api.open_dm(provider_user_id).await?;
        Ok(Some(format!("dm:{dm_channel}")))
    }
}

#[cfg(test)]
mod tests {
    use moatless_types::adapter::{
        BindingContext, OutboundFile, OutboundPresentation, OutboundReply, SlackBlock,
        SlackPlainText, TaskOrigin, TaskProvenance,
    };
    use moatless_types::loops::{LoopContext, RoutingMode};
    use moatless_loops::adapters::test_support::ProviderStub;
    use moatless_adapter_model::capabilities::{AdapterBindingReply, AdapterDirectDestination, AdapterDirectMessage};
    use moatless_types::access_control::Scope;

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
            repository_id: "repo-1".into(),
            routing_mode: RoutingMode::BySubject,
            prompt_template: None,
            active: true,
            allow_direct_send: true,
            adapter_config: None,
            ongoing_task_id: None,
            scope: Scope::Global,
            task_name: None,
            tag_ids: vec![],
            skills: Vec::new(),
            run_as_user_id: None,
            agent_type: "claude-code".into(),
            inactive_ttl_days: None,
            created_by: "user-1".into(),
            created_at: now,
            updated_at: now,
        }
    }

    fn body(request: &moatless_loops::adapters::test_support::RecordedRequest) -> serde_json::Value {
        serde_json::from_str(&request.body).expect("stub recorded a JSON body")
    }

    /// The budget: one `chat.postMessage`, nothing else. The reply threads under
    /// the binding's anchor and carries the task link.
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

    /// One attachment is one extra upload, and it lands in the same thread as
    /// the reply. The upload itself talks to the URL Slack hands back, which is
    /// not this stub — only the two Slack-side calls are budgeted here.
    #[tokio::test]
    async fn each_attachment_adds_one_upload_in_the_same_thread() {
        let stub = slack_stub().await;
        let binding = binding_context(serde_json::json!({
            "channel": "C1",
            "thread_ts": "1700000000.000100",
        }));
        let mut message = reply("with a file", None);
        message.files = vec![OutboundFile {
            filename: "log.txt".into(),
            mime_type: "text/plain".into(),
            data: b"contents".to_vec(),
        }];

        adapter(&stub)
            .send_reply(&binding, &loop_context("C1"), &message)
            .await
            .expect("an upload failure never fails the reply");

        assert_eq!(
            stub.paths(),
            vec![
                "/chat.postMessage".to_string(),
                "/files.getUploadURLExternal".to_string(),
            ],
            "the reply is sent first, then the attachment is offered"
        );
    }

    /// The budget: one `auth.test` (memoized) and one `chat.postMessage`. A
    /// top-level direct send binds under its own `ts` and renders the task
    /// link, because a channel post is often the only message a reader sees.
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

    /// Blocks go out untouched, and the task link moves into a block — the
    /// mrkdwn link in `text` is not displayed once blocks are present.
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
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

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
            posted["text"].as_str().is_some_and(|t| t.contains("loop: prod-release")),
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
    impl moatless_backend_core::identity::ProviderIdentityLink for LinkedLogins {
        async fn user_at(
            &self,
            _provider: &str,
            _provider_user_id: &str,
        ) -> Result<
            Option<moatless_backend_core::identity::LinkedUser>,
            moatless_backend_core::store::StoreError,
        > {
            Ok(None)
        }
        async fn user_by_email(
            &self,
            _email: &str,
        ) -> Result<
            Option<moatless_backend_core::identity::LinkedUser>,
            moatless_backend_core::store::StoreError,
        > {
            Ok(None)
        }
        async fn provider_id_for_login(
            &self,
            provider: &str,
            login: &str,
        ) -> Result<Option<String>, moatless_backend_core::store::StoreError> {
            Ok((provider == "slack" && login == "aorwall").then(|| "U0AAQ".to_string()))
        }
        async fn user_who_connected(
            &self,
            _provider: &str,
            _provider_user_id: &str,
        ) -> Result<Option<String>, moatless_backend_core::store::StoreError> {
            Ok(None)
        }
        async fn user_with_login(
            &self,
            _provider: &str,
            _login: &str,
        ) -> Result<Option<String>, moatless_backend_core::store::StoreError> {
            Ok(None)
        }
        async fn link(
            &self,
            _user_id: &uuid::Uuid,
            _provider: &str,
            _provider_user_id: &str,
        ) -> Result<bool, moatless_backend_core::store::StoreError> {
            Ok(false)
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
}
