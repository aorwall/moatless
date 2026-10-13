//! Telegram Bot API client.

use std::time::Duration;

use reqwest::Client;
use tracing::warn;

use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
use moatless_types::http::HttpTimeouts;

/// Maximum message length for Telegram's sendMessage API.
const MAX_MESSAGE_LENGTH: usize = 4096;

/// Telegram Bot API client.
#[derive(Clone)]
pub struct TelegramApi {
    client: Client,
    base_url: String,
}

/// `getUpdates` sets its own longer per-request timeout over this one.
#[expect(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "sets the timeouts `Config::http` holds, and fails as `Client::new` would"
)]
fn http_client(timeouts: HttpTimeouts) -> Client {
    Client::builder()
        .connect_timeout(timeouts.connect)
        .timeout(timeouts.total)
        .build()
        .expect("the TLS backend initialises")
}

fn bot_base(bot_token: &str) -> String {
    format!("https://api.telegram.org/bot{bot_token}")
}

impl TelegramApi {
    pub fn new(bot_token: &str, timeouts: HttpTimeouts) -> Self {
        Self {
            client: http_client(timeouts),
            base_url: bot_base(bot_token),
        }
    }

    /// The same client, calling as the bot `bot_token` names.
    pub(crate) fn with_token(&self, bot_token: &str) -> Self {
        Self {
            client: self.client.clone(),
            base_url: bot_base(bot_token),
        }
    }

    /// Point the client at a caller-supplied Bot API base so tests can drive
    /// the real request path against a local address.
    #[cfg(test)]
    pub(crate) fn with_base_url(base_url: impl Into<String>) -> Self {
        Self {
            client: http_client(moatless_core::config::HttpConfig::default().telegram),
            base_url: base_url.into(),
        }
    }

    /// Send a text message, chunking if it exceeds 4096 characters.
    ///
    /// `message_thread_id`, when set, posts into that forum/topic group's
    /// specified topic.
    pub async fn send_message(
        &self,
        chat_id: &str,
        text: &str,
        reply_to_message_id: Option<i64>,
        message_thread_id: Option<i64>,
    ) -> Result<(), AdapterError> {
        self.send_message_returning_id(chat_id, text, reply_to_message_id, message_thread_id)
            .await
            .map(|_| ())
    }

    /// Send a text message and return the first Telegram `message_id`.
    pub async fn send_message_returning_id(
        &self,
        chat_id: &str,
        text: &str,
        reply_to_message_id: Option<i64>,
        message_thread_id: Option<i64>,
    ) -> Result<i64, AdapterError> {
        let chunks = chunk_text(text, MAX_MESSAGE_LENGTH);
        let mut first_message_id = None;

        for (i, chunk) in chunks.iter().enumerate() {
            // Only reply to the original message on the first chunk
            let reply_id = if i == 0 { reply_to_message_id } else { None };

            let mut body = serde_json::json!({
                "chat_id": chat_id,
                "text": chunk,
                "parse_mode": "Markdown",
            });

            if let Some(id) = reply_id {
                body["reply_parameters"] = serde_json::json!({ "message_id": id });
            }

            if let Some(topic_id) = message_thread_id {
                body["message_thread_id"] = serde_json::json!(topic_id);
            }

            let resp = self
                .client
                .post(format!("{}/sendMessage", self.base_url))
                .json(&body)
                .send()
                .await
                .map_err(|e| SharedAdapterError::provider_unreachable("Telegram sendMessage", e))?;

            if resp.status().as_u16() == 400 {
                // Markdown parse error — retry without parse_mode
                let resp_text = resp.text().await.unwrap_or_default();
                if resp_text.contains("can't parse entities") {
                    warn!("Telegram Markdown parse error, retrying as plain text");
                    let mut plain_body = serde_json::json!({
                        "chat_id": chat_id,
                        "text": chunk,
                    });
                    if let Some(id) = reply_id {
                        plain_body["reply_parameters"] = serde_json::json!({ "message_id": id });
                    }
                    if let Some(topic_id) = message_thread_id {
                        plain_body["message_thread_id"] = serde_json::json!(topic_id);
                    }
                    let retry_resp = self
                        .client
                        .post(format!("{}/sendMessage", self.base_url))
                        .json(&plain_body)
                        .send()
                        .await
                        .map_err(|e| {
                            SharedAdapterError::provider_unreachable(
                                "Telegram sendMessage retry",
                                e,
                            )
                        })?;
                    if !retry_resp.status().is_success() {
                        let status = retry_resp.status();
                        let text = retry_resp.text().await.unwrap_or_default();
                        warn!(status = %status, body = %text, "Telegram sendMessage error (plain text retry)");
                        return Err(SharedAdapterError::AdapterApi(format!(
                            "Telegram sendMessage returned {status}: {text}"
                        ))
                        .into());
                    }
                    let message_id = parse_send_message_id(retry_resp).await?;
                    first_message_id.get_or_insert(message_id);
                    continue;
                }
                warn!(status = 400, body = %resp_text, "Telegram sendMessage error");
                return Err(SharedAdapterError::AdapterApi(format!(
                    "Telegram sendMessage returned 400 Bad Request: {resp_text}"
                ))
                .into());
            }

            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                warn!(status = %status, body = %text, "Telegram sendMessage error");
                return Err(SharedAdapterError::AdapterApi(format!(
                    "Telegram sendMessage returned {status}: {text}"
                ))
                .into());
            }

            let message_id = parse_send_message_id(resp).await?;
            first_message_id.get_or_insert(message_id);
        }

