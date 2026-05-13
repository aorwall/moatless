//! Linear Agents GraphQL client.
//!
//! Outbound surface for the Linear Agents flow:
//! - `agent_activity_create`: emit `thought` / `response` / `elicitation` /
//!   `error` activities on a session. Linear updates the session state from
//!   these.
//! - `agent_session_update`: set `externalUrls` so the Moatless task is
//!   linked from the Linear Agent UI.
//! - `viewer`: identify the authenticated agent (used at OAuth install time).
//!
//! Auth: workspace-scoped OAuth tokens persisted in `provider_tokens` with
//! `team_id = <linear org uuid>`. Expired tokens are transparently refreshed
//! against `https://api.linear.app/oauth/token` (form-encoded).

use std::time::Duration;

use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::Deserialize;
use tracing::{info, warn};

use crate::adapters::error::AdapterError;
use crate::auth::provider_token_dao::{PgProviderTokenRepository, ProviderTokenRepository};
use crate::db::DbPool;

const LINEAR_GRAPHQL_URL: &str = "https://api.linear.app/graphql";
const LINEAR_TOKEN_URL: &str = "https://api.linear.app/oauth/token";
const PROVIDER: &str = "linear";
/// Refresh tokens whose expiry is within this window.
const REFRESH_LEEWAY_SECS: i64 = 5 * 60;

/// Authenticated viewer information (agent identity from OAuth).
#[derive(Debug, Clone, Deserialize)]
pub struct ViewerInfo {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub email: Option<String>,
}

/// Linear `AgentActivityType`. Plain strings on the wire — we only model the
/// variants we emit.
#[derive(Debug, Clone, Copy)]
pub enum ActivityKind {
    Thought,
    Response,
    Elicitation,
    Error,
}

impl ActivityKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Thought => "thought",
            Self::Response => "response",
            Self::Elicitation => "elicitation",
            Self::Error => "error",
        }
    }
}

/// GraphQL client for Linear Agents APIs.
///
/// Constructed per-org so the workspace token lookup is bound to a single
/// connection. Cheap to clone — the underlying `reqwest::Client` and DB
/// pool are both `Arc`-shared.
#[derive(Clone)]
pub struct LinearAgentApi {
    http: Client,
    db: DbPool,
    /// Linear organization (workspace) UUID. Used as `team_id` in
    /// `provider_tokens` for the OAuth row.
    org_id: String,
    /// OAuth client_id / client_secret for the refresh-token grant.
    oauth_client_id: Option<String>,
    oauth_client_secret: Option<String>,
}

impl LinearAgentApi {
    pub fn new(
        http: Client,
        db: DbPool,
        org_id: String,
        oauth_client_id: Option<String>,
        oauth_client_secret: Option<String>,
    ) -> Self {
        Self {
            http,
            db,
            org_id,
            oauth_client_id,
            oauth_client_secret,
        }
    }

    /// Resolve a fresh access token for this organization, refreshing if the
    /// stored token is within `REFRESH_LEEWAY_SECS` of expiry.
    pub async fn resolve_access_token(&self) -> Result<String, AdapterError> {
        let token_dao = PgProviderTokenRepository(self.db.clone());

        let full = sqlx::query_as::<
            _,
            (
                String,
                Option<String>,
                Option<DateTime<Utc>>,
                Option<serde_json::Value>,
            ),
        >(
            "SELECT access_token, refresh_token, expires_at, metadata \
             FROM provider_tokens \
             WHERE team_id = $1 AND provider = $2 AND user_id IS NULL \
             LIMIT 1",
        )
        .bind(&self.org_id)
        .bind(PROVIDER)
        .fetch_optional(&self.db)
        .await
        .map_err(AdapterError::Db)?;

        let (access_token, refresh_token, expires_at, metadata) =
            full.ok_or_else(|| {
                AdapterError::AdapterApi(format!(
                    "No Linear OAuth token registered for workspace {} — \
                     run `moat adapters install --adapter linear` first",
                    self.org_id
                ))
            })?;

        if !needs_refresh(expires_at) {
            return Ok(access_token);
        }

        let Some(refresh_token) = refresh_token else {
            // No refresh token available — return what we have and let the
            // caller surface a 401 if expired.
            warn!(
                org_id = %self.org_id,
                "Linear access token near expiry but no refresh token stored"
            );
            return Ok(access_token);
        };

        info!(org_id = %self.org_id, "Refreshing Linear OAuth access token");
        let refreshed = self.refresh_tokens(&refresh_token).await?;

        let new_expires_at = refreshed
            .expires_in
            .map(|secs| Utc::now() + chrono::Duration::seconds(secs as i64));

        let new_refresh = refreshed
            .refresh_token
            .clone()
            .or(Some(refresh_token.clone()));

        let new_metadata = metadata
            .or_else(|| Some(serde_json::json!({})))
            .map(|mut v| {
                if let Some(obj) = v.as_object_mut() {
                    obj.insert("refreshed_at".into(), serde_json::json!(Utc::now()));
                    if let Some(ref scope) = refreshed.scope {
                        obj.insert("scope".into(), serde_json::json!(scope));
                    }
                }
                v
            });

        token_dao
            .upsert_team_token(
                &self.org_id,
                PROVIDER,
                &refreshed.access_token,
                new_refresh.as_deref(),
                refreshed.token_type.as_deref(),
                new_metadata.as_ref(),
            )
            .await
            .map_err(AdapterError::Db)?;

        if let Some(at) = new_expires_at {
            sqlx::query(
                "UPDATE provider_tokens SET expires_at = $1 \
                 WHERE team_id = $2 AND provider = $3 AND user_id IS NULL",
            )
            .bind(at)
            .bind(&self.org_id)
            .bind(PROVIDER)
            .execute(&self.db)
            .await
            .map_err(AdapterError::Db)?;
        }

        Ok(refreshed.access_token)
    }

