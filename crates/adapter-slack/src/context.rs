//! How a Slack thread is presented to the agent: the instructions and labels
//! that frame an inbound message, and the thread history fetched alongside it.

use moatless_adapter_model::binding::BindingContext;
use moatless_adapter_model::capabilities::AdapterInboundPresentation;
use moatless_adapter_model::error::AdapterError;
use moatless_types::adapter::InboundMessage;
use moatless_types::loops::LoopContext;

use super::adapter::SlackAdapter;
use super::config::SlackAdapterConfig;
use super::{subject, text};

#[async_trait::async_trait]
impl AdapterInboundPresentation for SlackAdapter {
    async fn reply_instructions(&self) -> Option<String> {
        Some(text::reply_instructions(self.ensure_bot_user_id().await))
    }

    fn inbound_message_prefix(&self, msg: &InboundMessage) -> Option<String> {
        msg.adapter_context
            .get("is_bot_mentioned")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
            .then(|| {
                "[You were @mentioned. If the message is addressed to you, reply with `moat messages reply --message <id>`: plain assistant text does not reach Slack.]"
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

        let app = self
            .app(moatless_adapter_model::apps::DEFAULT_APP_KEY)
            .await?;
        let replies = app
            .api
            .get_thread_replies(channel, thread_ts, config_limit, oldest)
            .await?;

        let replies = select_replies(replies, app.bot_user_id.as_deref());

        if replies.is_empty() {
            return Ok(None);
        }

        // Slack always returns the parent regardless of `oldest`; filter
        // client-side to messages strictly after the cursor. Also drop
        // messages already delivered to the task — matching both new
        // prefixed and legacy bare-ts forms — to avoid duplicates.
        let kept: Vec<&ThreadReply> = replies
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
            .collect();

        if kept.is_empty() {
            return Ok(None);
        }

        // Who said what, so a multi-person exchange the agent is catching up on
        // reads as one. Resolved once per distinct writer in the window.
        let mut names: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        if let Some(users) = self.users() {
            for reply in &kept {
                let Some(user) = reply.user.as_deref().filter(|u| !u.is_empty()) else {
                    continue;
                };
                if names.contains_key(user) {
                    continue;
                }
                if let Some(name) = super::auth::name_of(&app.api, users, user).await {
                    names.insert(user.to_string(), name);
                }
            }
        }

        let messages: Vec<String> = kept
            .iter()
            .map(|r| match r.user.as_deref().filter(|u| !u.is_empty()) {
                Some(user) => {
                    let who = names.get(user).map_or(user, String::as_str);
                    format!("{who}: {}", r.text)
                }
                None => r.text.clone(),
            })
            .collect();

        Ok(Some(messages.join("\n\n---\n\n")))
    }
}

/// A formatted thread reply with its Slack timestamp for dedup filtering.
pub struct ThreadReply {
    /// The formatted message text.
    pub text: String,
    /// The Slack message timestamp (unique ID).
    pub ts: Option<String>,
    /// The Slack id of whoever wrote it, so history reads as a conversation
    /// between named people rather than one anonymous block.
    pub user: Option<String>,
}

fn select_replies(
    messages: Vec<super::api::SlackMessage>,
    bot_user_id: Option<&str>,
) -> Vec<ThreadReply> {
    messages
        .into_iter()
        .filter(|m| {
            if let Some(ref uid) = m.user {
                if let Some(bot_id) = bot_user_id {
                    return uid != bot_id;
                }
            }
            true
        })
        .filter_map(|m| {
            let ts = m.ts.clone();
            let user = m.user.clone();
            // Try blocks first (Block Kit), then attachments (legacy), then text (fallback)
            let text = m
                .blocks
                .as_ref()
                .map(|b| crate::format::extract_text_from_blocks(b))
                .filter(|t| !t.is_empty())
                .or_else(|| {
                    m.attachments
                        .as_ref()
                        .map(|a| crate::format::extract_text_from_attachments(a))
                        .filter(|t| !t.is_empty())
                })
                .or_else(|| {
                    m.text
                        .filter(|t| !t.is_empty())
                        .map(|t| crate::format::format_slack_text(&t))
                })?;
            Some(ThreadReply { text, ts, user })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use moatless_adapter_model::binding::BindingContext;
    use moatless_adapter_model::capabilities::AdapterInboundPresentation;
    use moatless_adapter_model::test_support::ProviderStub;
    use moatless_types::access_control::Scope;
    use moatless_types::adapter::{AdapterKind, Audience, InboundMessage};
    use moatless_types::loops::{LoopContext, RoutingMode};

    use crate::adapter::SlackAdapter;
    use crate::api::SlackApi;

    #[tokio::test]
    async fn history_selects_provider_content_and_excludes_own_bot() {
        let stub = ProviderStub::start(|_| serde_json::json!({
            "ok": true,
            "messages": [
                {"user":"BOT", "text":"own output"},
                {"user":"U1", "text":""},
                {"user":"U1", "text":"fallback", "blocks":[{"type":"section", "text":{"type":"mrkdwn", "text":"block text"}}]},
                {"user":"U2", "text":"fallback", "attachments":[{"text":"attachment text"}]},
                {"text":"   "}
            ]
        })).await;
        let text = adapter(&stub, Some("BOT"))
            .fetch_context(
                &binding(serde_json::json!({"channel":"C1", "thread_ts":"1"})),
                &loop_context(None),
                50,
                &[],
            )
            .await
            .unwrap()
            .unwrap();
        assert!(text.contains("U1: block text"));
        assert!(text.contains("U2: attachment text"));
        assert!(!text.contains("fallback"));
        assert!(!text.contains("own output"));
        assert!(text.ends_with("   "));
        assert_eq!(stub.paths(), vec!["/conversations.replies"]);
    }

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
            adapter_kind: "slack".to_string(),
            source_id: "C1".into(),
            source_name: None,
            workspace_id: "ws-1".into(),
            routing_mode: RoutingMode::BySubject,
            prompt_template: None,
            active: true,
            allow_direct_send: true,
            adapter_config,
            ongoing_task_id: None,
            scope: Scope::Global,
            task_name: None,
            tag_ids: vec![],
            skills: Vec::new(),
            run_as_user_id: None,
            agent_profile: moatless_types::agent::AgentProfile::ClaudeCode,
            model: None,
            inactive_ttl_days: None,
            created_by: "user-1".into(),
            created_at: now,
            updated_at: now,
            node_selector: None,
            stop_when_done: false,
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
            sender_external_id: None,
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
        let adapter = SlackAdapter::new(
            "token",
            "secret",
            None,
            moatless_core::config::HttpConfig::default().slack,
        );
        let mentioned = message(serde_json::json!({"is_bot_mentioned": true}));
        let not_mentioned = message(serde_json::json!({"is_bot_mentioned": false}));

        assert_eq!(
            adapter.inbound_message_prefix(&mentioned),
            Some("[You were @mentioned. If the message is addressed to you, reply with `moat messages reply --message <id>`: plain assistant text does not reach Slack.]".to_string())
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

    #[tokio::test]
    async fn the_reply_instructions_name_the_configured_bot_without_asking_slack() {
        let stub = slack_stub().await;

        let instructions = adapter(&stub, Some("U0BOT"))
            .reply_instructions()
            .await
            .expect("Slack always has instructions");

        assert!(instructions.contains("Slack user ID `U0BOT`"));
        assert!(stub.paths().is_empty(), "a configured id costs no call");
    }

    /// The reported incident: an unset `bot_user_id` left the agent silently
    /// answering a question between two other people.
    ///
    /// The id is derivable, so an unset field must not decide agent behavior.
    #[tokio::test]
    async fn an_unconfigured_bot_id_is_derived_so_the_silence_rules_still_ship() {
        let stub = identified_stub().await;
        let adapter = adapter(&stub, None);

        let instructions = adapter
            .reply_instructions()
            .await
            .expect("Slack always has instructions");

        assert!(instructions.contains("Slack user ID `U0DERIVED`"));
        assert!(instructions.contains("### When to respond in thread conversations"));
        assert!(instructions.contains("**Stay silent** when people are talking to each other"));

        adapter
            .reply_instructions()
            .await
            .expect("Slack always has instructions");
        assert_eq!(
            stub.paths(),
            vec!["/auth.test".to_string()],
            "the derived id is memoized across firings"
        );
    }

    /// A deployment whose Slack is unreachable still fires tasks. It loses the
    /// bot's name, not the whole prompt.
    #[tokio::test]
    async fn an_unreachable_slack_still_yields_instructions() {
        let adapter = SlackAdapter::with_api(
            SlackApi::with_base_url(
                "xoxb-test",
                moatless_adapter_model::test_support::UNREACHABLE_BASE,
            ),
            "secret",
            None,
        );

        let instructions = adapter
            .reply_instructions()
            .await
            .expect("Slack always has instructions");

        assert!(instructions.contains("not configured"));
        assert!(!instructions.contains("### When to respond in thread conversations"));
    }

    async fn identified_stub() -> ProviderStub {
        ProviderStub::start(|path| match path {
            "/auth.test" => serde_json::json!({
                "ok": true, "user_id": "U0DERIVED", "user": "soapbot",
                "team": "acme", "team_id": "T1",
            }),
            other => serde_json::json!({"ok": false, "error": format!("unexpected {other}")}),
        })
        .await
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

        assert_eq!(
            context.as_deref(),
            Some("U1: after the cursor"),
            "no directory here, so the writer is named by the id that still tells them apart"
        );
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

        assert_eq!(context.as_deref(), Some("U1: after the cursor"));
    }

    /// A directory that links `U1` to the login `aorwall`, and nobody else.
    struct OneLink;

    #[async_trait::async_trait]
    impl moatless_ports::identity::ProviderIdentityLink for OneLink {
        async fn user_at(
            &self,
            _provider: &str,
            provider_user_id: &str,
        ) -> Result<Option<moatless_ports::identity::LinkedUser>, moatless_types::store::StoreError>
        {
            Ok(
                (provider_user_id == "U1").then(|| moatless_ports::identity::LinkedUser {
                    id: uuid::Uuid::nil(),
                    login: "aorwall".to_string(),
                }),
            )
        }
    }

    /// History the agent is catching up on says who wrote each line, in the
    /// same terms the `<from>` header uses.
    #[tokio::test]
    async fn history_names_the_writer_it_can_resolve() {
        let stub = slack_stub().await;

        let context = adapter(&stub, None)
            .with_users(std::sync::Arc::new(OneLink))
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

        assert_eq!(context.as_deref(), Some("aorwall: after the cursor"));
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
