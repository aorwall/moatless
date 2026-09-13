//! Linear Agents GraphQL client.
//!
//! Outbound surface: `agent_activity_create` (activities),
//! `agent_session_update` (sets `externalUrls`), `viewer` (OAuth install-time
//! identity).
//!
//! Auth: workspace OAuth token lives in the global `provider_tokens` row;
//! expired tokens refresh against `https://api.linear.app/oauth/token`.

use std::time::Duration;

use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::Deserialize;
use tracing::{info, warn};

use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
use moatless_adapter_model::metrics::record_api_call;
use std::sync::Arc;

use moatless_ports::identity::{AppCredentials, AppToken};

/// Linear's API host. Replace via [`LinearAgentApi::with_base_url`] so tests
/// can drive the real request path against a stub instead of the network.
const LINEAR_API_BASE: &str = "https://api.linear.app";
const PROVIDER: &str = "linear";
/// Refresh tokens whose expiry is within this window.
const REFRESH_LEEWAY_SECS: i64 = 5 * 60;

/// Authenticated viewer information (agent identity from OAuth).
#[derive(Debug, Clone, Deserialize)]
#[allow(
    dead_code,
    reason = "reachable only while `agent_api` was `pub`; narrowing `mod.rs` in stage \
              15 of the Linear breakup surfaced it. Kept rather than deleted because \
              `agent_api.rs` internals are out of scope for that plan — see the PR."
)]
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
    Action,
    Response,
    #[allow(
        dead_code,
        reason = "reachable only while `agent_api` was `pub`; narrowing `mod.rs` in stage \
                  15 of the Linear breakup surfaced it. Kept rather than deleted because \
                  `agent_api.rs` internals are out of scope for that plan — see the PR."
    )]
    Elicitation,
    Error,
}

impl ActivityKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Thought => "thought",
            Self::Action => "action",
            Self::Response => "response",
            Self::Elicitation => "elicitation",
            Self::Error => "error",
        }
    }
}

/// GraphQL client for Linear Agents APIs, constructed per-org. Cheap to clone —
/// `reqwest::Client` and the token store are `Arc`-shared.
#[derive(Clone)]
pub struct LinearAgentApi {
    http: Client,
    /// The app-level OAuth row this Adapter authenticates with.
    /// `provider_tokens` belongs to `identity`; this is the seam it publishes
    /// over it.
    tokens: Arc<dyn AppCredentials>,
    /// GraphQL endpoint. A field rather than a constant so tests can drive the
    /// real request path against a local address.
    graphql_url: String,
    /// OAuth token endpoint, derived from the same base as `graphql_url` so a
    /// redirected client cannot end up refreshing against the real Linear.
    token_url: String,
    /// Linear organization (workspace) UUID. Used as `team_id` in
    /// `provider_tokens` for the OAuth row.
    org_id: String,
    /// OAuth `client_id` / `client_secret` for the refresh-token grant.
    oauth_client_id: Option<String>,
    oauth_client_secret: Option<String>,
}