    /// Exchange a refresh token for a new access token.
    async fn refresh_tokens(&self, refresh_token: &str) -> Result<TokenResponse, AdapterError> {
        let client_id = self.oauth_client_id.as_deref().ok_or_else(|| {
            AdapterError::AdapterApi(
                "LINEAR_OAUTH_CLIENT_ID is not configured; cannot refresh Linear token".into(),
            )
        })?;
        let client_secret = self.oauth_client_secret.as_deref().ok_or_else(|| {
            AdapterError::AdapterApi(
                "LINEAR_OAUTH_CLIENT_SECRET is not configured; cannot refresh Linear token".into(),
            )
        })?;

        let form = [
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", client_id),
            ("client_secret", client_secret),
        ];

        let resp = self
            .http
            .post(LINEAR_TOKEN_URL)
            .form(&form)
            .send()
            .await
            .map_err(|e| {
                AdapterError::AdapterApi(format!("Linear refresh request failed: {e}"))
            })?;

        let status = resp.status();
        let text = resp.text().await.map_err(|e| {
            AdapterError::AdapterApi(format!("Linear refresh body read failed: {e}"))
        })?;

        if !status.is_success() {
            return Err(AdapterError::AdapterApi(format!(
                "Linear refresh returned {status}: {text}"
            )));
        }

        serde_json::from_str(&text)
            .map_err(|e| AdapterError::AdapterApi(format!("Linear refresh parse error: {e}")))
    }

    /// Send a GraphQL request with a fresh access token. Surface errors via
    /// `AdapterError::AdapterApi`.
    async fn graphql(&self, body: serde_json::Value) -> Result<serde_json::Value, AdapterError> {
        let token = self.resolve_access_token().await?;
        self.graphql_with_token(&token, body).await
    }

    /// Variant for the OAuth-install code path: the access token is in hand
    /// from a token-exchange response but hasn't been persisted yet.
    pub async fn graphql_with_token(
        &self,
        access_token: &str,
        body: serde_json::Value,
    ) -> Result<serde_json::Value, AdapterError> {
        let resp = self
            .http
            .post(LINEAR_GRAPHQL_URL)
            .header("Authorization", format!("Bearer {access_token}"))
            .header("Content-Type", "application/json")
            .header("User-Agent", "moatless-vibe")
            .json(&body)
            .send()
            .await
            .map_err(|e| AdapterError::AdapterApi(format!("Linear request failed: {e}")))?;

        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| AdapterError::AdapterApi(format!("Linear response read failed: {e}")))?;

        if !status.is_success() {
            warn!(status = %status, body = %text, "Linear GraphQL HTTP error");
            return Err(AdapterError::AdapterApi(format!(
                "Linear API returned {status}: {text}"
            )));
        }

