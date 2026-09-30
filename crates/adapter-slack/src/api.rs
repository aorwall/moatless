//! Slack Web API client — chat.postMessage, users.info,
//! conversations.list/replies.

use serde::Deserialize;
use tracing::warn;

use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
use moatless_types::adapter::SlackBlock;
use moatless_types::http::HttpTimeouts;

/// Slack API client wrapping reqwest.
#[derive(Clone)]
pub struct SlackApi {
    token: String,
    client: reqwest::Client,
    /// Web API root. A field rather than a constant so tests can drive the real
    /// request path against a local address.
    base_url: String,
}

/// Slack Web API root.
const SLACK_API_BASE: &str = "https://slack.com/api";

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
    #[allow(dead_code, reason = "logged with the resolved bot identity")]
    pub user: Option<String>,
    #[allow(dead_code, reason = "logged with the resolved bot identity")]
    pub team: Option<String>,
    pub team_id: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct SlackMessage {
    pub(crate) text: Option<String>,
    pub(crate) user: Option<String>,
    pub(crate) ts: Option<String>,
    pub(crate) blocks: Option<Vec<serde_json::Value>>,
    pub(crate) attachments: Option<Vec<serde_json::Value>>,
}

/// One `chat.postMessage` request.
///
/// `text` is always sent — Slack's notification/fallback text, and what
/// moatless persists as the task-timeline entry. `blocks` supersedes it for
/// channel display.
pub struct PostMessage<'a> {
    pub channel: &'a str,
    pub text: &'a str,
    pub thread_ts: Option<&'a str>,
    pub blocks: Option<&'a [SlackBlock]>,
}

impl PostMessage<'_> {
    fn has_blocks(&self) -> bool {
        self.blocks.is_some_and(|b| !b.is_empty())
    }

    fn body(&self, with_blocks: bool) -> serde_json::Value {
        let mut body = serde_json::json!({
            "channel": self.channel,
            "text": self.text,
        });
        if let Some(ts) = self.thread_ts {
            body["thread_ts"] = serde_json::Value::String(ts.to_string());
        }
        if with_blocks && self.has_blocks() {
            match serde_json::to_value(self.blocks) {
                Ok(v) => body["blocks"] = v,
                Err(e) => warn!(error = %e, "dropping unserialisable Slack blocks"),
            }
        }
        body
    }
}

#[expect(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "sets the timeouts `Config::http` holds, and fails as `Client::new` would"
)]
fn http_client(timeouts: HttpTimeouts) -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(timeouts.connect)
        .timeout(timeouts.total)
        .build()
        .expect("the TLS backend initialises")
}

impl SlackApi {
    pub fn new(token: &str, timeouts: HttpTimeouts) -> Self {
        Self {
            token: token.to_string(),
            client: http_client(timeouts),
            base_url: SLACK_API_BASE.to_string(),
        }
    }

    /// The same client and root, calling as `token`.
    pub(crate) fn with_token(&self, token: &str) -> Self {
        Self {
            token: token.to_string(),
            ..self.clone()
        }
    }