        first_message_id.ok_or_else(|| {
            SharedAdapterError::AdapterApi("Telegram sendMessage returned no message_id".into())
                .into()
        })
    }

    /// Delete any existing webhook. Required before switching to polling mode,
    /// because Telegram rejects `getUpdates` while a webhook is set.
    pub async fn delete_webhook(&self) -> Result<(), AdapterError> {
        let resp = self
            .client
            .post(format!("{}/deleteWebhook", self.base_url))
            .send()
            .await
            .map_err(|e| SharedAdapterError::provider_unreachable("Telegram deleteWebhook", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(SharedAdapterError::AdapterApi(format!(
                "Telegram deleteWebhook returned {status}: {text}"
            ))
            .into());
        }

        Ok(())
    }

    /// Long-poll for new updates via Telegram's `getUpdates` API.
    ///
    /// `offset` is `last_update_id + 1`, acknowledging previous updates;
    /// returns the raw `result` array so callers can feed each element into
    /// [`super::ingress::receive`].
    pub async fn get_updates(
        &self,
        offset: Option<i64>,
        timeout: u32,
    ) -> Result<Vec<serde_json::Value>, AdapterError> {
        let mut body = serde_json::json!({
            "timeout": timeout,
            "allowed_updates": ["message", "channel_post"],
        });
        if let Some(off) = offset {
            body["offset"] = serde_json::json!(off);
        }

        let resp = self
            .client
            .post(format!("{}/getUpdates", self.base_url))
            .json(&body)
            // Client-side timeout must exceed the long-poll timeout
            .timeout(Duration::from_secs(u64::from(timeout) + 10))
            .send()
            .await
            .map_err(|e| SharedAdapterError::provider_unreachable("Telegram getUpdates", e))?;

        let status = resp.status();
        if status.as_u16() == 409 {
            return Err(SharedAdapterError::AdapterApi(
                "Telegram getUpdates 409 Conflict: another bot instance is already polling".into(),
            )
            .into());
        }
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(SharedAdapterError::AdapterApi(format!(
                "Telegram getUpdates returned {status}: {text}"
            ))
            .into());
        }

        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| SharedAdapterError::provider_response_invalid("Telegram getUpdates", e))?;

        let updates = json
            .get("result")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        Ok(updates)
    }

    /// Set (or clear) a reaction on a Telegram message via
    /// `setMessageReaction`.
    ///
    /// Telegram's API is an idempotent set, not an add/remove pair — it
    /// replaces the entire reaction list; `Some(emoji)` sets that one, `None`
    /// clears all.
    pub async fn set_message_reaction(
        &self,
        chat_id: &str,
        message_id: i64,
        emoji: Option<&str>,
    ) -> Result<(), AdapterError> {
        let reaction = match emoji {
            Some(e) => serde_json::json!([{ "type": "emoji", "emoji": e }]),
            None => serde_json::json!([]),
        };
        let body = serde_json::json!({
            "chat_id": chat_id,
            "message_id": message_id,
            "reaction": reaction,
        });
        let resp = self
            .client
            .post(format!("{}/setMessageReaction", self.base_url))
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                SharedAdapterError::provider_unreachable("Telegram setMessageReaction", e)
            })?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            warn!(status = %status, body = %text, "Telegram setMessageReaction error");
            return Err(SharedAdapterError::AdapterApi(format!(
                "Telegram setMessageReaction returned {status}: {text}"
            ))
            .into());
        }
        Ok(())
    }
}

