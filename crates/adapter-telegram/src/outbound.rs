//! Outbound reply capabilities for Telegram chats.

use std::fmt::Write;

use moatless_adapter_model::binding::BindingContext;
use moatless_adapter_model::capabilities::{
    AdapterBindingReply, AdapterDirectDestination, AdapterDirectMessage, AdapterMessageReply,
};
use moatless_adapter_model::error::AdapterError;
use moatless_adapter_model::outbound::{DestinationPostResult, OutboundReply};
use moatless_types::adapter::AdapterConnection;
use moatless_types::adapter::TaskBinding;
use moatless_types::loops::SubscriptionMatch;
use moatless_types::loops::{LoopContext, RoutingMode};

use super::adapter::TelegramAdapter;
use super::subject::{
    SUBJECT_KIND, TelegramDestination, parse_direct_destination, parse_message_target,
};

/// The message text as Telegram receives it, task link included.
///
/// Every outbound path appends the link the same way, so none can render it
/// differently.
fn with_task_link(message: &OutboundReply) -> String {
    let mut text = message.text.clone();
    if let Some(p) = message.provenance.as_ref() {
        let _ = write!(text, "\n\n[{}]({})", p.label(), p.url);
        for detail in p.details() {
            let _ = write!(text, " · {detail}");
        }
    }
    text
}

#[async_trait::async_trait]
impl AdapterBindingReply for TelegramAdapter {
    async fn send_reply(
        &self,
        thread: &BindingContext,
        _config: &LoopContext,
        message: &OutboundReply,
    ) -> Result<Option<String>, AdapterError> {
        let destination = TelegramDestination::from_thread_context(&thread.destination_config)?;
        let api = self
            .app_api(moatless_adapter_model::apps::DEFAULT_APP_KEY)
            .await?;

        api.send_message(
            destination.chat_id(),
            &with_task_link(message),
            destination.message_id(),
            destination.forum_topic_id(),
        )
        .await?;
        Ok(None)
    }