        let value: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| AdapterError::AdapterApi(format!("Linear JSON parse error: {e}")))?;

        if let Some(errors) = value.get("errors").and_then(serde_json::Value::as_array) {
            if !errors.is_empty() {
                warn!(errors = %serde_json::Value::Array(errors.clone()), "Linear GraphQL returned errors");
                return Err(AdapterError::AdapterApi(format!(
                    "Linear GraphQL errors: {errors:?}"
                )));
            }
        }

        Ok(value)
    }

    /// Emit an activity on a session. Linear infers the session state from
    /// emitted activity kinds (e.g. `response` → completed, `elicitation` →
    /// awaiting input).
    pub async fn agent_activity_create(
        &self,
        session_id: &str,
        kind: ActivityKind,
        body: &str,
    ) -> Result<String, AdapterError> {
        let value = self
            .graphql(serde_json::json!({
                "query": "mutation($input: AgentActivityCreateInput!) { \
                    agentActivityCreate(input: $input) { success agentActivity { id } } \
                }",
                "variables": {
                    "input": {
                        "agentSessionId": session_id,
                        "content": { "type": kind.as_str(), "body": body },
                    }
                }
            }))
            .await?;

        let success = value
            .pointer("/data/agentActivityCreate/success")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        if !success {
            return Err(AdapterError::AdapterApi(
                "Linear agentActivityCreate returned success=false".into(),
            ));
        }
        let id = value
            .pointer("/data/agentActivityCreate/agentActivity/id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                AdapterError::AdapterApi(
                    "Linear agentActivityCreate missing agentActivity.id".into(),
                )
            })?;
        Ok(id.to_string())
    }

    /// Update the session's `externalUrls` so the moatless task is linked
    /// from the Linear Agent UI.
    pub async fn agent_session_update_external_urls(
        &self,
        session_id: &str,
        label: &str,
        url: &str,
    ) -> Result<(), AdapterError> {
        let value = self
            .graphql(serde_json::json!({
                "query": "mutation($id: String!, $input: AgentSessionUpdateInput!) { \
                    agentSessionUpdate(id: $id, input: $input) { success } \
                }",
                "variables": {
                    "id": session_id,
                    "input": {
                        "externalUrls": [{"label": label, "url": url}],
                    }
                }
            }))
            .await?;

        let success = value
            .pointer("/data/agentSessionUpdate/success")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        if !success {
            return Err(AdapterError::AdapterApi(
                "Linear agentSessionUpdate returned success=false".into(),
            ));
        }
        Ok(())
    }

    /// Fetch the authenticated viewer (used at OAuth install time to record
    /// the agent's identity).
    pub async fn viewer(&self) -> Result<ViewerInfo, AdapterError> {
        let value = self
            .graphql(serde_json::json!({
                "query": "query { viewer { id name email } }"
            }))
            .await?;
        parse_viewer(&value)
    }

    /// Variant used during OAuth install where the access token is held in
    /// memory and hasn't been persisted yet.
    pub async fn viewer_with_token(&self, access_token: &str) -> Result<ViewerInfo, AdapterError> {
        let value = self
            .graphql_with_token(
                access_token,
                serde_json::json!({
                    "query": "query { viewer { id name email } }"
                }),
            )
            .await?;
        parse_viewer(&value)
    }
}

fn parse_viewer(value: &serde_json::Value) -> Result<ViewerInfo, AdapterError> {
    let viewer = value
        .pointer("/data/viewer")
        .ok_or_else(|| AdapterError::AdapterApi("Linear viewer missing".into()))?;
    serde_json::from_value(viewer.clone())
        .map_err(|e| AdapterError::AdapterApi(format!("Linear viewer parse error: {e}")))
}

fn needs_refresh(expires_at: Option<DateTime<Utc>>) -> bool {
    let Some(expires_at) = expires_at else {
        return false; // No expiry recorded — leave as-is.
    };
    let margin = Utc::now() + chrono::Duration::seconds(REFRESH_LEEWAY_SECS);
    expires_at <= margin
}

/// Build the default HTTP client used by the Linear Agents API.
pub fn default_http_client() -> Client {
    Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent("moatless-vibe")
        .build()
        .unwrap_or_default()
}

#[derive(Debug, Clone, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    #[serde(default)]
    pub token_type: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub expires_in: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn needs_refresh_when_within_leeway() {
        let near = Utc::now() + chrono::Duration::seconds(60);
        assert!(needs_refresh(Some(near)));
    }

    #[test]
    fn does_not_refresh_when_far_in_future() {
        let far = Utc::now() + chrono::Duration::hours(2);
        assert!(!needs_refresh(Some(far)));
    }

    #[test]
    fn does_not_refresh_when_no_expiry() {
        assert!(!needs_refresh(None));
    }
}
