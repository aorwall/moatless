//! How a Slack thread is presented to the agent: the instructions and labels
//! that frame an inbound message, and the thread history fetched alongside it.

use moatless_loops::adapters::error::AdapterError;
use moatless_loops::adapters::models::{BindingContext, InboundMessage, LoopContext};
use moatless_loops::adapters::AdapterInboundPresentation;

use super::adapter::SlackAdapter;
use super::config::SlackAdapterConfig;
use super::{subject, text};

#[async_trait::async_trait]
impl AdapterInboundPresentation for SlackAdapter {
    fn reply_instructions(&self) -> Option<String> {
        Some(text::reply_instructions(self.bot_user_id()))
    }

    fn inbound_message_prefix(&self, msg: &InboundMessage) -> Option<String> {
        msg.adapter_context
            .get("is_bot_mentioned")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
            .then(|| {
                "[You were @mentioned - a reply via `moat messages reply --message <id>` is required]"
                    .to_string()
            })
    }

    fn thread_reply_hint(&self, msg: &InboundMessage) -> Option<String> {
        let is_bot_mentioned = msg
            .adapter_context
            .get("is_bot_mentioned")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        if is_bot_mentioned {
            None
        } else {
            Some("[New message in Slack thread]".to_string())
        }
    }

    async fn fetch_context(
        &self,
        thread: &BindingContext,
        config: &LoopContext,
        limit: usize,
        exclude_external_ids: &[String],
    ) -> Result<Option<String>, AdapterError> {
        let coordinates = subject::thread_coordinates(&thread.destination_config);
        // An absent coordinate and an empty one are the same nothing here: a
        // fetch with either is a Slack call that cannot succeed.
        let (Some(channel), Some(thread_ts)) = (
            coordinates.channel.filter(|s| !s.is_empty()),
            coordinates.thread_ts.filter(|s| !s.is_empty()),
        ) else {
            return Ok(None);
        };

        let slack_config = SlackAdapterConfig::from_config(config);
        let config_limit = slack_config.thread_context_limit.unwrap_or(limit as u64) as usize;

        // Use last_external_event_id as cursor — only fetch messages after it.
        // Strip the `slack:ts:` namespace prefix written by `send_reply` so the
        // Slack API still receives a bare ts.
        let oldest_raw = thread
            .destination_config
            .get("last_external_event_id")
            .and_then(|v| v.as_str());
        let oldest = oldest_raw.map(|s| s.strip_prefix("slack:ts:").unwrap_or(s));

        let app = self.app(moatless_identity::adapter_apps::DEFAULT_APP_KEY).await?;
        let replies = app
            .api
            .get_thread_replies(
                channel,
                thread_ts,
                config_limit,
                app.bot_user_id.as_deref(),
                oldest,
            )
            .await?;

        if replies.is_empty() {
            return Ok(None);
        }

        // Slack always returns the parent message regardless of `oldest`,
        // so filter client-side to only include messages strictly after the cursor.
        // Also exclude messages already delivered to the task (e.g. the current
        // message being processed) to avoid duplicates in the context. Match
        // both new prefixed and legacy bare-ts forms.
        let messages: Vec<&str> = replies
            .iter()
            .filter(|r| match (&r.ts, oldest) {
                (Some(ts), Some(cursor)) => ts.as_str() > cursor,
                _ => true,
            })
            .filter(|r| match &r.ts {
                Some(ts) => {
                    let prefixed = super::external_id::message_ts(ts);
                    !exclude_external_ids.contains(&prefixed) && !exclude_external_ids.contains(ts)
                }
                None => true,
            })
            .map(|r| r.text.as_str())
            .collect();

        if messages.is_empty() {
            return Ok(None);
        }

        Ok(Some(messages.join("\n\n---\n\n")))
    }
}

#[cfg(test)]
mod tests {
    use moatless_loops::adapters::models::{
        Audience, BindingContext, InboundMessage, LoopContext, RoutingMode,
    };
    use moatless_loops::adapters::test_support::ProviderStub;
    use moatless_loops::adapters::{AdapterInboundPresentation, AdapterKind};
    use moatless_backend_core::scope::Scope;

    use crate::adapter::SlackAdapter;
    use crate::api::SlackApi;

    /// A thread of four messages: the parent, one before the cursor, and two
    /// after it.
    async fn slack_stub() -> ProviderStub {
        ProviderStub::start(|path| match path {
            "/conversations.replies" => serde_json::json!({
                "ok": true,
                "messages": [
                    {"ts": "1700000000.000100", "text": "the parent", "user": "U1"},
                    {"ts": "1700000000.000200", "text": "before the cursor", "user": "U1"},
                    {"ts": "1700000000.000400", "text": "after the cursor", "user": "U1"},
                    {"ts": "1700000000.000500", "text": "already delivered", "user": "U1"},
                ],
            }),
            other => serde_json::json!({"ok": false, "error": format!("unexpected {other}")}),
        })
        .await
    }

    fn adapter(stub: &ProviderStub, bot_user_id: Option<&str>) -> SlackAdapter {
        SlackAdapter::with_api(
            SlackApi::with_base_url("xoxb-test", stub.base_url()),
            "secret",
            bot_user_id.map(ToString::to_string),
        )
    }