impl LinearAgentApi {
    #[allow(
        dead_code,
        reason = "reachable only while `agent_api` was `pub`; narrowing `mod.rs` in stage \
                  15 of the Linear breakup surfaced it. Kept rather than deleted because \
                  `agent_api.rs` internals are out of scope for that plan — see the PR."
    )]
    pub fn new(
        http: Client,
        tokens: Arc<dyn AppCredentials>,
        org_id: String,
        oauth_client_id: Option<String>,
        oauth_client_secret: Option<String>,
    ) -> Self {
        Self::with_base_url(
            http,
            tokens,
            org_id,
            oauth_client_id,
            oauth_client_secret,
            LINEAR_API_BASE,
        )
    }

    /// [`Self::new`] with Linear's host replaced, for driving requests against
    /// a local stub. Both endpoints derive from `base_url` — deriving only the
    /// GraphQL one would let token refresh still reach the real API.
    pub(crate) fn with_base_url(
        http: Client,
        tokens: Arc<dyn AppCredentials>,
        org_id: String,
        oauth_client_id: Option<String>,
        oauth_client_secret: Option<String>,
        base_url: &str,
    ) -> Self {
        let base = base_url.trim_end_matches('/');
        Self {
            http,
            tokens,
            graphql_url: format!("{base}/graphql"),
            token_url: format!("{base}/oauth/token"),
            org_id,
            oauth_client_id,
            oauth_client_secret,
        }
    }

    /// The one construction every caller in this Adapter goes through.
    ///
    /// `base_url` defaults to [`LINEAR_API_BASE`] here, so a caller that forgot
    /// it doesn't silently reach the real API from a test.
    pub(crate) fn for_org(
        tokens: Arc<dyn AppCredentials>,
        org_id: &str,
        oauth_client_id: Option<&str>,
        oauth_client_secret: Option<&str>,
        base_url: Option<&str>,
    ) -> Self {
        Self::with_base_url(
            default_http_client(),
            tokens,
            org_id.to_string(),
            oauth_client_id.map(str::to_string),
            oauth_client_secret.map(str::to_string),
            base_url.unwrap_or(LINEAR_API_BASE),
        )
    }

    /// Resolve a fresh access token for this organization, refreshing if the
    /// stored token is within `REFRESH_LEEWAY_SECS` of expiry.
    pub async fn resolve_access_token(&self) -> Result<String, AdapterError> {
        let stored = self.tokens.app_token(PROVIDER).await?.ok_or_else(|| {
            SharedAdapterError::AdapterApi(
                "No Linear OAuth token registered — \
                     run `moat adapters install --adapter linear` first"
                    .to_string(),
            )
        })?;
        let AppToken {
            access_token,
            refresh_token,
            expires_at,
            metadata,
            ..
        } = stored;

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
        self.store_refreshed(refreshed, &refresh_token, metadata)
            .await
    }

    /// Persist a freshly-refreshed token set and return the new access token.
    ///
    /// `prev_refresh` is kept as a fallback when Linear's response rotates in
    /// no new refresh token.
    async fn store_refreshed(
        &self,
        refreshed: TokenResponse,
        prev_refresh: &str,
        prev_metadata: Option<serde_json::Value>,
    ) -> Result<String, AdapterError> {
        let new_expires_at = refreshed
            .expires_in
            .map(|secs| Utc::now() + chrono::Duration::seconds(secs as i64));

        let new_refresh = refreshed
            .refresh_token
            .clone()
            .or_else(|| Some(prev_refresh.to_string()));

        let new_metadata = prev_metadata
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

        self.tokens
            .store_app_token(
                PROVIDER,
                &AppToken {
                    access_token: refreshed.access_token.clone(),
                    refresh_token: new_refresh,
                    token_type: refreshed.token_type.clone(),
                    expires_at: new_expires_at,
                    metadata: new_metadata,
                },
            )
            .await?;

        Ok(refreshed.access_token)
    }

    /// Force a token refresh regardless of recorded expiry.
    ///
    /// Linear can invalidate a token server-side without touching `expires_at`,
    /// so `needs_refresh` never trips on its own; errors with
    /// [`SharedAdapterError::Unauthorized`] when no refresh token is stored.
    async fn force_refresh_access_token(&self) -> Result<String, AdapterError> {
        let stored = self.tokens.app_token(PROVIDER).await?;
        let (refresh_token, metadata) =
            refresh_token_from_row(stored.map(|t| (t.refresh_token, t.metadata)))?;

        info!(org_id = %self.org_id, "Force-refreshing Linear OAuth access token after 401");
        let refreshed = self.refresh_tokens(&refresh_token).await?;
        self.store_refreshed(refreshed, &refresh_token, metadata)
            .await
    }

    /// Exchange a refresh token for a new access token.
    async fn refresh_tokens(&self, refresh_token: &str) -> Result<TokenResponse, AdapterError> {
        let client_id = self.oauth_client_id.as_deref().ok_or_else(|| {
            SharedAdapterError::AdapterApi(
                "LINEAR_OAUTH_CLIENT_ID is not configured; cannot refresh Linear token".into(),
            )
        })?;
        let client_secret = self.oauth_client_secret.as_deref().ok_or_else(|| {
            SharedAdapterError::AdapterApi(
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
            .post(&self.token_url)
            .form(&form)
            .send()
            .await
            .map_err(|e| SharedAdapterError::provider_unreachable("Linear refresh request", e))?;

        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| SharedAdapterError::provider_unreachable("Linear refresh body read", e))?;

        if !status.is_success() {
            return Err(SharedAdapterError::AdapterApi(format!(
                "Linear refresh returned {status}: {text}"
            ))
            .into());
        }

        serde_json::from_str(&text)
            .map_err(|e| SharedAdapterError::provider_response_invalid("Linear refresh", e).into())
    }

    /// Send a GraphQL request with a fresh access token. Surface errors via
    /// `SharedAdapterError::AdapterApi`.
    pub async fn graphql(
        &self,
        body: serde_json::Value,
    ) -> Result<serde_json::Value, AdapterError> {
        let value = self.graphql_raw(body).await?;
        check_graphql_errors(&value)?;
        Ok(value)
    }

    /// Variant for the OAuth-install code path: the access token is in hand
    /// from a token-exchange response but hasn't been persisted yet.
    #[allow(
        dead_code,
        reason = "reachable only while `agent_api` was `pub`; narrowing `mod.rs` in stage \
                  15 of the Linear breakup surfaced it. Kept rather than deleted because \
                  `agent_api.rs` internals are out of scope for that plan — see the PR."
    )]
    pub async fn graphql_with_token(
        &self,
        access_token: &str,
        body: serde_json::Value,
    ) -> Result<serde_json::Value, AdapterError> {
        let value = self.graphql_raw_with_token(access_token, body).await?;
        check_graphql_errors(&value)?;
        Ok(value)
    }

    /// Send a GraphQL request without converting a non-empty `errors` array
    /// into `AdapterError`. Use to inspect Linear's structured error codes;
    /// most callers should prefer [`graphql`].
    async fn graphql_raw(
        &self,
        body: serde_json::Value,
    ) -> Result<serde_json::Value, AdapterError> {
        let token = self.resolve_access_token().await?;
        match self.graphql_raw_with_token(&token, body.clone()).await {
            Err(AdapterError::Shared(SharedAdapterError::Unauthorized(_))) => {
                warn!(org_id = %self.org_id, "Linear 401 — forcing token refresh and retrying once");
                let fresh = self.force_refresh_access_token().await?;
                self.graphql_raw_with_token(&fresh, body).await
            }
            other => other,
        }
    }

    async fn graphql_raw_with_token(
        &self,
        access_token: &str,
        body: serde_json::Value,
    ) -> Result<serde_json::Value, AdapterError> {
        let resp = self
            .http
            .post(&self.graphql_url)
            .header("Authorization", format!("Bearer {access_token}"))
            .header("Content-Type", "application/json")
            .header("User-Agent", "moatless-vibe")
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                record_api_call(PROVIDER, "error");
                SharedAdapterError::provider_unreachable("Linear request", e)
            })?;

        let status = resp.status();
        let text = resp.text().await.map_err(|e| {
            record_api_call(PROVIDER, "error");
            SharedAdapterError::provider_unreachable("Linear response read", e)
        })?;

        if status == reqwest::StatusCode::UNAUTHORIZED {
            warn!(status = %status, body = %text, "Linear GraphQL 401 unauthorized");
            record_api_call(PROVIDER, "auth_error");
            return Err(SharedAdapterError::Unauthorized(format!(
                "Linear API returned {status}: {text}"
            ))
            .into());
        }

        if !status.is_success() {
            warn!(status = %status, body = %text, "Linear GraphQL HTTP error");
            record_api_call(PROVIDER, "error");
            return Err(SharedAdapterError::AdapterApi(format!(
                "Linear API returned {status}: {text}"
            ))
            .into());
        }

        match serde_json::from_str(&text) {
            Ok(value) => {
                record_api_call(PROVIDER, "success");
                Ok(value)
            }
            Err(e) => {
                record_api_call(PROVIDER, "error");
                Err(SharedAdapterError::provider_response_invalid("Linear JSON", e).into())
            }
        }
    }

    /// Emit an activity on a session. Linear infers session state from the
    /// activity kind (`response` → completed, `elicitation` → awaiting input).
    /// Only `thought` or `action` activities may be marked `ephemeral`.
    pub async fn agent_activity_create(
        &self,
        session_id: &str,
        kind: ActivityKind,
        body: &str,
        ephemeral: bool,
    ) -> Result<String, AdapterError> {
        let value = self
            .graphql(serde_json::json!({
                "query": "mutation($input: AgentActivityCreateInput!) { \
                    agentActivityCreate(input: $input) { success agentActivity { id } } \
                }",
                "variables": {
                    "input": {
                        "agentSessionId": session_id,
                        "ephemeral": ephemeral,
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
            return Err(SharedAdapterError::AdapterApi(
                "Linear agentActivityCreate returned success=false".into(),
            )
            .into());
        }
        let id = value
            .pointer("/data/agentActivityCreate/agentActivity/id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                SharedAdapterError::AdapterApi(
                    "Linear agentActivityCreate missing agentActivity.id".into(),
                )
            })?;
        Ok(id.to_string())
    }

    /// Emit an `action` activity. `result` is omitted when `None`; the pattern
    /// is an ephemeral start (`result=None`) then a non-ephemeral completion
    /// with the same verb and `result=Some(...)`.
    pub async fn agent_activity_create_action(
        &self,
        session_id: &str,
        action: &str,
        parameter: &str,
        result: Option<&str>,
        ephemeral: bool,
    ) -> Result<String, AdapterError> {
        let mut content = serde_json::json!({
            "type": ActivityKind::Action.as_str(),
            "action": action,
            "parameter": parameter,
        });
        if let Some(r) = result {
            content["result"] = serde_json::Value::String(r.to_string());
        }

        let value = self
            .graphql(serde_json::json!({
                "query": "mutation($input: AgentActivityCreateInput!) { \
                    agentActivityCreate(input: $input) { success agentActivity { id } } \
                }",
                "variables": {
                    "input": {
                        "agentSessionId": session_id,
                        "ephemeral": ephemeral,
                        "content": content,
                    }
                }
            }))
            .await?;

        let success = value
            .pointer("/data/agentActivityCreate/success")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        if !success {
            return Err(SharedAdapterError::AdapterApi(
                "Linear agentActivityCreate(action) returned success=false".into(),
            )
            .into());
        }
        let id = value
            .pointer("/data/agentActivityCreate/agentActivity/id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                SharedAdapterError::AdapterApi(
                    "Linear agentActivityCreate(action) missing agentActivity.id".into(),
                )
            })?;
        Ok(id.to_string())
    }

    /// Update the session's `externalUrls` so the moatless task is linked from
    /// the Linear Agent UI.
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
            return Err(SharedAdapterError::AdapterApi(
                "Linear agentSessionUpdate returned success=false".into(),
            )
            .into());
        }
        Ok(())
    }

    /// Fetch the authenticated viewer (used at OAuth install time to record the
    /// agent's identity).
    #[allow(
        dead_code,
        reason = "reachable only while `agent_api` was `pub`; narrowing `mod.rs` in stage \
                  15 of the Linear breakup surfaced it. Kept rather than deleted because \
                  `agent_api.rs` internals are out of scope for that plan — see the PR."
    )]
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
    #[allow(
        dead_code,
        reason = "reachable only while `agent_api` was `pub`; narrowing `mod.rs` in stage \
                  15 of the Linear breakup surfaced it. Kept rather than deleted because \
                  `agent_api.rs` internals are out of scope for that plan — see the PR."
    )]
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

    // ------------------------------------------------------------------
    // Issue CRUD + lookup operations
    //
    // Thin wrappers around the Linear GraphQL API. Inputs are passed
    // through as `Value` so callers (HTTP handlers) own DTO shape and
    // name-to-UUID resolution. Each operation returns the relevant
    // subobject — never the full GraphQL envelope.
    // ------------------------------------------------------------------

    /// Fetch one issue by identifier (`SP-123`) or UUID. Returns the issue
    /// subobject.
    pub async fn get_issue(&self, id_or_key: &str) -> Result<serde_json::Value, AdapterError> {
        let value = self
            .graphql(serde_json::json!({
                "query": format!(
                    "query($id: String!) {{ issue(id: $id) {{ {ISSUE_FIELDS} }} }}"
                ),
                "variables": { "id": id_or_key }
            }))
            .await?;
        value.pointer("/data/issue").cloned().ok_or_else(|| {
            SharedAdapterError::NotFound(format!("Linear issue {id_or_key} not found")).into()
        })
    }

    /// Run `issueCreate` and return the created issue. Caller is responsible
    /// for resolving names to UUIDs in `input`.
    pub async fn create_issue(
        &self,
        input: serde_json::Value,
    ) -> Result<serde_json::Value, AdapterError> {
        let value = self
            .graphql(serde_json::json!({
                "query": format!(
                    "mutation($input: IssueCreateInput!) {{ \
                        issueCreate(input: $input) {{ success issue {{ {ISSUE_FIELDS} }} }} \
                    }}"
                ),
                "variables": { "input": input }
            }))
            .await?;
        require_mutation_success(&value, "/data/issueCreate", "issueCreate")?;
        value
            .pointer("/data/issueCreate/issue")
            .cloned()
            .ok_or_else(|| {
                SharedAdapterError::AdapterApi("issueCreate missing issue subobject".into()).into()
            })
    }

    /// Run `issueUpdate` and return the updated issue.
    pub async fn update_issue(
        &self,
        id: &str,
        input: serde_json::Value,
    ) -> Result<serde_json::Value, AdapterError> {
        let value = self
            .graphql(serde_json::json!({
                "query": format!(
                    "mutation($id: String!, $input: IssueUpdateInput!) {{ \
                        issueUpdate(id: $id, input: $input) {{ success issue {{ {ISSUE_FIELDS} }} }} \
                    }}"
                ),
                "variables": { "id": id, "input": input }
            }))
            .await?;
        require_mutation_success(&value, "/data/issueUpdate", "issueUpdate")?;
        value
            .pointer("/data/issueUpdate/issue")
            .cloned()
            .ok_or_else(|| {
                SharedAdapterError::AdapterApi("issueUpdate missing issue subobject".into()).into()
            })
    }

    /// Post a comment on an issue. Returns the created comment subobject (`id`,
    /// `url`, `body`).
    pub async fn create_comment(
        &self,
        issue_id: &str,
        body: &str,
    ) -> Result<serde_json::Value, AdapterError> {
        let value = self
            .graphql(serde_json::json!({
                "query": "mutation($input: CommentCreateInput!) { \
                    commentCreate(input: $input) { success comment { id url body createdAt } } \
                }",
                "variables": {
                    "input": { "issueId": issue_id, "body": body }
                }
            }))
            .await?;
        require_mutation_success(&value, "/data/commentCreate", "commentCreate")?;
        value
            .pointer("/data/commentCreate/comment")
            .cloned()
            .ok_or_else(|| {
                SharedAdapterError::AdapterApi("commentCreate missing comment subobject".into())
                    .into()
            })
    }

    /// Attach an external URL to a Linear issue as a "source" pill.
    ///
    /// Linear enforces URL-uniqueness workspace-wide, so this is idempotent on
    /// `(issueId, url)` but not across issues; an already-linked-elsewhere
    /// `INPUT_ERROR` is treated as success.
    pub async fn attachment_link_url(
        &self,
        issue_id: &str,
        url: &str,
        title: &str,
    ) -> Result<(), AdapterError> {
        let value = self
            .graphql_raw(serde_json::json!({
                "query": "mutation($issueId: String!, $url: String!, $title: String!) { \
                    attachmentLinkURL(issueId: $issueId, url: $url, title: $title) { \
                        success attachment { id url } \
                    } \
                }",
                "variables": { "issueId": issue_id, "url": url, "title": title }
            }))
            .await?;

        if let Some(errors) = value.get("errors").and_then(serde_json::Value::as_array) {
            if errors.iter().any(is_attachment_already_linked_error) {
                info!(
                    issue = %issue_id,
                    url = %url,
                    "Linear attachmentLinkURL: URL already linked to a different issue; treating as success"
                );
                return Ok(());
            }
        }
        check_graphql_errors(&value)?;
        require_mutation_success(&value, "/data/attachmentLinkURL", "attachmentLinkURL")
    }

    /// Non-destructive label add.
    pub async fn issue_add_label(
        &self,
        issue_id: &str,
        label_id: &str,
    ) -> Result<(), AdapterError> {
        let value = self
            .graphql(serde_json::json!({
                "query": "mutation($id: String!, $labelId: String!) { \
                    issueAddLabel(id: $id, labelId: $labelId) { success } \
                }",
                "variables": { "id": issue_id, "labelId": label_id }
            }))
            .await?;
        require_mutation_success(&value, "/data/issueAddLabel", "issueAddLabel")
    }

    /// Non-destructive label remove.
    pub async fn issue_remove_label(
        &self,
        issue_id: &str,
        label_id: &str,
    ) -> Result<(), AdapterError> {
        let value = self
            .graphql(serde_json::json!({
                "query": "mutation($id: String!, $labelId: String!) { \
                    issueRemoveLabel(id: $id, labelId: $labelId) { success } \
                }",
                "variables": { "id": issue_id, "labelId": label_id }
            }))
            .await?;
        require_mutation_success(&value, "/data/issueRemoveLabel", "issueRemoveLabel")
    }

    /// Run `issueRelationCreate` and return the created relation.
    ///
    /// Both ids must be UUIDs — Linear's `IssueRelationCreateInput` rejects
    /// `SP-123` keys, so the caller resolves them first; `relation` is Linear's
    /// `IssueRelationType` wire value.
    pub async fn issue_relate(
        &self,
        issue_uuid: &str,
        related_issue_uuid: &str,
        relation: &str,
    ) -> Result<serde_json::Value, AdapterError> {
        let value = self
            .graphql(serde_json::json!({
                "query": "mutation($input: IssueRelationCreateInput!) { \
                    issueRelationCreate(input: $input) { \
                        success issueRelation { \
                            id type \
                            issue { id identifier } \
                            relatedIssue { id identifier } \
                        } \
                    } \
                }",
                "variables": { "input": {
                    "issueId": issue_uuid,
                    "relatedIssueId": related_issue_uuid,
                    "type": relation,
                }}
            }))
            .await?;
        require_mutation_success(&value, "/data/issueRelationCreate", "issueRelationCreate")?;
        value
            .pointer("/data/issueRelationCreate/issueRelation")
            .cloned()
            .ok_or_else(|| {
                SharedAdapterError::AdapterApi(
                    "issueRelationCreate missing issueRelation subobject".into(),
                )
                .into()
            })
    }

    /// List issues matching `filter` (a Linear `IssueFilter` object), newest
    /// first. `first` is passed straight through as the page size; this reads
    /// one page only, which is what a sibling-ticket lookup needs.
    pub async fn list_issues(
        &self,
        filter: serde_json::Value,
        first: i64,
    ) -> Result<Vec<serde_json::Value>, AdapterError> {
        let value = self
            .graphql(serde_json::json!({
                "query": format!(
                    "query($filter: IssueFilter, $first: Int!) {{ \
                        issues(filter: $filter, first: $first, orderBy: updatedAt) {{ \
                            nodes {{ {ISSUE_FIELDS} }} \
                        }} \
                    }}"
                ),
                "variables": { "filter": filter, "first": first }
            }))
            .await?;
        Ok(value
            .pointer("/data/issues/nodes")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default())
    }

    /// Resolve a human name to a Linear entity UUID (first case-insensitive
    /// match, `None` if nothing matches). Pass `team_id` when in hand; unscoped
    /// search is a guess for anything but a user or team.
    pub async fn lookup(
        &self,
        kind: LookupKind,
        name: &str,
        team_id: Option<&str>,
    ) -> Result<Option<LookupResult>, AdapterError> {
        let (collection, filter_type, fields) = match kind {
            LookupKind::State => ("workflowStates", "WorkflowStateFilter", "id name type"),
            LookupKind::Label => ("issueLabels", "IssueLabelFilter", "id name color"),
            LookupKind::Project => ("projects", "ProjectFilter", "id name state"),
            LookupKind::User => ("users", "UserFilter", "id name displayName email"),
            LookupKind::Team => ("teams", "TeamFilter", "id name key"),
        };
        let mut filter = serde_json::json!({ "name": { "eqIgnoreCase": name } });
        if let Some((field, clause)) = team_id.and_then(|id| kind.team_scope(id)) {
            filter[field] = clause;
        }
        let query = format!(
            "query($filter: {filter_type}) {{ \
                {collection}(filter: $filter, first: 5) {{ \
                    nodes {{ {fields} }} \
                }} \
            }}"
        );
        let value = self
            .graphql(serde_json::json!({
                "query": query,
                "variables": { "filter": filter }
            }))
            .await?;
        let nodes = value
            .pointer(&format!("/data/{collection}/nodes"))
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        let Some(first) = nodes.into_iter().next() else {
            return Ok(None);
        };
        let id = first
            .get("id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                SharedAdapterError::AdapterApi(format!("Linear {collection} missing id"))
            })?
            .to_string();
        let resolved_name = first
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(name)
            .to_string();
        Ok(Some(LookupResult {
            id,
            name: resolved_name,
            raw: first,
        }))
    }
}

