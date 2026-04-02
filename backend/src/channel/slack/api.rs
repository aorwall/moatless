//! Slack Web API client — chat.postMessage, users.info, conversations.list/replies.

use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::channel::error::ChannelError;

/// Slack API client wrapping reqwest.
#[derive(Clone)]
pub struct SlackApi {
    token: String,
    client: reqwest::Client,
}

/// Response from `chat.postMessage` — includes the posted message's `ts`.
#[derive(Deserialize)]
struct PostMessageResponse {
    ok: bool,
    error: Option<String>,
    ts: Option<String>,
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
    ts: Option<String>,
    blocks: Option<Vec<serde_json::Value>>,
    attachments: Option<Vec<serde_json::Value>>,
}

/// A formatted thread reply with its Slack timestamp for dedup filtering.
pub struct ThreadReply {
    /// The formatted message text.
    pub text: String,
    /// The Slack message timestamp (unique ID).
    pub ts: Option<String>,
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
    ///
    /// Returns the posted message's `ts` if available.
    pub async fn post_message(
        &self,
        channel: &str,
        text: &str,
        thread_ts: Option<&str>,
    ) -> Result<Option<String>, ChannelError> {
        let mut body = serde_json::json!({
            "channel": channel,
            "text": text,
        });
        if let Some(ts) = thread_ts {
            body["thread_ts"] = serde_json::Value::String(ts.to_string());
        }

        let resp: PostMessageResponse = self
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

        Ok(resp.ts)
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
    ///
    /// Returns structured replies with Slack `ts` for filtering already-delivered messages.
    /// When `oldest` is provided, only messages with `ts` strictly greater than it are returned.
    pub async fn get_thread_replies(
        &self,
        channel: &str,
        thread_ts: &str,
        limit: usize,
        bot_user_id: Option<&str>,
        oldest: Option<&str>,
    ) -> Result<Vec<ThreadReply>, ChannelError> {
        let limit_str = limit.to_string();
        let mut query: Vec<(&str, &str)> = vec![
            ("channel", channel),
            ("ts", thread_ts),
            ("limit", &limit_str),
        ];
        if let Some(oldest_ts) = oldest {
            query.push(("oldest", oldest_ts));
        }

        let resp: ConversationsRepliesResponse = self
            .client
            .get("https://slack.com/api/conversations.replies")
            .bearer_auth(&self.token)
            .query(&query)
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
        let filtered: Vec<ThreadReply> = messages
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
            .filter_map(|m| {
                let ts = m.ts.clone();
                // Try blocks first (Block Kit), then attachments (legacy), then text (fallback)
                let text = m.blocks
                    .as_ref()
                    .map(|b| super::format::extract_text_from_blocks(b))
                    .filter(|t| !t.is_empty())
                    .or_else(|| {
                        m.attachments
                            .as_ref()
                            .map(|a| super::format::extract_text_from_attachments(a))
                            .filter(|t| !t.is_empty())
                    })
                    .or_else(|| {
                        m.text
                            .filter(|t| !t.is_empty())
                            .map(|t| super::format::format_slack_text(&t))
                    })?;
                Some(ThreadReply { text, ts })
            })
            .collect();

        Ok(filtered)
    }

    /// Upload a file to a Slack channel/thread using the V2 upload flow.
    ///
    /// 1. `files.getUploadURLExternal` — get a presigned upload URL from Slack
    /// 2. Upload file bytes to that URL
    /// 3. `files.completeUploadExternal` — finalize, associating with channel + thread
    pub async fn upload_file(
        &self,
        channel: &str,
        thread_ts: Option<&str>,
        filename: &str,
        content: Vec<u8>,
    ) -> Result<(), ChannelError> {
        let length_str = content.len().to_string();

        // Step 1: Get presigned upload URL
        let resp: GetUploadUrlResponse = self
            .client
            .get("https://slack.com/api/files.getUploadURLExternal")
            .bearer_auth(&self.token)
            .query(&[("filename", filename), ("length", &length_str)])
            .send()
            .await
            .map_err(|e| {
                ChannelError::ProviderApi(format!(
                    "files.getUploadURLExternal failed: {e}"
                ))
            })?
            .json()
            .await
            .map_err(|e| {
                ChannelError::ProviderApi(format!(
                    "files.getUploadURLExternal parse failed: {e}"
                ))
            })?;

        if !resp.ok {
            let err = resp.error.unwrap_or_default();
            return Err(ChannelError::ProviderApi(format!(
                "files.getUploadURLExternal: {err}"
            )));
        }

        let upload_url = resp.upload_url.ok_or_else(|| {
            ChannelError::ProviderApi("files.getUploadURLExternal: no upload_url".into())
        })?;
        let slack_file_id = resp.file_id.ok_or_else(|| {
            ChannelError::ProviderApi("files.getUploadURLExternal: no file_id".into())
        })?;

        // Step 2: Upload file content to presigned URL
        let upload_status = self
            .client
            .post(&upload_url)
            .body(content)
            .send()
            .await
            .map_err(|e| {
                ChannelError::ProviderApi(format!("File upload to presigned URL failed: {e}"))
            })?
            .status();

        if !upload_status.is_success() {
            return Err(ChannelError::ProviderApi(format!(
                "File upload returned HTTP {upload_status}"
            )));
        }

        // Step 3: Complete the upload, associating with channel + thread
        let mut complete_body = serde_json::json!({
            "files": [{ "id": slack_file_id, "title": filename }],
            "channel_id": channel,
        });
        if let Some(ts) = thread_ts {
            complete_body["thread_ts"] = serde_json::Value::String(ts.to_string());
        }

        let complete_resp: CompleteUploadResponse = self
            .client
            .post("https://slack.com/api/files.completeUploadExternal")
            .bearer_auth(&self.token)
            .json(&complete_body)
            .send()
            .await
            .map_err(|e| {
                ChannelError::ProviderApi(format!(
                    "files.completeUploadExternal failed: {e}"
                ))
            })?
            .json()
            .await
            .map_err(|e| {
                ChannelError::ProviderApi(format!(
                    "files.completeUploadExternal parse failed: {e}"
                ))
            })?;

        if !complete_resp.ok {
            let err = complete_resp.error.unwrap_or_default();
            return Err(ChannelError::ProviderApi(format!(
                "files.completeUploadExternal: {err}"
            )));
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Response types for file upload V2 flow
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct GetUploadUrlResponse {
    ok: bool,
    error: Option<String>,
    upload_url: Option<String>,
    file_id: Option<String>,
}

#[derive(Deserialize)]
struct CompleteUploadResponse {
    ok: bool,
    error: Option<String>,
}