    fn binding(destination_config: serde_json::Value) -> BindingContext {
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

    fn loop_context(adapter_config: Option<serde_json::Value>) -> LoopContext {
        let now = chrono::Utc::now();
        LoopContext {
            id: "tt_1".into(),
            adapter_kind: "slack".into(),
            source_id: "C1".into(),
            source_name: None,
            repository_id: "repo-1".into(),
            routing_mode: RoutingMode::BySubject,
            prompt_template: None,
            active: true,
            allow_direct_send: true,
            adapter_config,
            ongoing_task_id: None,
            scope: Scope::Global,
            task_name: None,
            tag_ids: vec![],
            run_as_user_id: None,
            agent_type: "claude-code".into(),
            inactive_ttl_days: None,
            created_by: "user-1".into(),
            created_at: now,
            updated_at: now,
        }
    }

    fn message(adapter_context: serde_json::Value) -> InboundMessage {
        InboundMessage {
            adapter_kind: AdapterKind::Slack,
            source_id: "C1".into(),
            thread_id: None,
            message_thread_id: "1700000000.000100".into(),
            user_id: "user-1".into(),
            author: None,
            text: "hello".into(),
            file_ids: vec![],
            thread_properties: serde_json::json!({}),
            adapter_context,
            external_event_id: None,
            source_task_id: None,
            audience: Audience::Everyone,
        }
    }

    /// A mention demands a reply; anything else is announced as a thread
    /// message. The two are mutually exclusive, and that is the whole rule.
    #[test]
    fn a_mention_is_prefixed_and_everything_else_is_hinted() {
        let adapter = SlackAdapter::new("token", "secret", None);
        let mentioned = message(serde_json::json!({"is_bot_mentioned": true}));
        let not_mentioned = message(serde_json::json!({"is_bot_mentioned": false}));

        assert_eq!(
            adapter.inbound_message_prefix(&mentioned),
            Some("[You were @mentioned - a reply via `moat messages reply --message <id>` is required]".to_string())
        );
        assert_eq!(adapter.thread_reply_hint(&mentioned), None);

        assert_eq!(adapter.inbound_message_prefix(&not_mentioned), None);
        assert_eq!(
            adapter.thread_reply_hint(&not_mentioned),
            Some("[New message in Slack thread]".to_string())
        );
        assert_eq!(
            adapter.thread_reply_hint(&message(serde_json::json!({}))),
            Some("[New message in Slack thread]".to_string()),
            "an absent flag is not a mention"
        );
    }

    #[test]
    fn the_reply_instructions_name_the_bot() {
        assert!(SlackAdapter::new("token", "secret", Some("U0BOT".into()))
            .reply_instructions()
            .expect("Slack always has instructions")
            .contains("Slack user ID `U0BOT`"));
    }

    /// The budget: one `conversations.replies`. The cursor is sent bare, the
    /// parent Slack always returns is dropped client-side, and an
    /// already-delivered message is excluded in either id form.
    #[tokio::test]
    async fn a_context_fetch_costs_one_call_and_filters_what_it_gets_back() {
        let stub = slack_stub().await;

        let context = adapter(&stub, Some("U0BOT"))
            .fetch_context(
                &binding(serde_json::json!({
                    "channel": "C1",
                    "thread_ts": "1700000000.000100",
                    "last_external_event_id": "slack:ts:1700000000.000300",
                })),
                &loop_context(None),
                50,
                &["slack:ts:1700000000.000500".to_string()],
            )
            .await
            .expect("the stub answers");

        assert_eq!(context.as_deref(), Some("after the cursor"));
        assert_eq!(stub.paths(), vec!["/conversations.replies".to_string()]);
        let query = &stub.requests()[0].query;
        assert!(
            query.contains("oldest=1700000000.000300"),
            "the `slack:ts:` prefix is stripped before Slack sees it, got `{query}`"
        );
        assert!(query.contains("limit=50"));
    }

    #[tokio::test]
    async fn a_bare_legacy_id_is_excluded_too() {
        let stub = slack_stub().await;

        let context = adapter(&stub, None)
            .fetch_context(
                &binding(serde_json::json!({
                    "channel": "C1",
                    "thread_ts": "1700000000.000100",
                    "last_external_event_id": "1700000000.000300",
                })),
                &loop_context(None),
                50,
                &["1700000000.000500".to_string()],
            )
            .await
            .expect("the stub answers");

        assert_eq!(context.as_deref(), Some("after the cursor"));
    }

    #[tokio::test]
    async fn the_trigger_limit_overrides_the_callers() {
        let stub = slack_stub().await;

        adapter(&stub, None)
            .fetch_context(
                &binding(serde_json::json!({
                    "channel": "C1",
                    "thread_ts": "1700000000.000100",
                })),
                &loop_context(Some(serde_json::json!({"thread_context_limit": 5}))),
                50,
                &[],
            )
            .await
            .expect("the stub answers");

        assert!(stub.requests()[0].query.contains("limit=5"));
    }

    /// Nothing left after filtering is the same answer as nothing fetched: no
    /// context block at all, rather than an empty one.
    #[tokio::test]
    async fn everything_filtered_out_is_no_context() {
        let stub = slack_stub().await;

        let context = adapter(&stub, None)
            .fetch_context(
                &binding(serde_json::json!({
                    "channel": "C1",
                    "thread_ts": "1700000000.000100",
                    "last_external_event_id": "1700000000.000900",
                })),
                &loop_context(None),
                50,
                &[],
            )
            .await
            .expect("the stub answers");

        assert_eq!(context, None);
    }

    #[tokio::test]
    async fn a_binding_without_coordinates_is_never_fetched_for() {
        let stub = slack_stub().await;
        let adapter = adapter(&stub, None);

        for destination_config in [
            serde_json::json!({"thread_ts": "1700000000.000100"}),
            serde_json::json!({"channel": "C1"}),
            serde_json::json!({}),
        ] {
            assert_eq!(
                adapter
                    .fetch_context(&binding(destination_config), &loop_context(None), 50, &[])
                    .await
                    .expect("no call, no failure"),
                None
            );
        }
        assert!(stub.paths().is_empty());
    }
}