/// Kind of Linear entity name being looked up.
#[derive(Debug, Clone, Copy)]
pub enum LookupKind {
    State,
    Label,
    Project,
    User,
    Team,
}

impl LookupKind {
    /// Filter field + clause restricting this kind to `team_id`.
    ///
    /// Names are unique only within a team, so an unscoped match can return
    /// another team's entity, which `issueUpdate` rejects; the relation field
    /// differs per collection (`accessibleTeams` for projects, none for
    /// `users`/`teams`).
    fn team_scope(self, team_id: &str) -> Option<(&'static str, serde_json::Value)> {
        let team_is = serde_json::json!({ "id": { "eq": team_id } });
        match self {
            Self::State => Some(("team", team_is)),
            Self::Label => Some((
                "or",
                serde_json::json!([{ "team": team_is }, { "team": { "null": true } }]),
            )),
            Self::Project => Some((
                "accessibleTeams",
                serde_json::json!({ "some": { "id": { "eq": team_id } } }),
            )),
            Self::User | Self::Team => None,
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "state" | "workflow_state" => Some(Self::State),
            "label" => Some(Self::Label),
            "project" => Some(Self::Project),
            "user" => Some(Self::User),
            "team" => Some(Self::Team),
            _ => None,
        }
    }
}

/// Resolved entity returned by [`LinearAgentApi::lookup`].
#[derive(Debug, Clone)]
pub struct LookupResult {
    pub id: String,
    pub name: String,
    pub raw: serde_json::Value,
}