async fn parse_send_message_id(resp: reqwest::Response) -> Result<i64, AdapterError> {
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| SharedAdapterError::provider_response_invalid("Telegram sendMessage", e))?;
    body.pointer("/result/message_id")
        .and_then(serde_json::Value::as_i64)
        .ok_or_else(|| {
            SharedAdapterError::AdapterApi("Telegram sendMessage: missing message_id".into()).into()
        })
}

/// Split text into chunks of at most `max_len` characters, breaking at newlines
/// when possible.
fn chunk_text(text: &str, max_len: usize) -> Vec<&str> {
    if text.len() <= max_len {
        return vec![text];
    }

    let mut chunks = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        if remaining.len() <= max_len {
            chunks.push(remaining);
            break;
        }

        // Find last newline within max_len
        let split_at = remaining[..max_len]
            .rfind('\n')
            .map_or(max_len, |pos| pos + 1);

        chunks.push(&remaining[..split_at]);
        remaining = &remaining[split_at..];
    }

    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_short_text() {
        let chunks = chunk_text("hello", 4096);
        assert_eq!(chunks, vec!["hello"]);
    }

    #[test]
    fn chunk_long_text_at_newline() {
        let text = format!("{}\n{}", "a".repeat(10), "b".repeat(10));
        let chunks = chunk_text(&text, 15);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0], format!("{}\n", "a".repeat(10)));
        assert_eq!(chunks[1], "b".repeat(10));
    }

    #[test]
    fn chunk_long_text_no_newline() {
        let text = "a".repeat(100);
        let chunks = chunk_text(&text, 30);
        assert_eq!(chunks.len(), 4); // 30 + 30 + 30 + 10
        assert_eq!(chunks[0].len(), 30);
        assert_eq!(chunks[3].len(), 10);
    }
}

#[cfg(test)]
mod error_source_tests {
    use std::error::Error as _;

    use axum::http::StatusCode;

    use super::*;
    use moatless_adapter_model::test_support::{UNREACHABLE_BASE, stub_provider};

    #[tokio::test]
    async fn unreachable_telegram_keeps_transport_source() {
        let api = TelegramApi::with_base_url(UNREACHABLE_BASE);

        let err = api
            .send_message_returning_id("42", "hello", None, None)
            .await
            .expect_err("dead loopback port must fail");

        let AdapterError::Shared(SharedAdapterError::ProviderUnreachable {
            ref operation,
            ref source,
        }) = err
        else {
            panic!("expected ProviderUnreachable, got {err:?}");
        };
        assert_eq!(operation, "Telegram sendMessage");
        assert!(source.is_connect() || source.is_request(), "{source}");
        assert!(err.source().is_some(), "cause must stay reachable");
    }

    #[tokio::test]
    async fn undecodable_telegram_body_keeps_decode_source() {
        let base = stub_provider(StatusCode::OK, "not json at all").await;
        let api = TelegramApi::with_base_url(base);

        let err = api
            .send_message_returning_id("42", "hello", None, None)
            .await
            .expect_err("plain-text body must not decode");

        let AdapterError::Shared(SharedAdapterError::ProviderResponseInvalid {
            ref operation, ..
        }) = err
        else {
            panic!("expected ProviderResponseInvalid, got {err:?}");
        };
        assert_eq!(operation, "Telegram sendMessage");
        assert!(err.source().is_some(), "cause must stay reachable");
    }

    /// Telegram answering a non-2xx is a rejection, not a wrapped failure.
    #[tokio::test]
    async fn telegram_rejection_stays_a_synthesized_message() {
        let base = stub_provider(StatusCode::FORBIDDEN, r#"{"ok":false}"#).await;
        let api = TelegramApi::with_base_url(base);

        let err = api
            .send_message_returning_id("42", "hello", None, None)
            .await
            .expect_err("403 must be an error");

        assert!(
            matches!(err, AdapterError::Shared(SharedAdapterError::AdapterApi(_))),
            "got {err:?}"
        );
        assert!(err.source().is_none());
    }
}
