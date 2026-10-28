//! Fetching the files a Slack message shares, as the connection's own bot.

use moatless_adapter_model::capabilities::AdapterInboundFiles;
use moatless_adapter_model::error::AdapterError;
use moatless_types::adapter::{AdapterConnection, InboundAttachment};

use super::adapter::SlackAdapter;

#[async_trait::async_trait]
impl AdapterInboundFiles for SlackAdapter {
    async fn fetch_attachment(
        &self,
        connection: &AdapterConnection,
        attachment: &InboundAttachment,
        max_bytes: u64,
    ) -> Result<Vec<u8>, AdapterError> {
        let app_key = moatless_adapter_model::apps::app_key_from_transport_config(
            &connection.transport_config,
        );
        let app = self.app(app_key).await?;
        app.api
            .download_file(&attachment.download_url, &attachment.mime_type, max_bytes)
            .await
    }
}

#[cfg(test)]
mod tests {
    use moatless_adapter_model::capabilities::AdapterInboundFiles;
    use moatless_adapter_model::test_support::ProviderStub;
    use moatless_types::access_control::Scope;
    use moatless_types::adapter::{AdapterConnection, AdapterKind, InboundAttachment};

    use crate::adapter::SlackAdapter;
    use crate::api::SlackApi;

    fn attachment(download_url: String) -> InboundAttachment {
        InboundAttachment {
            external_id: "F1".into(),
            filename: "log.txt".into(),
            mime_type: "text/plain".into(),
            size: Some(7),
            download_url,
        }
    }

    fn connection() -> AdapterConnection {
        let now = chrono::Utc::now();
        AdapterConnection {
            id: "ac_1".into(),
            adapter_kind: AdapterKind::Slack,
            connection_kind: "app".into(),
            scope_value: "T1".into(),
            credential_ref: None,
            webhook_secret: None,
            transport_config: serde_json::json!({}),
            created_by: "user-1".into(),
            scope: Scope::Global,
            owner_user_id: None,
            created_at: now,
            updated_at: now,
        }
    }

    #[tokio::test]
    async fn a_shared_file_is_read_with_the_bot_token() {
        let stub = ProviderStub::start(|_| serde_json::json!("payload")).await;
        let adapter = SlackAdapter::with_api(
            SlackApi::with_base_url("xoxb-test", stub.base_url()),
            "secret",
            None,
        );

        let bytes = adapter
            .fetch_attachment(
                &connection(),
                &attachment(format!(
                    "{}/files-pri/T1-F1/download/log.txt",
                    stub.base_url()
                )),
                1024,
            )
            .await
            .expect("the stub serves the file");

        assert_eq!(bytes, b"\"payload\"");
        assert_eq!(
            stub.paths(),
            vec!["/files-pri/T1-F1/download/log.txt".to_string()]
        );
    }

    /// A body past the cap fails the read instead of being buffered whole.
    #[tokio::test]
    async fn a_file_past_the_cap_is_refused() {
        let stub =
            ProviderStub::start(|_| serde_json::json!("a body longer than eight bytes")).await;
        let adapter = SlackAdapter::with_api(
            SlackApi::with_base_url("xoxb-test", stub.base_url()),
            "secret",
            None,
        );

        let error = adapter
            .fetch_attachment(
                &connection(),
                &attachment(format!(
                    "{}/files-pri/T1-F1/download/log.txt",
                    stub.base_url()
                )),
                8,
            )
            .await
            .expect_err("the body is larger than the cap");

        assert!(error.to_string().contains("more than 8 bytes"), "{error}");
    }

    /// A payload naming another host must not receive the bot token.
    #[tokio::test]
    async fn a_url_off_slack_is_refused_before_any_request() {
        let stub = ProviderStub::start(|_| serde_json::json!("payload")).await;
        let adapter = SlackAdapter::with_api(
            SlackApi::with_base_url("xoxb-test", stub.base_url()),
            "secret",
            None,
        );

        let error = adapter
            .fetch_attachment(
                &connection(),
                &attachment("https://attacker.example/steal".into()),
                1024,
            )
            .await
            .expect_err("only Slack hosts get the token");

        assert!(
            error
                .to_string()
                .contains("Refusing to send the Slack token"),
            "{error}"
        );
        assert!(stub.paths().is_empty());
    }
}