/// Standard issue field selection used by `get_issue`, `create_issue`,
/// `update_issue`. Kept compact to stay well under Linear's per-query
/// complexity budget.
const ISSUE_FIELDS: &str = "id identifier number title description priority \
estimate dueDate url \
state { id name type } \
assignee { id name displayName email } \
delegate { id name displayName email } \
creator { id name displayName email } \
project { id name state } \
cycle { id number startsAt endsAt } \
parent { id identifier } \
labels { nodes { id name color } } \
team { id key name } \
createdAt updatedAt";

/// Convert a non-empty top-level `errors` array into `AdapterError`.
///
/// Shared by [`LinearAgentApi::graphql`] and callers like `attachment_link_url`
/// that need to peek at errors before surfacing the standard adapter error.
fn check_graphql_errors(value: &serde_json::Value) -> Result<(), AdapterError> {
    let Some(errors) = value.get("errors").and_then(serde_json::Value::as_array) else {
        return Ok(());
    };
    if errors.is_empty() {
        return Ok(());
    }
    warn!(
        errors = %serde_json::Value::Array(errors.clone()),
        "Linear GraphQL returned errors"
    );
    Err(SharedAdapterError::AdapterApi(format!("Linear GraphQL errors: {errors:?}")).into())
}

/// Recognise Linear's "URL already linked with <issue>" response from
/// `attachmentLinkURL`. Linear enforces workspace-wide URL uniqueness; the
/// rejection is a structured `INPUT_ERROR` with `userError: true`.
fn is_attachment_already_linked_error(error: &serde_json::Value) -> bool {
    let code = error
        .pointer("/extensions/code")
        .and_then(serde_json::Value::as_str);
    let path_matches = error
        .get("path")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|p| p.iter().any(|s| s.as_str() == Some("attachmentLinkURL")));
    let msg = error
        .pointer("/extensions/userPresentableMessage")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    code == Some("INPUT_ERROR") && path_matches && msg.contains("already been linked")
}

