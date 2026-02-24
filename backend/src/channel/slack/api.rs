//! Slack Web API client — chat.postMessage, reactions.add/remove, users.info, conversations.list/replies.

use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::channel::error::ChannelError;

/// Slack API client wrapping reqwest.
#[derive(Clone)]
pub struct SlackApi {
    token: String,
    client: reqwest::Client,
}

#[derive(Deserialize)]
struct SlackResponse {
    ok: bool,
    error: Option<String>,
}

#[derive(Deserialize)]
struct UserInfoResponse {
    ok: bool,
    error: Option<String>,
    user: Option<UserProfile>,
}

#[derive(Deserialize)]
struct UserProfile {
    profile: Option<ProfileInfo>,
}

#[derive(Deserialize)]
struct ProfileInfo {
    email: Option<String>,
}

#[derive(Deserialize)]
struct ConversationsRepliesResponse {
    ok: bool,
    error: Option<String>,
    messages: Option<Vec<SlackMessage>>,
}

#[derive(Deserialize)]
struct SlackMessage {
    text: Option<String>,
    user: Option<String>,
    #[allow(dead_code)] // Deserialized from Slack API but not directly read
    ts: Option<String>,
}

#[derive(Deserialize)]
struct ConversationsListResponse {
    ok: bool,
    error: Option<String>,
    channels: Option<Vec<SlackChannelItem>>,
    response_metadata: Option<ResponseMetadata>,
}

#[derive(Deserialize)]
struct SlackChannelItem {
    id: String,
    name: Option<String>,
    is_member: Option<bool>,
}

#[derive(Deserialize)]
struct ResponseMetadata {
    next_cursor: Option<String>,
}

/// Slack channel info returned to callers.
#[derive(Debug, Clone, Serialize)]
pub struct SlackChannelInfo {
    pub id: String,
    pub name: String,
}

impl SlackApi {
    pub fn new(token: &str) -> Self {
        Self {
            token: token.to_string(),
            client: reqwest::Client::new(),
        }
    }

    /// Post a message to a Slack channel (optionally in a thread).
    pub async fn post_message(
        &self,
        channel: &str,
        text: &str,
        thread_ts: Option<&str>,
    ) -> Result<(), ChannelError> {
        let mut body = serde_json::json!({
            "channel": channel,
            "text": text,
        });
        if let Some(ts) = thread_ts {
            body["thread_ts"] = serde_json::Value::String(ts.to_string());
        }

        let resp: SlackResponse = self
            .client
            .post("https://slack.com/api/chat.postMessage")
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .map_err(|e| ChannelError::ProviderApi(format!("chat.postMessage failed: {e}")))?
            .json()
            .await
            .map_err(|e| {
                ChannelError::ProviderApi(format!("chat.postMessage parse failed: {e}"))
            })?;

        if !resp.ok {
            let err = resp.error.unwrap_or_default();
            warn!(error = %err, "Slack chat.postMessage failed");
            return Err(ChannelError::ProviderApi(format!(
                "chat.postMessage: {err}"
            )));
        }

        Ok(())
    }

    /// Add an emoji reaction to a message.
    pub async fn add_reaction(
        &self,
        channel: &str,
        timestamp: &str,
        emoji: &str,
    ) -> Result<(), ChannelError> {
        let body = serde_json::json!({
            "channel": channel,
            "timestamp": timestamp,
            "name": emoji,
        });

        let resp: SlackResponse = self
            .client
            .post("https://slack.com/api/reactions.add")
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .map_err(|e| ChannelError::ProviderApi(format!("reactions.add failed: {e}")))?
            .json()
            .await
            .map_err(|e| ChannelError::ProviderApi(format!("reactions.add parse: {e}")))?;

        if !resp.ok {
            let err = resp.error.unwrap_or_default();
            // "already_reacted" is not a real error
            if err != "already_reacted" {
                warn!(error = %err, "Slack reactions.add failed");
            }
        }

        Ok(())
    }

    /// Remove an emoji reaction from a message.
    pub async fn remove_reaction(
        &self,
        channel: &str,
        timestamp: &str,
        emoji: &str,
    ) -> Result<(), ChannelError> {
        let body = serde_json::json!({
            "channel": channel,
            "timestamp": timestamp,
            "name": emoji,
        });

        let resp: SlackResponse = self
            .client
            .post("https://slack.com/api/reactions.remove")
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .map_err(|e| ChannelError::ProviderApi(format!("reactions.remove failed: {e}")))?
            .json()
            .await
            .map_err(|e| ChannelError::ProviderApi(format!("reactions.remove parse: {e}")))?;

        if !resp.ok {
            let err = resp.error.unwrap_or_default();
            // "no_reaction" is not a real error
            if err != "no_reaction" {
                warn!(error = %err, "Slack reactions.remove failed");
            }
        }

        Ok(())
    }