    async fn send_reply_for_binding(
        &self,
        binding: &TaskBinding,
        connection: &AdapterConnection,
        rule: Option<&SubscriptionMatch>,
        message: &OutboundReply,
    ) -> Result<Option<String>, AdapterError> {
        let thread = moatless_adapter_model::binding::binding_context(binding);
        let app_key = moatless_adapter_model::apps::app_key_from_transport_config(
            &connection.transport_config,
        );
        let api = self.app_api(app_key).await?;
        let _config = rule.map_or_else(
            || LoopContext {
                id: binding.loop_id.clone().unwrap_or_default(),
                adapter_kind: "telegram".to_string(),
                source_id: binding
                    .destination_config
                    .get("chat_id")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                source_name: None,
                workspace_id: String::new(),
                routing_mode: RoutingMode::BySubject,
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
        let destination = TelegramDestination::from_thread_context(&thread.destination_config)?;
        api.send_message(
            destination.chat_id(),
            &with_task_link(message),
            destination.message_id(),
            destination.forum_topic_id(),
        )
        .await?;
        Ok(None)
    }
}

#[async_trait::async_trait]
impl AdapterMessageReply for TelegramAdapter {
    async fn reply_to_message(
        &self,
        binding: &TaskBinding,
        connection: &AdapterConnection,
        _rule: Option<&SubscriptionMatch>,
        target_external_id: &str,
        message: &OutboundReply,
    ) -> Result<Option<String>, AdapterError> {
        let target = parse_message_target(target_external_id, "reply")?;
        let destination =
            TelegramDestination::from_destination_config(&binding.destination_config)?;
        let app_key = moatless_adapter_model::apps::app_key_from_transport_config(
            &connection.transport_config,
        );
        let api = self.app_api(app_key).await?;

        let new_id = api
            .send_message_returning_id(
                destination.chat_id(),
                &with_task_link(message),
                Some(target),
                destination.forum_topic_id(),
            )
            .await?;
        Ok(Some(super::external_id::message(new_id)))
    }
}

#[async_trait::async_trait]
impl AdapterDirectDestination for TelegramAdapter {
    async fn post_to_destination(
        &self,
        destination: &str,
        message: &OutboundReply,
    ) -> Result<DestinationPostResult, AdapterError> {
        let chat_id = parse_direct_destination(destination)?;
        let api = self
            .app_api(moatless_adapter_model::apps::DEFAULT_APP_KEY)
            .await?;

        let message_id = api
            .send_message_returning_id(chat_id, &with_task_link(message), None, None)
            .await?;

        Ok(DestinationPostResult {
            message_external_id: super::external_id::message(message_id),
            subject_kind: SUBJECT_KIND,
            subject_key: format!("{chat_id}:thread:{message_id}"),
            destination_config: serde_json::json!({
                "chat_id": chat_id,
                "message_id": message_id,
                "forum_topic_id": serde_json::Value::Null,
            }),
        })
    }
}

#[async_trait::async_trait]
impl AdapterDirectMessage for TelegramAdapter {
    /// A Telegram user's id doubles as the id of their private chat with the
    /// bot, so the destination needs no API call.
    async fn dm_destination(&self, provider_user_id: &str) -> Result<Option<String>, AdapterError> {
        Ok(Some(format!("chat:{provider_user_id}")))
    }
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use moatless_adapter_model::outbound::{TaskOrigin, TaskProvenance};
    use moatless_types::adapter::AdapterKind;

    use moatless_adapter_model::test_support::stub_provider;

    use super::super::api::TelegramApi;
    use super::super::test_support::{BotApiStub, bot_api_stub, test_loop_context};
    use super::*;
    use moatless_adapter_model::error::SharedAdapterError;

    /// A provider whose Bot API calls are recorded rather than sent.
    async fn provider_with_stub() -> (TelegramAdapter, BotApiStub) {
        let stub = bot_api_stub(serde_json::json!({ "message_id": 77 })).await;
        let provider = TelegramAdapter::with_api(TelegramApi::with_base_url(stub.base_url()));
        (provider, stub)
    }

    /// A binding pointing at one message in one chat.
    fn binding(destination_config: serde_json::Value) -> TaskBinding {
        let now = chrono::Utc::now();
        TaskBinding {
            id: "tb_test".into(),
            task_id: "task_test".into(),
            adapter_kind: AdapterKind::Telegram,
            subject_kind: SUBJECT_KIND.into(),
            subject_key: "999:thread:42".into(),
            connection_id: "pc_test".into(),
            loop_id: Some("tt_test".into()),
            destination_config,
            metadata: serde_json::json!({}),
            last_external_event_id: None,
            origin: "connect".into(),
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

    fn connection() -> AdapterConnection {
        let now = chrono::Utc::now();
        AdapterConnection {
            id: "pc_test".into(),
            adapter_kind: AdapterKind::Telegram,
            connection_kind: "env".into(),
            scope_value: "default".into(),
            credential_ref: None,
            webhook_secret: None,
            transport_config: serde_json::json!({}),
            created_by: "system".into(),
            scope: moatless_types::access_control::Scope::Global,
            owner_user_id: None,
            created_at: now,
            updated_at: now,
        }
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

    /// One append rule, and it is the same one on every outbound path.
    #[test]
    fn the_task_link_is_appended_once_and_only_when_there_is_one() {
        assert_eq!(with_task_link(&reply("hello", None)), "hello");
        assert_eq!(
            with_task_link(&reply("hello", Some("https://app.example.com/t/1"))),
            "hello\n\n[View task](https://app.example.com/t/1)"
        );
    }

    #[tokio::test]
    async fn direct_destination_posts_to_chat_destination_and_builds_binding_shape() {
        let base = stub_provider(StatusCode::OK, r#"{"ok":true,"result":{"message_id":77}}"#).await;
        let provider = TelegramAdapter::with_api(TelegramApi::with_base_url(base));

        let result = provider
            .post_to_destination("chat:999", &reply("hello", None))
            .await
            .unwrap();

        assert_eq!(result.message_external_id, "tg:m:77");
        assert_eq!(result.subject_kind, "telegram_thread");
        assert_eq!(result.subject_key, "999:thread:77");
        assert_eq!(
            result.destination_config,
            serde_json::json!({
                "chat_id": "999",
                "message_id": 77,
                "forum_topic_id": serde_json::Value::Null,
            })
        );
    }

    #[tokio::test]
    async fn direct_destination_rejects_non_chat_destination_shape() {
        let provider = TelegramAdapter::new(
            "fake_token",
            moatless_core::config::HttpConfig::default().telegram,
        );
        let err = provider
            .post_to_destination("999", &reply("hello", None))
            .await
            .expect_err("destination needs explicit chat prefix");

        match err {
            AdapterError::Shared(SharedAdapterError::BadRequest(msg)) => {
                assert!(msg.contains("chat:<id>"), "unexpected message: {msg}");
            }
            other => panic!("expected BadRequest, got {other:?}"),
        }
    }

    /// A binding reply must be exactly one `sendMessage`: a second call
    /// duplicates the message, and a lost `reply_parameters` breaks threading.
    #[tokio::test]
    async fn a_binding_reply_is_one_send_message_onto_the_thread() {
        let (provider, stub) = provider_with_stub().await;
        let thread =
            moatless_adapter_model::binding::binding_context(&binding(serde_json::json!({
                "chat_id": "999",
                "message_id": 42,
                "forum_topic_id": 7,
            })));

        provider
            .send_reply(
                &thread,
                &test_loop_context("999"),
                &reply("hello", Some("https://app.example.com/t/1")),
            )
            .await
            .unwrap();

        assert_eq!(stub.methods(), vec!["sendMessage"]);
        let call = &stub.calls()[0];
        assert_eq!(call.body["chat_id"], "999");
        assert_eq!(
            call.body["text"],
            "hello\n\n[View task](https://app.example.com/t/1)"
        );
        assert_eq!(call.body["reply_parameters"]["message_id"], 42);
        assert_eq!(call.body["message_thread_id"], 7);
    }

    /// The connection-shaped entry point is the same one send: it builds the
    /// context the binding path needs and delegates rather than sending again.
    #[tokio::test]
    async fn replying_for_a_binding_without_a_rule_is_still_one_send_message() {
        let (provider, stub) = provider_with_stub().await;
        let binding = binding(serde_json::json!({ "chat_id": "999", "message_id": 42 }));

        provider
            .send_reply_for_binding(&binding, &connection(), None, &reply("hello", None))
            .await
            .unwrap();

        assert_eq!(stub.methods(), vec!["sendMessage"]);
        assert_eq!(stub.calls()[0].body["chat_id"], "999");
        assert_eq!(stub.calls()[0].body["text"], "hello");
    }

    /// A reply to a named message targets *that* message, not the binding's,
    /// and reports back the id it created so the next reply can chain onto it.
    #[tokio::test]
    async fn replying_to_a_message_targets_it_and_returns_the_new_id() {
        let (provider, stub) = provider_with_stub().await;
        let binding = binding(serde_json::json!({
            "chat_id": "999",
            "message_id": 42,
            "forum_topic_id": serde_json::Value::Null,
        }));

        let new_id = provider
            .reply_to_message(&binding, &connection(), None, "tg:m:55", &reply("hi", None))
            .await
            .unwrap();

        assert_eq!(new_id.as_deref(), Some("tg:m:77"));
        assert_eq!(stub.methods(), vec!["sendMessage"]);
        assert_eq!(
            stub.calls()[0].body["reply_parameters"]["message_id"],
            55,
            "the named target, not the binding's message"
        );
    }

    #[tokio::test]
    async fn dm_destination_addresses_the_user_private_chat() {
        let provider = TelegramAdapter::new(
            "fake_token",
            moatless_core::config::HttpConfig::default().telegram,
        );

        assert_eq!(
            provider.dm_destination("999").await.unwrap(),
            Some("chat:999".to_string())
        );
    }
}