/// Verify a mutation returned `success: true` and propagate a clean error
/// otherwise. `op_name` is used in the error message; `pointer` is the
/// JSON-pointer prefix to the mutation result (e.g. `/data/issueCreate`).
fn require_mutation_success(
    value: &serde_json::Value,
    pointer: &str,
    op_name: &str,
) -> Result<(), AdapterError> {
    let success = value
        .pointer(&format!("{pointer}/success"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    if !success {
        return Err(SharedAdapterError::AdapterApi(format!(
            "Linear {op_name} returned success=false"
        ))
        .into());
    }
    Ok(())
}

#[allow(
    dead_code,
    reason = "reachable only while `agent_api` was `pub`; narrowing `mod.rs` in stage \
              15 of the Linear breakup surfaced it. Kept rather than deleted because \
              `agent_api.rs` internals are out of scope for that plan — see the PR."
)]
fn parse_viewer(value: &serde_json::Value) -> Result<ViewerInfo, AdapterError> {
    let viewer = value
        .pointer("/data/viewer")
        .ok_or_else(|| SharedAdapterError::AdapterApi("Linear viewer missing".into()))?;
    serde_json::from_value(viewer.clone())
        .map_err(|e| SharedAdapterError::provider_response_invalid("Linear viewer", e).into())
}

