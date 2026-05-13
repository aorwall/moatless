//! Slack Web API client — chat.postMessage, users.info, conversations.list/replies.

use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::adapters::error::AdapterError;

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
    name: Option<String>,
    is_bot: Option<bool>,
    is_app_user: Option<bool>,
    profile: Option<ProfileInfo>,
}

#[derive(Deserialize)]
struct ProfileInfo {
    email: Option<String>,
}

/// Per-user info returned by `users.info` — email plus bot flags.
#[derive(Debug, Clone)]
pub struct SlackUserInfo {
    pub email: Option<String>,
    pub is_bot: bool,
    pub is_app_user: bool,
    /// The Slack display name / handle (e.g. "soapbot").
    pub handle: Option<String>,
}

#[derive(Deserialize)]
struct ConversationsRepliesResponse {
    ok: bool,
    error: Option<String>,
    messages: Option<Vec<SlackMessage>>,
}

#[derive(Deserialize)]
struct AuthTestResponse {
    ok: bool,
    error: Option<String>,
    user_id: Option<String>,
    user: Option<String>,
    team: Option<String>,
    team_id: Option<String>,
}

/// Identity the current token authenticates as (from `auth.test`).
#[derive(Debug, Clone)]
pub struct AuthTestInfo {
    pub user_id: String,
    pub user: Option<String>,
    pub team: Option<String>,
    pub team_id: Option<String>,
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
    ) -> Result<Option<String>, AdapterError> {
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
            .map_err(|e| AdapterError::AdapterApi(format!("chat.postMessage failed: {e}")))?
            .json()
            .await
            .map_err(|e| AdapterError::AdapterApi(format!("chat.postMessage parse failed: {e}")))?;

        if !resp.ok {
            let err = resp.error.unwrap_or_default();
            warn!(error = %err, "Slack chat.postMessage failed");
            return Err(AdapterError::AdapterApi(format!("chat.postMessage: {err}")));
        }

        Ok(resp.ts)
    }

    /// Resolve the identity of the Slack token via `auth.test`.
    pub async fn auth_test(&self) -> Result<AuthTestInfo, AdapterError> {
        let resp: AuthTestResponse = self
            .client
            .post("https://slack.com/api/auth.test")
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|e| AdapterError::AdapterApi(format!("auth.test failed: {e}")))?
            .json()
            .await
            .map_err(|e| AdapterError::AdapterApi(format!("auth.test parse: {e}")))?;

        if !resp.ok {
            let err = resp.error.unwrap_or_default();
            return Err(AdapterError::AdapterApi(format!("auth.test: {err}")));
        }

        let user_id = resp
            .user_id
            .ok_or_else(|| AdapterError::AdapterApi("auth.test: missing user_id".into()))?;

        Ok(AuthTestInfo {
            user_id,
            user: resp.user,
            team: resp.team,
            team_id: resp.team_id,
        })
    }

    /// Fetch user info from Slack (email + bot flags).
    ///
    /// Returns `Ok(None)` when the API call fails or the user is not found.
    pub async fn get_user_info(
        &self,
        user_id: &str,
    ) -> Result<Option<SlackUserInfo>, AdapterError> {
        let resp: UserInfoResponse = self
            .client
            .get("https://slack.com/api/users.info")
            .bearer_auth(&self.token)
            .query(&[("user", user_id)])
            .send()
            .await
            .map_err(|e| AdapterError::AdapterApi(format!("users.info failed: {e}")))?
            .json()
            .await
            .map_err(|e| AdapterError::AdapterApi(format!("users.info parse: {e}")))?;

        if !resp.ok {
            let err = resp.error.unwrap_or_default();
            warn!(error = %err, "Slack users.info failed");
            return Ok(None);
        }

        Ok(resp.user.map(|u| SlackUserInfo {
            email: u.profile.and_then(|p| p.email),
            is_bot: u.is_bot.unwrap_or(false),
            is_app_user: u.is_app_user.unwrap_or(false),
            handle: u.name,
        }))
    }

    /// List Slack channels the bot is a member of.
    pub async fn list_conversations(&self) -> Result<Vec<SlackChannelInfo>, AdapterError> {
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
                .map_err(|e| AdapterError::AdapterApi(format!("conversations.list failed: {e}")))?
                .json()
                .await
                .map_err(|e| AdapterError::AdapterApi(format!("conversations.list parse: {e}")))?;

            if !resp.ok {
                let err = resp.error.unwrap_or_default();
                warn!(error = %err, "Slack conversations.list failed");
                return Err(AdapterError::AdapterApi(format!(
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
    ) -> Result<Vec<ThreadReply>, AdapterError> {
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
            .map_err(|e| AdapterError::AdapterApi(format!("conversations.replies failed: {e}")))?
            .json()
            .await
            .map_err(|e| AdapterError::AdapterApi(format!("conversations.replies parse: {e}")))?;

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
                let text = m
                    .blocks
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
    ) -> Result<(), AdapterError> {
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
                AdapterError::AdapterApi(format!("files.getUploadURLExternal failed: {e}"))
            })?
            .json()
            .await
            .map_err(|e| {
                AdapterError::AdapterApi(format!("files.getUploadURLExternal parse failed: {e}"))
            })?;

        if !resp.ok {
            let err = resp.error.unwrap_or_default();
            return Err(AdapterError::AdapterApi(format!(
                "files.getUploadURLExternal: {err}"
            )));
        }

        let upload_url = resp.upload_url.ok_or_else(|| {
            AdapterError::AdapterApi("files.getUploadURLExternal: no upload_url".into())
        })?;
        let slack_file_id = resp.file_id.ok_or_else(|| {
            AdapterError::AdapterApi("files.getUploadURLExternal: no file_id".into())
        })?;

        // Step 2: Upload file content to presigned URL
        let upload_status = self
            .client
            .post(&upload_url)
            .body(content)
            .send()
            .await
            .map_err(|e| {
                AdapterError::AdapterApi(format!("File upload to presigned URL failed: {e}"))
            })?
            .status();

        if !upload_status.is_success() {
            return Err(AdapterError::AdapterApi(format!(
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
                AdapterError::AdapterApi(format!("files.completeUploadExternal failed: {e}"))
            })?
            .json()
            .await
            .map_err(|e| {
                AdapterError::AdapterApi(format!("files.completeUploadExternal parse failed: {e}"))
            })?;

        if !complete_resp.ok {
            let err = complete_resp.error.unwrap_or_default();
            return Err(AdapterError::AdapterApi(format!(
                "files.completeUploadExternal: {err}"
            )));
        }

        Ok(())
    }

    /// Open a DM channel with a Slack user and return the channel ID.
    ///
    /// Calls `conversations.open` and returns the `channel.id` (starts with "D").
    pub async fn open_dm(&self, slack_user_id: &str) -> Result<String, AdapterError> {
        let body = serde_json::json!({ "users": slack_user_id });

        let resp: ConversationsOpenResponse = self
            .client
            .post("https://slack.com/api/conversations.open")
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .map_err(|e| AdapterError::AdapterApi(format!("conversations.open failed: {e}")))?
            .json()
            .await
            .map_err(|e| {
                AdapterError::AdapterApi(format!("conversations.open parse failed: {e}"))
            })?;

        if !resp.ok {
            let err = resp.error.unwrap_or_default();
            return Err(AdapterError::AdapterApi(format!(
                "conversations.open: {err}"
            )));
        }

        let channel = resp.channel.ok_or_else(|| {
            AdapterError::AdapterApi("conversations.open: missing channel in response".into())
        })?;

        Ok(channel.id)
    }

    /// Add an emoji reaction to a Slack message.
    ///
    /// `name` is a bare reaction name (e.g. `thumbsup`, not `:thumbsup:`).
    pub async fn add_reaction(
        &self,
        channel: &str,
        name: &str,
        timestamp: &str,
    ) -> Result<(), AdapterError> {
        self.reaction_op("reactions.add", channel, name, timestamp)
            .await
    }

    /// Remove an emoji reaction from a Slack message.
    pub async fn remove_reaction(
        &self,
        channel: &str,
        name: &str,
        timestamp: &str,
    ) -> Result<(), AdapterError> {
        self.reaction_op("reactions.remove", channel, name, timestamp)
            .await
    }

    async fn reaction_op(
        &self,
        method: &str,
        channel: &str,
        name: &str,
        timestamp: &str,
    ) -> Result<(), AdapterError> {
        let body = serde_json::json!({
            "channel": channel,
            "name": name,
            "timestamp": timestamp,
        });

        let resp: SimpleOkResponse = self
            .client
            .post(format!("https://slack.com/api/{method}"))
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .map_err(|e| AdapterError::AdapterApi(format!("{method} failed: {e}")))?
            .json()
            .await
            .map_err(|e| AdapterError::AdapterApi(format!("{method} parse failed: {e}")))?;

        if !resp.ok {
            let err = resp.error.unwrap_or_default();
            // Slack signals "no-op" cases (already_reacted, no_reaction) we want
            // to surface but not break the caller. Map them to a BadRequest so
            // the user gets a clear message.
            return Err(match err.as_str() {
                "already_reacted" => AdapterError::BadRequest(
                    "Slack: reaction already exists on this message".into(),
                ),
                "no_reaction" => AdapterError::BadRequest(
                    "Slack: no matching reaction to remove on this message".into(),
                ),
                "invalid_name" => {
                    AdapterError::BadRequest(format!("Slack: invalid reaction name `{name}`"))
                }
                _ => AdapterError::AdapterApi(format!("{method}: {err}")),
            });
        }

        Ok(())
    }

    /// Fetch info for a single Slack channel by ID.
    ///
    /// Returns `Ok(None)` when the API call fails (best-effort name resolution).
    pub async fn get_channel_info(
        &self,
        channel_id: &str,
    ) -> Result<Option<SlackChannelInfo>, AdapterError> {
        let resp: ConversationsInfoResponse = self
            .client
            .get("https://slack.com/api/conversations.info")
            .bearer_auth(&self.token)
            .query(&[("channel", channel_id)])
            .send()
            .await
            .map_err(|e| AdapterError::AdapterApi(format!("conversations.info failed: {e}")))?
            .json()
            .await
            .map_err(|e| AdapterError::AdapterApi(format!("conversations.info parse: {e}")))?;

        if !resp.ok {
            let err = resp.error.unwrap_or_default();
            warn!(error = %err, channel_id, "Slack conversations.info failed");
            return Ok(None);
        }

        Ok(resp.channel.map(|ch| SlackChannelInfo {
            id: ch.id,
            name: ch.name.unwrap_or_default(),
        }))
    }
}

#[derive(Deserialize)]
struct SimpleOkResponse {
    ok: bool,
    error: Option<String>,
}

#[derive(Deserialize)]
struct ConversationsInfoResponse {
    ok: bool,
    error: Option<String>,
    channel: Option<SlackChannelItem>,
}

#[derive(Deserialize)]
struct ConversationsOpenResponse {
    ok: bool,
    error: Option<String>,
    channel: Option<ConversationsOpenChannel>,
}

#[derive(Deserialize)]
struct ConversationsOpenChannel {
    id: String,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A realistic `users.info` response for a regular human — no bot flags,
    /// email present on the profile.
    #[test]
    fn user_info_human_parses_correctly() {
        let body = serde_json::json!({
            "ok": true,
            "user": {
                "id": "U123HUMAN",
                "name": "alice",
                "is_bot": false,
                "is_app_user": false,
                "profile": { "email": "alice@example.com" }
            }
        });
        let resp: UserInfoResponse = serde_json::from_value(body).unwrap();
        assert!(resp.ok);
        let info = SlackUserInfo {
            email: resp
                .user
                .as_ref()
                .and_then(|u| u.profile.as_ref())
                .and_then(|p| p.email.clone()),
            is_bot: resp.user.as_ref().and_then(|u| u.is_bot).unwrap_or(false),
            is_app_user: resp
                .user
                .as_ref()
                .and_then(|u| u.is_app_user)
                .unwrap_or(false),
            handle: resp.user.and_then(|u| u.name),
        };
        assert_eq!(info.email.as_deref(), Some("alice@example.com"));
        assert!(!info.is_bot);
        assert!(!info.is_app_user);
        assert_eq!(info.handle.as_deref(), Some("alice"));
    }

    /// A bot account response — `is_bot: true`, email may or may not be
    /// present. Callers MUST key off the bot flag, not absence of email.
    #[test]
    fn user_info_bot_account_reports_is_bot_true() {
        let body = serde_json::json!({
            "ok": true,
            "user": {
                "id": "U0000000003",
                "name": "soapbot",
                "is_bot": true,
                "is_app_user": false,
                "profile": { "email": "bot@example.com" }
            }
        });
        let resp: UserInfoResponse = serde_json::from_value(body).unwrap();
        let user = resp.user.expect("user present");
        assert_eq!(user.is_bot, Some(true));
        assert_eq!(user.is_app_user, Some(false));
        assert_eq!(user.name.as_deref(), Some("soapbot"));
    }

    /// Slack sometimes omits the bot flags for older accounts; treat missing
    /// as false.
    #[test]
    fn user_info_missing_flags_default_to_false() {
        let body = serde_json::json!({
            "ok": true,
            "user": {
                "id": "U999",
                "profile": {}
            }
        });
        let resp: UserInfoResponse = serde_json::from_value(body).unwrap();
        let user = resp.user.expect("user present");
        assert_eq!(user.is_bot, None);
        assert_eq!(user.is_app_user, None);
    }
}