    #[cfg(test)]
    pub(crate) fn with_base_url(token: &str, base_url: impl Into<String>) -> Self {
        Self {
            token: token.to_string(),
            client: http_client(moatless_core::config::HttpConfig::default().slack),
            base_url: base_url.into(),
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
        self.post_message_with(PostMessage {
            channel,
            text,
            thread_ts,
            blocks: None,
        })
        .await
    }

    /// Post a message that may carry Block Kit blocks.
    ///
    /// When Slack rejects the blocks, the message is reposted once without
    /// them: a bad block costs formatting, not the reply's existence.
    pub async fn post_message_with(
        &self,
        msg: PostMessage<'_>,
    ) -> Result<Option<String>, AdapterError> {
        let resp = self.post_once(&msg.body(true)).await?;

        let resp = match (resp.ok, resp.error.as_deref()) {
            (false, Some("invalid_blocks")) if msg.has_blocks() => {
                warn!("Slack rejected the blocks; reposting as text only");
                self.post_once(&msg.body(false)).await?
            }
            _ => resp,
        };

        if !resp.ok {
            let err = resp.error.unwrap_or_default();
            warn!(error = %err, "Slack chat.postMessage failed");
            return Err(SharedAdapterError::AdapterApi(format!("chat.postMessage: {err}")).into());
        }

        Ok(resp.ts)
    }

    /// One `chat.postMessage` round trip. Returns the parsed response even when
    /// Slack reports failure, so the caller can decide whether to retry.
    async fn post_once(
        &self,
        body: &serde_json::Value,
    ) -> Result<PostMessageResponse, AdapterError> {
        self.client
            .post(format!("{}/chat.postMessage", self.base_url))
            .bearer_auth(&self.token)
            .json(body)
            .send()
            .await
            .map_err(|e| SharedAdapterError::provider_unreachable("chat.postMessage", e))?
            .json()
            .await
            .map_err(|e| {
                SharedAdapterError::provider_response_invalid("chat.postMessage", e).into()
            })
    }

    /// Resolve the identity of the Slack token via `auth.test`.
    pub async fn auth_test(&self) -> Result<AuthTestInfo, AdapterError> {
        let resp: AuthTestResponse = self
            .client
            .post(format!("{}/auth.test", self.base_url))
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|e| SharedAdapterError::provider_unreachable("auth.test", e))?
            .json()
            .await
            .map_err(|e| SharedAdapterError::provider_response_invalid("auth.test", e))?;

        if !resp.ok {
            let err = resp.error.unwrap_or_default();
            return Err(SharedAdapterError::AdapterApi(format!("auth.test: {err}")).into());
        }

        let user_id = resp
            .user_id
            .ok_or_else(|| SharedAdapterError::AdapterApi("auth.test: missing user_id".into()))?;

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
            .get(format!("{}/users.info", self.base_url))
            .bearer_auth(&self.token)
            .query(&[("user", user_id)])
            .send()
            .await
            .map_err(|e| SharedAdapterError::provider_unreachable("users.info", e))?
            .json()
            .await
            .map_err(|e| SharedAdapterError::provider_response_invalid("users.info", e))?;

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

    /// Fetch thread replies for context enrichment.
    ///
    /// Returns structured replies with Slack `ts` for filtering
    /// already-delivered messages; `oldest`, when given, returns only `ts`
    /// strictly greater than it.
    pub async fn get_thread_replies(
        &self,
        channel: &str,
        thread_ts: &str,
        limit: usize,
        oldest: Option<&str>,
    ) -> Result<Vec<SlackMessage>, AdapterError> {
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
            .get(format!("{}/conversations.replies", self.base_url))
            .bearer_auth(&self.token)
            .query(&query)
            .send()
            .await
            .map_err(|e| SharedAdapterError::provider_unreachable("conversations.replies", e))?
            .json()
            .await
            .map_err(|e| {
                SharedAdapterError::provider_response_invalid("conversations.replies", e)
            })?;

        if !resp.ok {
            let err = resp.error.unwrap_or_default();
            warn!(error = %err, "Slack conversations.replies failed");
            return Ok(vec![]);
        }

        Ok(resp.messages.unwrap_or_default())
    }

    /// Upload a file to a Slack channel/thread via the V2 upload flow: get a
    /// presigned URL (`files.getUploadURLExternal`), upload the bytes, then
    /// finalize with `files.completeUploadExternal`.
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
            .get(format!("{}/files.getUploadURLExternal", self.base_url))
            .bearer_auth(&self.token)
            .query(&[("filename", filename), ("length", &length_str)])
            .send()
            .await
            .map_err(|e| SharedAdapterError::provider_unreachable("files.getUploadURLExternal", e))?
            .json()
            .await
            .map_err(|e| {
                SharedAdapterError::provider_response_invalid("files.getUploadURLExternal", e)
            })?;

        if !resp.ok {
            let err = resp.error.unwrap_or_default();
            return Err(SharedAdapterError::AdapterApi(format!(
                "files.getUploadURLExternal: {err}"
            ))
            .into());
        }

        let upload_url = resp.upload_url.ok_or_else(|| {
            SharedAdapterError::AdapterApi("files.getUploadURLExternal: no upload_url".into())
        })?;
        let slack_file_id = resp.file_id.ok_or_else(|| {
            SharedAdapterError::AdapterApi("files.getUploadURLExternal: no file_id".into())
        })?;

        // Step 2: Upload file content to presigned URL
        let upload_status = self
            .client
            .post(&upload_url)
            .body(content)
            .send()
            .await
            .map_err(|e| {
                SharedAdapterError::provider_unreachable("File upload to presigned URL", e)
            })?
            .status();

        if !upload_status.is_success() {
            return Err(SharedAdapterError::AdapterApi(format!(
                "File upload returned HTTP {upload_status}"
            ))
            .into());
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
            .post(format!("{}/files.completeUploadExternal", self.base_url))
            .bearer_auth(&self.token)
            .json(&complete_body)
            .send()
            .await
            .map_err(|e| {
                SharedAdapterError::provider_unreachable("files.completeUploadExternal", e)
            })?
            .json()
            .await
            .map_err(|e| {
                SharedAdapterError::provider_response_invalid("files.completeUploadExternal", e)
            })?;

        if !complete_resp.ok {
            let err = complete_resp.error.unwrap_or_default();
            return Err(SharedAdapterError::AdapterApi(format!(
                "files.completeUploadExternal: {err}"
            ))
            .into());
        }

        Ok(())
    }

    /// Open a DM channel with a Slack user and return the channel ID.
    ///
    /// Calls `conversations.open` and returns the `channel.id` (starts with
    /// "D").
    pub async fn open_dm(&self, slack_user_id: &str) -> Result<String, AdapterError> {
        let body = serde_json::json!({ "users": slack_user_id });

        let resp: ConversationsOpenResponse = self
            .client
            .post(format!("{}/conversations.open", self.base_url))
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .map_err(|e| SharedAdapterError::provider_unreachable("conversations.open", e))?
            .json()
            .await
            .map_err(|e| SharedAdapterError::provider_response_invalid("conversations.open", e))?;

        if !resp.ok {
            let err = resp.error.unwrap_or_default();
            return Err(
                SharedAdapterError::AdapterApi(format!("conversations.open: {err}")).into(),
            );
        }

        let channel = resp.channel.ok_or_else(|| {
            SharedAdapterError::AdapterApi("conversations.open: missing channel in response".into())
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
            .post(format!("{}/{method}", self.base_url))
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .map_err(|e| SharedAdapterError::provider_unreachable(method, e))?
            .json()
            .await
            .map_err(|e| SharedAdapterError::provider_response_invalid(method, e))?;

        if !resp.ok {
            let err = resp.error.unwrap_or_default();
            // Slack signals "no-op" cases (already_reacted, no_reaction) we want
            // to surface but not break the caller. Map them to a BadRequest so
            // the user gets a clear message.
            return Err(match err.as_str() {
                "already_reacted" => SharedAdapterError::BadRequest(
                    "Slack: reaction already exists on this message".into(),
                )
                .into(),
                "no_reaction" => SharedAdapterError::BadRequest(
                    "Slack: no matching reaction to remove on this message".into(),
                )
                .into(),
                "invalid_name" => {
                    SharedAdapterError::BadRequest(format!("Slack: invalid reaction name `{name}`"))
                        .into()
                }
                _ => SharedAdapterError::AdapterApi(format!("{method}: {err}")).into(),
            });
        }

        Ok(())
    }
}

#[derive(Deserialize)]
struct SimpleOkResponse {
    ok: bool,
    error: Option<String>,
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

    /// Slack sometimes omits the bot flags for older accounts; treat missing as
    /// false.
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

#[cfg(test)]
mod error_source_tests {
    use std::error::Error as _;

    use axum::http::StatusCode;

    use super::*;
    use moatless_adapter_model::test_support::{UNREACHABLE_BASE, stub_provider};

    #[tokio::test]
    async fn unreachable_slack_keeps_transport_source() {
        let api = SlackApi::with_base_url("xoxb-test", UNREACHABLE_BASE);

        let err = api
            .post_message("C1", "hello", None)
            .await
            .expect_err("dead loopback port must fail");

        let AdapterError::Shared(SharedAdapterError::ProviderUnreachable {
            ref operation,
            ref source,
        }) = err
        else {
            panic!("expected ProviderUnreachable, got {err:?}");
        };
        assert_eq!(operation, "chat.postMessage");
        assert!(source.is_connect() || source.is_request(), "{source}");
        assert!(err.source().is_some(), "cause must stay reachable");
    }

    #[tokio::test]
    async fn undecodable_slack_body_keeps_decode_source() {
        let base = stub_provider(StatusCode::OK, "<html>not json</html>").await;
        let api = SlackApi::with_base_url("xoxb-test", base);

        let err = api
            .post_message("C1", "hello", None)
            .await
            .expect_err("html body must not decode");

        let AdapterError::Shared(SharedAdapterError::ProviderResponseInvalid {
            ref operation, ..
        }) = err
        else {
            panic!("expected ProviderResponseInvalid, got {err:?}");
        };
        assert_eq!(operation, "chat.postMessage");
        assert!(err.source().is_some(), "cause must stay reachable");
    }

    /// Slack answering `ok: false` is a rejection, not a wrapped failure — it
    /// must stay classified apart from the transport variants.
    #[tokio::test]
    async fn slack_rejection_stays_a_synthesized_message() {
        let base = stub_provider(
            StatusCode::OK,
            r#"{"ok":false,"error":"channel_not_found"}"#,
        )
        .await;
        let api = SlackApi::with_base_url("xoxb-test", base);

        let err = api
            .post_message("C1", "hello", None)
            .await
            .expect_err("ok:false must be an error");

        assert!(
            matches!(err, AdapterError::Shared(SharedAdapterError::AdapterApi(ref m)) if m == "chat.postMessage: channel_not_found"),
            "got {err:?}"
        );
        assert!(err.source().is_none());
    }
}