/// Decide whether a force-refresh can proceed from the stored token row.
///
/// Returns the refresh token + metadata to exchange, or
/// [`SharedAdapterError::Unauthorized`] when the row is missing or has no
/// refresh token — the unrecoverable case that surfaces to the auth-failure
/// alert.
type TokenRow = Option<(Option<String>, Option<serde_json::Value>)>;

fn refresh_token_from_row(
    row: TokenRow,
) -> Result<(String, Option<serde_json::Value>), AdapterError> {
    let (refresh_token, metadata) = row.ok_or_else(|| {
        SharedAdapterError::Unauthorized(
            "No Linear OAuth token registered — \
             run `moat adapters install --adapter linear` first"
                .to_string(),
        )
    })?;

    let Some(refresh_token) = refresh_token else {
        return Err(SharedAdapterError::Unauthorized(
            "Linear access token rejected (401) and no refresh token stored — \
             re-install with `moat adapters install --adapter linear`"
                .to_string(),
        )
        .into());
    };

    Ok((refresh_token, metadata))
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

    /// Linear moves an app-agent assignment to `delegate` and leaves `assignee`
    /// null.
    ///
    /// Without this field, a read-back of a delegated issue reports it as
    /// unassigned and callers re-claim already-claimed work.
    #[test]
    fn issue_fields_select_delegate() {
        assert!(ISSUE_FIELDS.contains("delegate {"));
        assert!(ISSUE_FIELDS.contains("assignee {"));
    }

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

    #[test]
    fn force_refresh_unauthorized_when_no_row() {
        // No stored token row at all → unrecoverable, surfaces to the alert.
        let err = refresh_token_from_row(None).unwrap_err();
        assert!(matches!(
            err,
            AdapterError::Shared(SharedAdapterError::Unauthorized(_))
        ));
    }

    #[test]
    fn force_refresh_unauthorized_when_no_refresh_token() {
        // Row exists but refresh_token is NULL → still unrecoverable.
        let err = refresh_token_from_row(Some((None, None))).unwrap_err();
        assert!(matches!(
            err,
            AdapterError::Shared(SharedAdapterError::Unauthorized(_))
        ));
    }

    #[test]
    fn force_refresh_returns_token_when_present() {
        let meta = Some(serde_json::json!({"scope": "read"}));
        let (token, returned_meta) =
            refresh_token_from_row(Some((Some("refresh-abc".to_string()), meta.clone()))).unwrap();
        assert_eq!(token, "refresh-abc");
        assert_eq!(returned_meta, meta);
    }

    #[test]
    fn detects_attachment_already_linked_error() {
        let err = serde_json::json!({
            "extensions": {
                "code": "INPUT_ERROR",
                "userError": true,
                "userPresentableMessage": "This URL has already been linked with OP-3."
            },
            "message": "Unable to create issue attachment",
            "path": ["attachmentLinkURL"],
        });
        assert!(is_attachment_already_linked_error(&err));
    }

    #[test]
    fn does_not_match_unrelated_input_error() {
        let err = serde_json::json!({
            "extensions": {
                "code": "INPUT_ERROR",
                "userPresentableMessage": "Some other validation problem"
            },
            "path": ["attachmentLinkURL"],
        });
        assert!(!is_attachment_already_linked_error(&err));
    }

    #[test]
    fn does_not_match_already_linked_on_other_mutation() {
        // Defensive: same wording but a different mutation path — don't
        // silently swallow.
        let err = serde_json::json!({
            "extensions": {
                "code": "INPUT_ERROR",
                "userPresentableMessage": "This URL has already been linked with OP-3."
            },
            "path": ["someOtherMutation"],
        });
        assert!(!is_attachment_already_linked_error(&err));
    }
}

