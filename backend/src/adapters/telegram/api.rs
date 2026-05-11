//! Telegram Bot API client.

use reqwest::Client;
use tracing::warn;

use crate::adapters::error::AdapterError;

/// Maximum message length for Telegram's sendMessage API.
const MAX_MESSAGE_LENGTH: usize = 4096;

/// Telegram Bot API client.
pub struct TelegramApi {
    client: Client,
    base_url: String,
}

impl TelegramApi {
    pub fn new(bot_token: &str) -> Self {
        Self {
            client: Client::new(),
            base_url: format!("https://api.telegram.org/bot{bot_token}"),
        }
    }

    /// Send a text message, chunking if it exceeds 4096 characters.
    ///
    /// `message_thread_id` is used for forum/topic groups — when set, the message
    /// is posted into the specified forum topic.
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
                .map_err(|e| {
                    AdapterError::AdapterApi(format!("Telegram sendMessage failed: {e}"))
                })?;

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
                            AdapterError::AdapterApi(format!(
                                "Telegram sendMessage retry failed: {e}"
                            ))
                        })?;
                    if !retry_resp.status().is_success() {
                        let status = retry_resp.status();
                        let text = retry_resp.text().await.unwrap_or_default();
                        warn!(status = %status, body = %text, "Telegram sendMessage error (plain text retry)");
                        return Err(AdapterError::AdapterApi(format!(
                            "Telegram sendMessage returned {status}: {text}"
                        )));
                    }
                    let message_id = parse_send_message_id(retry_resp).await?;
                    first_message_id.get_or_insert(message_id);
                    continue;
                }
                warn!(status = 400, body = %resp_text, "Telegram sendMessage error");
                return Err(AdapterError::AdapterApi(format!(
                    "Telegram sendMessage returned 400 Bad Request: {resp_text}"
                )));
            }

            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                warn!(status = %status, body = %text, "Telegram sendMessage error");
                return Err(AdapterError::AdapterApi(format!(
                    "Telegram sendMessage returned {status}: {text}"
                )));
            }

            let message_id = parse_send_message_id(resp).await?;
            first_message_id.get_or_insert(message_id);
        }

        first_message_id.ok_or_else(|| {
            AdapterError::AdapterApi("Telegram sendMessage returned no message_id".into())
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
            .map_err(|e| AdapterError::AdapterApi(format!("Telegram deleteWebhook failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(AdapterError::AdapterApi(format!(
                "Telegram deleteWebhook returned {status}: {text}"
            )));
        }

        Ok(())
    }

    /// Long-poll for new updates via Telegram's `getUpdates` API.
    ///
    /// * `offset` — pass `last_update_id + 1` to acknowledge previous updates.
    /// * `timeout` — how many seconds Telegram should hold the connection open.
    ///
    /// Returns the raw `result` array from the response so callers can feed each
    /// element into `TelegramAdapter::parse_update()`.
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
            .timeout(std::time::Duration::from_secs(u64::from(timeout) + 10))
            .send()
            .await
            .map_err(|e| AdapterError::AdapterApi(format!("Telegram getUpdates failed: {e}")))?;

        let status = resp.status();
        if status.as_u16() == 409 {
            return Err(AdapterError::AdapterApi(
                "Telegram getUpdates 409 Conflict: another bot instance is already polling".into(),
            ));
        }
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(AdapterError::AdapterApi(format!(
                "Telegram getUpdates returned {status}: {text}"
            )));
        }

        let json: serde_json::Value = resp.json().await.map_err(|e| {
            AdapterError::AdapterApi(format!("Telegram getUpdates parse error: {e}"))
        })?;

        let updates = json
            .get("result")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        Ok(updates)
    }

    /// Send a chat action indicator (e.g., "typing").
    pub async fn send_chat_action(&self, chat_id: &str, action: &str) -> Result<(), AdapterError> {
        let body = serde_json::json!({
            "chat_id": chat_id,
            "action": action,
        });

        let resp = self
            .client
            .post(format!("{}/sendChatAction", self.base_url))
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                AdapterError::AdapterApi(format!("Telegram sendChatAction failed: {e}"))
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            warn!(status = %status, body = %text, "Telegram sendChatAction error");
        }

        Ok(())
    }

    /// Get chat info (for validation/discovery).
    pub async fn get_chat(&self, chat_id: &str) -> Result<serde_json::Value, AdapterError> {
        let resp = self
            .client
            .get(format!("{}/getChat", self.base_url))
            .query(&[("chat_id", chat_id)])
            .send()
            .await
            .map_err(|e| AdapterError::AdapterApi(format!("Telegram getChat failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(AdapterError::AdapterApi(format!(
                "Telegram getChat returned {status}: {text}"
            )));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| AdapterError::AdapterApi(format!("Telegram getChat parse error: {e}")))?;

        body.get("result")
            .cloned()
            .ok_or_else(|| AdapterError::AdapterApi("Telegram getChat: missing result".into()))
    }
}

async fn parse_send_message_id(resp: reqwest::Response) -> Result<i64, AdapterError> {
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| AdapterError::AdapterApi(format!("Telegram sendMessage parse error: {e}")))?;
    body.pointer("/result/message_id")
        .and_then(serde_json::Value::as_i64)
        .ok_or_else(|| AdapterError::AdapterApi("Telegram sendMessage: missing message_id".into()))
}

/// Split text into chunks of at most `max_len` characters, breaking at newlines when possible.
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
