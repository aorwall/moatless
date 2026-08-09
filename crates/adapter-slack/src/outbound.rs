//! Everything the Slack Adapter sends: thread replies, direct posts, DMs, and
//! the first message of a new thread.

use tracing::warn;

use moatless_loops::adapters::error::{AdapterError, SharedAdapterError};
use moatless_loops::adapters::models::{
    AdapterConnection, BindingContext, LoopContext, OutboundReply, SubscriptionMatch, TaskBinding,
};
use moatless_loops::adapters::{
    AdapterBindingReply, AdapterDirectDestination, AdapterDirectMessage, AdapterMessageReply,
    DestinationPostResult,
};

use super::adapter::SlackAdapter;
use super::api::SlackApi;
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

#[async_trait::async_trait]
impl AdapterBindingReply for SlackAdapter {
    async fn send_reply(
        &self,
        thread: &BindingContext,
        _config: &LoopContext,
        message: &OutboundReply,
    ) -> Result<Option<String>, AdapterError> {
        let app = self.app(moatless_identity::adapter_apps::DEFAULT_APP_KEY).await?;
        let coordinates = subject::thread_coordinates(&thread.destination_config);
        let channel = coordinates.channel.ok_or_else(|| {
            SharedAdapterError::Internal("No channel in thread properties".into())
        })?;
        let thread_ts = coordinates.thread_ts.ok_or_else(|| {
            SharedAdapterError::Internal("No thread_ts in thread properties".into())
        })?;

        let text = text::with_task_link(&message.text, message.task_url.as_deref());

        let ts = app
            .api
            .post_message(channel, &text, Some(thread_ts))
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
            moatless_identity::adapter_apps::app_key_from_transport_config(&_connection.transport_config);
        let app = self.app(app_key).await?;
        let thread = moatless_adapter_model::models::binding_context(binding);
        let _config = rule.map_or_else(
            || LoopContext {
                id: binding.loop_id.clone().unwrap_or_default(),
                adapter_kind: "slack".to_string(),
                source_id: subject::thread_coordinates(&binding.destination_config)
                    .channel
                    .unwrap_or_default()
                    .to_string(),
                source_name: None,
                repository_id: String::new(),
                routing_mode: moatless_loops::adapters::models::RoutingMode::BySubject,
                prompt_template: None,
                active: true,
                allow_direct_send: false,
                adapter_config: None,
                ongoing_task_id: None,
                scope: moatless_backend_core::scope::Scope::Global,
                task_name: None,
                tag_ids: vec![],
                skills: Vec::new(),
                run_as_user_id: None,
                agent_type: "claude-code".to_string(),
                inactive_ttl_days: None,
                created_by: "system".to_string(),
                created_at: binding.created_at,
                updated_at: binding.last_event_at,
            },
            |r| moatless_adapter_model::models::subscription_context(r, "slack"),
        );
        let coordinates = subject::thread_coordinates(&thread.destination_config);
        let channel = coordinates.channel.ok_or_else(|| {
            SharedAdapterError::Internal("No channel in thread properties".into())
        })?;
        let thread_ts = coordinates.thread_ts.ok_or_else(|| {
            SharedAdapterError::Internal("No thread_ts in thread properties".into())
        })?;

        let text = text::with_task_link(&message.text, message.task_url.as_deref());
        let ts = app
            .api
            .post_message(channel, &text, Some(thread_ts))
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
        let app = self.app(moatless_identity::adapter_apps::DEFAULT_APP_KEY).await?;
        let workspace_id = self.ensure_workspace_id().await?;
        let (channel, thread_ts) = subject::parse_destination(destination)?;

        let ts = app
            .api
            .post_message(&channel, &message.text, thread_ts.as_deref())
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
        let app = self.app(moatless_identity::adapter_apps::DEFAULT_APP_KEY).await?;
        let dm_channel = app.api.open_dm(provider_user_id).await?;
        Ok(Some(format!("dm:{dm_channel}")))
    }
}

#[cfg(test)]
mod tests {
    use moatless_loops::adapters::models::{
        BindingContext, LoopContext, OutboundFile, OutboundReply, RoutingMode,
    };
    use moatless_loops::adapters::test_support::ProviderStub;
    use moatless_loops::adapters::{
        AdapterBindingReply, AdapterDirectDestination, AdapterDirectMessage,
    };
    use moatless_backend_core::scope::Scope;

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
            task_url: task_url.map(ToString::to_string),
            footer: None,
            author_user_id: None,
            is_error: false,
            files: vec![],
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
            adapter_kind: "slack".into(),
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
    /// top-level direct send binds under its own `ts`, and — unlike a reply —
    /// renders no task link.
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
            "hello",
            "a direct send has never rendered the task link"
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

}