/// The filter each [`LookupKind`] sends. Every case here was run against the
/// live API before it was written down: Linear rejects an unknown filter field
/// outright, and a stub would happily accept one this crate invented.
#[cfg(test)]
mod lookup_scope_tests {
    use moatless_adapter_model::test_support::ProviderStub;
    use moatless_ports::identity::MockAppCredentials;
    use serde_json::json;

    use super::*;

    const TEAM: &str = "3fa49663-046d-46db-9566-584753ff4b23";

    /// The document and filter one lookup put on the wire.
    async fn request_for(kind: LookupKind, team_id: Option<&str>) -> (String, serde_json::Value) {
        let stub = ProviderStub::start(|_| json!({ "data": {} })).await;
        let mut tokens = MockAppCredentials::new();
        tokens.expect_app_token().returning(|_| {
            Ok(Some(AppToken {
                access_token: "lin_test".into(),
                refresh_token: None,
                token_type: None,
                expires_at: Some(Utc::now() + chrono::Duration::days(1)),
                metadata: None,
            }))
        });
        let api = LinearAgentApi::with_base_url(
            Client::new(),
            Arc::new(tokens),
            "test-org".to_string(),
            None,
            None,
            stub.base_url(),
        );

        api.lookup(kind, "Canceled", team_id)
            .await
            .expect("the stub answers");

        let body: serde_json::Value = serde_json::from_str(&stub.requests()[0].body)
            .expect("the recorded body is the JSON we sent");
        (
            body["query"].as_str().expect("query is a string").into(),
            body["variables"]["filter"].clone(),
        )
    }

    #[tokio::test]
    async fn a_state_lookup_narrows_to_the_team() {
        let (query, filter) = request_for(LookupKind::State, Some(TEAM)).await;
        assert!(
            query.contains("query($filter: WorkflowStateFilter)"),
            "{query}"
        );
        assert_eq!(
            filter,
            json!({
                "name": { "eqIgnoreCase": "Canceled" },
                "team": { "id": { "eq": TEAM } }
            })
        );
    }

    /// A workspace label has no team and stays usable on every issue, so the
    /// team clause has to admit a null team alongside the match.
    #[tokio::test]
    async fn a_label_lookup_keeps_workspace_labels_in_reach() {
        let (query, filter) = request_for(LookupKind::Label, Some(TEAM)).await;
        assert!(query.contains("query($filter: IssueLabelFilter)"), "{query}");
        assert_eq!(
            filter,
            json!({
                "name": { "eqIgnoreCase": "Canceled" },
                "or": [
                    { "team": { "id": { "eq": TEAM } } },
                    { "team": { "null": true } }
                ]
            })
        );
    }

