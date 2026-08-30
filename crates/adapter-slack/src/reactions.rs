//! Emoji reactions on a Slack message.

use moatless_loops::adapters::AdapterReactions;
use moatless_loops::adapters::error::{AdapterError, SharedAdapterError};
use moatless_types::adapter::BindingContext;

use super::adapter::SlackAdapter;
use super::subject;

/// Strip leading/trailing `:` characters and validate that the result is a
/// non-empty Slack-compatible bare reaction name.
///
/// Slack expects bare names (`thumbsup`), not `:thumbsup:`. Returns
/// `BadRequest` for empty input so the caller surfaces a clear error before
/// hitting the API.
fn normalize_emoji(emoji: &str) -> Result<&str, AdapterError> {
    let trimmed = emoji.trim().trim_matches(':');
    if trimmed.is_empty() {
        return Err(SharedAdapterError::BadRequest("Empty emoji name".into()).into());
    }
    Ok(trimmed)
}

#[async_trait::async_trait]
impl AdapterReactions for SlackAdapter {
    async fn add_reaction(
        &self,
        binding: &BindingContext,
        target_external_id: &str,
        emoji: &str,
    ) -> Result<Option<String>, AdapterError> {
        let (channel, ts) = subject::reaction_target(binding, target_external_id)?;
        let name = normalize_emoji(emoji)?;
        let app = self.app(moatless_adapter_model::apps::DEFAULT_APP_KEY).await?;
        app.api.add_reaction(&channel, name, ts).await?;
        Ok(None)
    }

    async fn remove_reaction(
        &self,
        binding: &BindingContext,
        target_external_id: &str,
        emoji: &str,
    ) -> Result<(), AdapterError> {
        let (channel, ts) = subject::reaction_target(binding, target_external_id)?;
        let name = normalize_emoji(emoji)?;
        let app = self.app(moatless_adapter_model::apps::DEFAULT_APP_KEY).await?;
        app.api.remove_reaction(&channel, name, ts).await
    }
}

#[cfg(test)]
mod tests {
    use moatless_loops::adapters::AdapterReactions;
    use moatless_types::adapter::BindingContext;
    use moatless_loops::adapters::test_support::ProviderStub;

    use crate::adapter::SlackAdapter;
    use crate::api::SlackApi;
    use super::normalize_emoji;

    async fn slack_stub() -> ProviderStub {
        ProviderStub::start(|_| serde_json::json!({"ok": true})).await
    }

    fn adapter(stub: &ProviderStub) -> SlackAdapter {
        SlackAdapter::with_api(
            SlackApi::with_base_url("xoxb-test", stub.base_url()),
            "secret",
            None,
        )
    }

    fn binding() -> BindingContext {
        let now = chrono::Utc::now();
        BindingContext {
            binding_id: "tb_1".into(),
            loop_id: None,
            task_id: "task-1".into(),
            subject_kind: "slack_thread".into(),
            subject_key: "T1:C1:1700000000.000100".into(),
            destination_config: serde_json::json!({"channel": "C1"}),
            metadata: serde_json::json!({}),
            last_external_event_id: None,
            last_message_at: now,
            created_at: now,
        }
    }

    #[test]
    fn a_wrapped_emoji_is_sent_bare_and_an_empty_one_is_refused() {
        assert_eq!(normalize_emoji(":thumbsup:").unwrap(), "thumbsup");
        assert_eq!(normalize_emoji("  eyes  ").unwrap(), "eyes");
        assert_eq!(
            normalize_emoji(" :: ").unwrap_err().to_string(),
            "Invalid request: Empty emoji name"
        );
    }

    /// The budget: one call per reaction, carrying the binding's channel and
    /// the target's bare `ts`.
    #[tokio::test]
    async fn adding_and_removing_a_reaction_cost_one_call_each() {
        let stub = slack_stub().await;
        let adapter = adapter(&stub);

        assert_eq!(
            adapter
                .add_reaction(&binding(), "slack:ts:1700000000.000100", ":eyes:")
                .await
                .expect("the stub accepts the reaction"),
            None,
            "Slack reactions have no external id of their own"
        );
        adapter
            .remove_reaction(&binding(), "slack:ts:1700000000.000100", "eyes")
            .await
            .expect("the stub accepts the removal");

        assert_eq!(
            stub.paths(),
            vec![
                "/reactions.add".to_string(),
                "/reactions.remove".to_string()
            ]
        );
        let added: serde_json::Value =
            serde_json::from_str(&stub.requests()[0].body).expect("a JSON body");
        assert_eq!(
            added,
            serde_json::json!({
                "channel": "C1",
                "name": "eyes",
                "timestamp": "1700000000.000100",
            })
        );
    }

    #[tokio::test]
    async fn an_unusable_target_or_emoji_is_refused_before_any_call() {
        let stub = slack_stub().await;
        let adapter = adapter(&stub);

        assert_eq!(
            adapter
                .add_reaction(&binding(), "github:ic:42", "eyes")
                .await
                .unwrap_err()
                .to_string(),
            "Invalid request: Slack reaction target `github:ic:42` is not a valid slack:ts:<ts> id"
        );
        assert_eq!(
            adapter
                .add_reaction(&binding(), "slack:ts:1700000000.000100", ":")
                .await
                .unwrap_err()
                .to_string(),
            "Invalid request: Empty emoji name"
        );
        assert!(stub.paths().is_empty());
    }
}