    /// Get a user's email address from Slack.
    pub async fn get_user_email(&self, user_id: &str) -> Result<Option<String>, ChannelError> {
        let resp: UserInfoResponse = self
            .client
            .get("https://slack.com/api/users.info")
            .bearer_auth(&self.token)
            .query(&[("user", user_id)])
            .send()
            .await
            .map_err(|e| ChannelError::ProviderApi(format!("users.info failed: {e}")))?
            .json()
            .await
            .map_err(|e| ChannelError::ProviderApi(format!("users.info parse: {e}")))?;

        if !resp.ok {
            let err = resp.error.unwrap_or_default();
            warn!(error = %err, "Slack users.info failed");
            return Ok(None);
        }

        Ok(resp
            .user
            .and_then(|u| u.profile)
            .and_then(|p| p.email))
    }

    /// List Slack channels the bot is a member of.
    pub async fn list_conversations(&self) -> Result<Vec<SlackChannelInfo>, ChannelError> {
        let mut all_channels = Vec::new();
        let mut cursor: Option<String> = None;

        loop {
            let mut query = vec![
                ("types", "public_channel,private_channel"),
                ("exclude_archived", "true"),
                ("limit", "200"),
            ];
            let cursor_val;
            if let Some(ref c) = cursor {
                cursor_val = c.clone();
                query.push(("cursor", &cursor_val));
            }

            let resp: ConversationsListResponse = self
                .client
                .get("https://slack.com/api/conversations.list")
                .bearer_auth(&self.token)
                .query(&query)
                .send()
                .await
                .map_err(|e| {
                    ChannelError::ProviderApi(format!("conversations.list failed: {e}"))
                })?
                .json()
                .await
                .map_err(|e| {
                    ChannelError::ProviderApi(format!("conversations.list parse: {e}"))
                })?;

            if !resp.ok {
                let err = resp.error.unwrap_or_default();
                warn!(error = %err, "Slack conversations.list failed");
                return Err(ChannelError::ProviderApi(format!(
                    "conversations.list: {err}"
                )));
            }

            if let Some(channels) = resp.channels {
                for ch in channels {
                    // Only include channels the bot is a member of
                    if ch.is_member.unwrap_or(false) {
                        all_channels.push(SlackChannelInfo {
                            id: ch.id,
                            name: ch.name.unwrap_or_default(),
                        });
                    }
                }
            }

            // Handle pagination
            match resp.response_metadata.and_then(|m| m.next_cursor) {
                Some(c) if !c.is_empty() => cursor = Some(c),
                _ => break,
            }
        }

        all_channels.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(all_channels)
    }

    /// Fetch thread replies for context enrichment.
    pub async fn get_thread_replies(
        &self,
        channel: &str,
        thread_ts: &str,
        limit: usize,
        bot_user_id: Option<&str>,
    ) -> Result<Vec<String>, ChannelError> {
        let resp: ConversationsRepliesResponse = self
            .client
            .get("https://slack.com/api/conversations.replies")
            .bearer_auth(&self.token)
            .query(&[
                ("channel", channel),
                ("ts", thread_ts),
                ("limit", &limit.to_string()),
            ])
            .send()
            .await
            .map_err(|e| {
                ChannelError::ProviderApi(format!("conversations.replies failed: {e}"))
            })?
            .json()
            .await
            .map_err(|e| {
                ChannelError::ProviderApi(format!("conversations.replies parse: {e}"))
            })?;

        if !resp.ok {
            let err = resp.error.unwrap_or_default();
            warn!(error = %err, "Slack conversations.replies failed");
            return Ok(vec![]);
        }

        let messages = resp.messages.unwrap_or_default();
        let filtered: Vec<String> = messages
            .into_iter()
            .filter(|m| {
                // Exclude bot's own messages
                if let Some(ref uid) = m.user {
                    if let Some(bot_id) = bot_user_id {
                        return uid != bot_id;
                    }
                }
                true
            })
            .filter_map(|m| m.text)
            .collect();

        Ok(filtered)
    }
}
