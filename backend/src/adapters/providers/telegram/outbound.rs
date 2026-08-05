//! Outbound reply capabilities for Telegram chats.

use std::fmt::Write;

use crate::adapters::error::AdapterError;
use crate::adapters::models::{
    AdapterConnection, BindingContext, LoopContext, NewThreadInfo, OutboundReply, RoutingMode,
    SubscriptionMatch, TaskBinding,
};
use crate::adapters::{
    AdapterBindingReply, AdapterDirectDestination, AdapterDirectMessage, AdapterMessageReply,
    AdapterNewThread, DestinationPostResult,
};

use super::adapter::TelegramAdapter;
use super::subject::{
    parse_direct_destination, parse_message_target, TelegramDestination, SUBJECT_KIND,
};

/// The message text as Telegram receives it, task link included.
///
/// Every outbound path appends the same link the same way; before the breakup
/// it did so in four separate copies, which is three chances for a fifth path
/// to render it differently.
fn with_task_link(message: &OutboundReply) -> String {
    let mut text = message.text.clone();
    if let Some(ref url) = message.task_url {
        let _ = write!(text, "\n\n[View task]({url})");
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
        let api = self.app_api(crate::adapter_apps::DEFAULT_APP_KEY).await?;

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
        let thread = moatless_adapter_model::models::binding_context(binding);
        let app_key =
            crate::adapter_apps::app_key_from_transport_config(&connection.transport_config);
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
                repository_id: String::new(),
                routing_mode: RoutingMode::BySubject,
                prompt_template: None,
                active: true,
                allow_direct_send: false,
                adapter_config: None,
                ongoing_task_id: None,
                scope: moatless_backend_core::scope::Scope::Global,
                task_name: None,
                tag_ids: vec![],
                run_as_user_id: None,
                agent_type: "claude-code".to_string(),
                inactive_ttl_days: None,
                created_by: "system".to_string(),
                created_at: binding.created_at,
                updated_at: binding.last_event_at,
            },
            |r| moatless_adapter_model::models::subscription_context(r, "telegram"),
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
        let app_key =
            crate::adapter_apps::app_key_from_transport_config(&connection.transport_config);
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
        let api = self.app_api(crate::adapter_apps::DEFAULT_APP_KEY).await?;

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

#[async_trait::async_trait]
impl AdapterNewThread for TelegramAdapter {
    async fn send_message(
        &self,
        config: &LoopContext,
        message: &OutboundReply,
    ) -> Result<NewThreadInfo, AdapterError> {
        let chat_id = &config.source_id;
        let api = self.app_api(crate::adapter_apps::DEFAULT_APP_KEY).await?;

        let message_id = api
            .send_message_returning_id(chat_id, &with_task_link(message), None, None)
            .await?;
        Ok(NewThreadInfo {
            external_thread_id: message_id.to_string(),
            properties: serde_json::json!({
                "chat_id": chat_id,
                "message_id": message_id,
                "forum_topic_id": serde_json::Value::Null,
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;

    use crate::adapters::test_support::stub_provider;

    use super::super::api::TelegramApi;
    use super::super::test_support::{bot_api_stub, test_loop_context, BotApiStub};
    use super::*;

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
            adapter_kind: "telegram".into(),
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
        }
    }

    fn connection() -> AdapterConnection {
        let now = chrono::Utc::now();
        AdapterConnection {
            id: "pc_test".into(),
            adapter_kind: "telegram".into(),
            connection_kind: "env".into(),
            scope_value: "default".into(),
            credential_ref: None,
            webhook_secret: None,
            transport_config: serde_json::json!({}),
            created_by: "system".into(),
            scope: moatless_backend_core::scope::Scope::Global,
            owner_user_id: None,
            created_at: now,
            updated_at: now,
        }
    }

    fn reply(text: &str, task_url: Option<&str>) -> OutboundReply {
        OutboundReply {
            text: text.to_string(),
            task_url: task_url.map(str::to_string),
            footer: None,
            author_user_id: None,
            is_error: false,
            files: vec![],
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
        let provider = TelegramAdapter::new("fake_token");
        let err = provider
            .post_to_destination("999", &reply("hello", None))
            .await
            .expect_err("destination needs explicit chat prefix");

        match err {
            AdapterError::BadRequest(msg) => {
                assert!(msg.contains("chat:<id>"), "unexpected message: {msg}");
            }
            other => panic!("expected BadRequest, got {other:?}"),
        }
    }

    /// The spec's budget for a Telegram reply is one `sendMessage`, and every
    /// binding-shaped path has to hit it — a second call is a duplicate message
    /// in someone's chat, and a lost `reply_parameters` is a reply that stops
    /// threading.
    #[tokio::test]
    async fn a_binding_reply_is_one_send_message_onto_the_thread() {
        let (provider, stub) = provider_with_stub().await;
        let thread = moatless_adapter_model::models::binding_context(&binding(serde_json::json!({
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

    /// Opening a thread is one send too, and the properties it returns are what
    /// every later reply is addressed by.
    #[tokio::test]
    async fn opening_a_thread_is_one_send_message_and_reports_where_it_landed() {
        let (provider, stub) = provider_with_stub().await;

        let info = provider
            .send_message(&test_loop_context("999"), &reply("hello", None))
            .await
            .unwrap();

        assert_eq!(stub.methods(), vec!["sendMessage"]);
        assert_eq!(stub.calls()[0].body["chat_id"], "999");
        assert_eq!(info.external_thread_id, "77");
        assert_eq!(
            info.properties,
            serde_json::json!({
                "chat_id": "999",
                "message_id": 77,
                "forum_topic_id": serde_json::Value::Null,
            })
        );
    }

    #[tokio::test]
    async fn dm_destination_addresses_the_user_private_chat() {
        let provider = TelegramAdapter::new("fake_token");

        assert_eq!(
            provider.dm_destination("999").await.unwrap(),
            Some("chat:999".to_string())
        );
    }
}