    /// A project is shared with teams rather than owned by one, so
    /// `ProjectFilter` carries no `team` field to filter on.
    #[tokio::test]
    async fn a_project_lookup_narrows_through_accessible_teams() {
        let (query, filter) = request_for(LookupKind::Project, Some(TEAM)).await;
        assert!(query.contains("query($filter: ProjectFilter)"), "{query}");
        assert_eq!(
            filter,
            json!({
                "name": { "eqIgnoreCase": "Canceled" },
                "accessibleTeams": { "some": { "id": { "eq": TEAM } } }
            })
        );
    }

    /// Users and teams are workspace-global: a team id passed anyway is dropped
    /// rather than turned into a filter field neither collection has.
    #[tokio::test]
    async fn users_and_teams_ignore_a_team_scope() {
        for kind in [LookupKind::User, LookupKind::Team] {
            let (_, filter) = request_for(kind, Some(TEAM)).await;
            assert_eq!(filter, json!({ "name": { "eqIgnoreCase": "Canceled" } }));
        }
    }

    #[tokio::test]
    async fn an_unscoped_lookup_filters_on_the_name_alone() {
        let (_, filter) = request_for(LookupKind::State, None).await;
        assert_eq!(filter, json!({ "name": { "eqIgnoreCase": "Canceled" } }));
    }
}

#[cfg(test)]
mod error_source_tests {
    use std::error::Error as _;

    use axum::http::StatusCode;

    use super::*;
    use moatless_adapter_model::error::ProviderDecodeError;
    use moatless_adapter_model::test_support::{UNREACHABLE_BASE, stub_provider};

    /// An app-credential port nothing stands behind.
    ///
    /// These tests assert the request path, not the credential read;
    /// `graphql_with_token` takes the token as an argument, so it never touches
    /// the database.
    struct NoAppToken;

    #[async_trait::async_trait]
    impl AppCredentials for NoAppToken {
        async fn app_token(
            &self,
            _provider: &str,
        ) -> Result<Option<AppToken>, moatless_types::store::StoreError> {
            Ok(None)
        }

        async fn store_app_token(
            &self,
            _provider: &str,
            _token: &AppToken,
        ) -> Result<(), moatless_types::store::StoreError> {
            Ok(())
        }
    }

    fn offline_pool() -> Arc<dyn AppCredentials> {
        Arc::new(NoAppToken)
    }

    /// A client whose whole API host is `base`, built the way production builds
    /// one — same constructor, same fields, only the host replaced.
    fn api_at(base: impl AsRef<str>) -> LinearAgentApi {
        LinearAgentApi::with_base_url(
            Client::new(),
            offline_pool(),
            "test-org".to_string(),
            None,
            None,
            base.as_ref(),
        )
    }

    #[tokio::test]
    async fn unreachable_linear_keeps_transport_source() {
        let api = api_at(UNREACHABLE_BASE);

        let err = api
            .graphql_with_token(
                "lin_test",
                serde_json::json!({ "query": "{ viewer { id } }" }),
            )
            .await
            .expect_err("dead loopback port must fail");

        let AdapterError::Shared(SharedAdapterError::ProviderUnreachable {
            ref operation,
            ref source,
        }) = err
        else {
            panic!("expected ProviderUnreachable, got {err:?}");
        };
        assert_eq!(operation, "Linear request");
        assert!(source.is_connect() || source.is_request(), "{source}");
        assert!(err.source().is_some(), "cause must stay reachable");
    }

    #[tokio::test]
    async fn undecodable_linear_body_keeps_serde_source() {
        let base = stub_provider(StatusCode::OK, "<html>not json</html>").await;
        let api = api_at(base);

        let err = api
            .graphql_with_token(
                "lin_test",
                serde_json::json!({ "query": "{ viewer { id } }" }),
            )
            .await
            .expect_err("html body must not decode");

        let AdapterError::Shared(SharedAdapterError::ProviderResponseInvalid {
            ref operation,
            ref source,
        }) = err
        else {
            panic!("expected ProviderResponseInvalid, got {err:?}");
        };
        assert_eq!(operation, "Linear JSON");
        assert!(
            matches!(source, ProviderDecodeError::Json(_)),
            "Linear buffers the body itself, so the cause is a serde error"
        );
        assert!(err.source().is_some(), "cause must stay reachable");
    }

    /// A Linear 401 is a rejection we classify separately from "unreachable".
    #[tokio::test]
    async fn linear_401_stays_unauthorized() {
        let base = stub_provider(StatusCode::UNAUTHORIZED, r#"{"error":"invalid token"}"#).await;
        let api = api_at(base);

        let err = api
            .graphql_with_token(
                "lin_test",
                serde_json::json!({ "query": "{ viewer { id } }" }),
            )
            .await
            .expect_err("401 must be an error");

        assert!(
            matches!(
                err,
                AdapterError::Shared(SharedAdapterError::Unauthorized(_))
            ),
            "got {err:?}"
        );
        assert!(err.source().is_none());
    }
}
