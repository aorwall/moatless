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
//! Auth: the workspace OAuth token is persisted as the single global
//! `provider_tokens` row for the provider. Expired tokens are transparently refreshed
//! against `https://api.linear.app/oauth/token` (form-encoded).

use std::time::Duration;

use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::Deserialize;
use tracing::{info, warn};

use crate::adapters::error::AdapterError;
use crate::adapters::metrics::record_api_call;
use crate::db::DbPool;

/// Linear's API host. Every endpoint this client speaks to hangs off it, so
/// replacing this one value points the whole client — GraphQL and the OAuth
/// token exchange alike — somewhere else. That is what
/// [`LinearAgentApi::with_base_url`] exists for: without it no test can drive
/// the real request path, and an API-call budget cannot be asserted at all.
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

/// GraphQL client for Linear Agents APIs.
///
/// Constructed per-org so the workspace token lookup is bound to a single
/// connection. Cheap to clone — the underlying `reqwest::Client` and DB
/// pool are both `Arc`-shared.
#[derive(Clone)]
pub struct LinearAgentApi {
    http: Client,
    db: DbPool,
    /// GraphQL endpoint. A field rather than a constant so tests can drive the
    /// real request path against a local address.
    graphql_url: String,
    /// OAuth token endpoint, derived from the same base as `graphql_url` so a
    /// redirected client cannot end up refreshing against the real Linear.
    token_url: String,
    /// Linear organization (workspace) UUID. Used as `team_id` in
    /// `provider_tokens` for the OAuth row.
    org_id: String,
    /// OAuth client_id / client_secret for the refresh-token grant.
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
        db: DbPool,
        org_id: String,
        oauth_client_id: Option<String>,
        oauth_client_secret: Option<String>,
    ) -> Self {
        Self::with_base_url(
            http,
            db,
            org_id,
            oauth_client_id,
            oauth_client_secret,
            LINEAR_API_BASE,
        )
    }

    /// [`Self::new`] with Linear's host replaced, so the real request path can
    /// be driven against a local address.
    ///
    /// Both endpoints are derived from `base_url` rather than only the GraphQL
    /// one: a client pointed at a stub that then refreshed its token against
    /// `api.linear.app` would be reaching the network from a test.
    pub(crate) fn with_base_url(
        http: Client,
        db: DbPool,
        org_id: String,
        oauth_client_id: Option<String>,
        oauth_client_secret: Option<String>,
        base_url: &str,
    ) -> Self {
        let base = base_url.trim_end_matches('/');
        Self {
            http,
            db,
            graphql_url: format!("{base}/graphql"),
            token_url: format!("{base}/oauth/token"),
            org_id,
            oauth_client_id,
            oauth_client_secret,
        }
    }

    /// The one construction every caller in this Adapter goes through.
    ///
    /// Callers hold their credentials in different shapes — an `AppState`, the
    /// Adapter type, a route handler — but they all answer the same three
    /// questions, and `base_url` is the one whose default must not be decided
    /// per call site: a caller that forgot it would reach `api.linear.app`
    /// from a test that believed it was pointed at a stub.
    pub(crate) fn for_org(
        db: DbPool,
        org_id: &str,
        oauth_client_id: Option<&str>,
        oauth_client_secret: Option<&str>,
        base_url: Option<&str>,
    ) -> Self {
        Self::with_base_url(
            default_http_client(),
            db,
            org_id.to_string(),
            oauth_client_id.map(str::to_string),
            oauth_client_secret.map(str::to_string),
            base_url.unwrap_or(LINEAR_API_BASE),
        )
    }

    /// Resolve a fresh access token for this organization, refreshing if the
    /// stored token is within `REFRESH_LEEWAY_SECS` of expiry.
    pub async fn resolve_access_token(&self) -> Result<String, AdapterError> {
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
             WHERE provider = $1 AND user_id IS NULL AND team_id IS NULL \
             LIMIT 1",
        )
        .bind(PROVIDER)
        .fetch_optional(&self.db)
        .await
        .map_err(AdapterError::Db)?;

        let (access_token, refresh_token, expires_at, metadata) = full.ok_or_else(|| {
            AdapterError::AdapterApi(
                "No Linear OAuth token registered — \
                     run `moat adapters install --adapter linear` first"
                    .to_string(),
            )
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
        self.store_refreshed(refreshed, &refresh_token, metadata)
            .await
    }

    /// Persist a freshly-refreshed token set: upsert the access/refresh token
    /// and metadata, then update `expires_at`. Returns the new access token.
    ///
    /// Shared by the proactive (`resolve_access_token`) and forced
    /// (`force_refresh_access_token`) refresh paths. `prev_refresh` is the
    /// refresh token that was used, kept as a fallback when Linear's response
    /// rotates no new one.
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

        sqlx::query(
            r#"INSERT INTO provider_tokens (
                   provider, auth_method, access_token, refresh_token, token_type,
                   metadata, connected_at, updated_at, scope, owner_user_id, expires_at
               )
               VALUES ($1, 'oauth', $2, $3, $4, $5, NOW(), NOW(), 'global', NULL, $6)
               ON CONFLICT (user_id, provider, team_id) DO UPDATE SET
                   access_token = $2, refresh_token = $3, token_type = $4,
                   metadata = $5, auth_method = 'oauth', scope = 'global',
                   owner_user_id = NULL, updated_at = NOW(), expires_at = $6"#,
        )
        .bind(PROVIDER)
        .bind(&refreshed.access_token)
        .bind(new_refresh.as_deref())
        .bind(refreshed.token_type.as_deref())
        .bind(new_metadata.as_ref())
        .bind(new_expires_at)
        .execute(&self.db)
        .await
        .map_err(AdapterError::Db)?;

        Ok(refreshed.access_token)
    }

    /// Force a token refresh regardless of the recorded expiry, for the
    /// reactive 401 path: Linear can invalidate a token server-side without
    /// touching our `expires_at`, so `needs_refresh` never trips. Re-SELECT
    /// the current refresh token + metadata and exchange it.
    ///
    /// Returns [`AdapterError::Unauthorized`] when no refresh token is stored
    /// — that case is unrecoverable and surfaces to the auth-failure alert,
    /// prompting a re-install.
    async fn force_refresh_access_token(&self) -> Result<String, AdapterError> {
        let row = sqlx::query_as::<_, (Option<String>, Option<serde_json::Value>)>(
            "SELECT refresh_token, metadata \
             FROM provider_tokens \
             WHERE provider = $1 AND user_id IS NULL AND team_id IS NULL \
             LIMIT 1",
        )
        .bind(PROVIDER)
        .fetch_optional(&self.db)
        .await
        .map_err(AdapterError::Db)?;

        let (refresh_token, metadata) = refresh_token_from_row(row)?;

        info!(org_id = %self.org_id, "Force-refreshing Linear OAuth access token after 401");
        let refreshed = self.refresh_tokens(&refresh_token).await?;
        self.store_refreshed(refreshed, &refresh_token, metadata)
            .await
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
            .post(&self.token_url)
            .form(&form)
            .send()
            .await
            .map_err(|e| AdapterError::provider_unreachable("Linear refresh request", e))?;

        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| AdapterError::provider_unreachable("Linear refresh body read", e))?;

        if !status.is_success() {
            return Err(AdapterError::AdapterApi(format!(
                "Linear refresh returned {status}: {text}"
            )));
        }

        serde_json::from_str(&text)
            .map_err(|e| AdapterError::provider_response_invalid("Linear refresh", e))
    }

    /// Send a GraphQL request with a fresh access token. Surface errors via
    /// `AdapterError::AdapterApi`.
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

    /// Send a GraphQL request and return the raw response value *without*
    /// converting a non-empty `errors` array into `AdapterError`. Use this
    /// when you need to inspect Linear's structured error codes (e.g. to
    /// tolerate "already exists" on an upsert). Most callers should prefer
    /// [`graphql`].
    async fn graphql_raw(
        &self,
        body: serde_json::Value,
    ) -> Result<serde_json::Value, AdapterError> {
        let token = self.resolve_access_token().await?;
        match self.graphql_raw_with_token(&token, body.clone()).await {
            Err(AdapterError::Unauthorized(_)) => {
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
                AdapterError::provider_unreachable("Linear request", e)
            })?;

        let status = resp.status();
        let text = resp.text().await.map_err(|e| {
            record_api_call(PROVIDER, "error");
            AdapterError::provider_unreachable("Linear response read", e)
        })?;

        if status == reqwest::StatusCode::UNAUTHORIZED {
            warn!(status = %status, body = %text, "Linear GraphQL 401 unauthorized");
            record_api_call(PROVIDER, "auth_error");
            return Err(AdapterError::Unauthorized(format!(
                "Linear API returned {status}: {text}"
            )));
        }

        if !status.is_success() {
            warn!(status = %status, body = %text, "Linear GraphQL HTTP error");
            record_api_call(PROVIDER, "error");
            return Err(AdapterError::AdapterApi(format!(
                "Linear API returned {status}: {text}"
            )));
        }

        match serde_json::from_str(&text) {
            Ok(value) => {
                record_api_call(PROVIDER, "success");
                Ok(value)
            }
            Err(e) => {
                record_api_call(PROVIDER, "error");
                Err(AdapterError::provider_response_invalid("Linear JSON", e))
            }
        }
    }

    /// Emit an activity on a session. Linear infers the session state from
    /// emitted activity kinds (e.g. `response` → completed, `elicitation` →
    /// awaiting input).
    ///
    /// Per the Linear Agent Interaction spec, only `thought` or `action`
    /// activities can be marked `ephemeral`; ephemeral activities are
    /// displayed temporarily and replaced when the next activity arrives.
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

    /// Emit an `action` activity. Spec shape:
    /// `{ "type": "action", "action": "...", "parameter": "...", "result": "..." }`.
    /// `result` is omitted when `None`. The canonical pattern is to emit an
    /// ephemeral start (`result=None`) and then a non-ephemeral completion
    /// with the same verb (past tense) and `result=Some(...)`.
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
            return Err(AdapterError::AdapterApi(
                "Linear agentActivityCreate(action) returned success=false".into(),
            ));
        }
        let id = value
            .pointer("/data/agentActivityCreate/agentActivity/id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                AdapterError::AdapterApi(
                    "Linear agentActivityCreate(action) missing agentActivity.id".into(),
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
        value
            .pointer("/data/issue")
            .cloned()
            .ok_or_else(|| AdapterError::NotFound(format!("Linear issue {id_or_key} not found")))
    }

    /// Run `issueCreate` and return the created issue. Caller is
    /// responsible for resolving names to UUIDs in `input`.
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
            .ok_or_else(|| AdapterError::AdapterApi("issueCreate missing issue subobject".into()))
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
            .ok_or_else(|| AdapterError::AdapterApi("issueUpdate missing issue subobject".into()))
    }

    /// Resolve a workflow-state NAME to its UUID within a specific team.
    /// Linear scopes workflow states per team, so a name like "In Progress"
    /// can exist in several teams — filtering by team guarantees the issue is
    /// transitioned to the right team's state (an `issueUpdate` with a
    /// foreign-team state id is rejected). Case-insensitive; returns `None`
    /// when no state in the team matches.
    pub async fn find_team_state_id(
        &self,
        team_id: &str,
        state_name: &str,
    ) -> Result<Option<String>, AdapterError> {
        let value = self
            .graphql(serde_json::json!({
                "query": "query($teamId: ID!, $name: String!) { \
                    workflowStates(filter: { team: { id: { eq: $teamId } }, \
                                             name: { eqIgnoreCase: $name } }, first: 1) { \
                        nodes { id name } \
                    } \
                }",
                "variables": { "teamId": team_id, "name": state_name }
            }))
            .await?;
        Ok(value
            .pointer("/data/workflowStates/nodes/0/id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string))
    }

    /// Post a comment on an issue. Returns the created comment subobject
    /// (`id`, `url`, `body`).
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
                AdapterError::AdapterApi("commentCreate missing comment subobject".into())
            })
    }

    /// Attach an external URL to a Linear issue as a "source" pill. Used
    /// to link a moatless task to the Linear ticket it works on (visible
    /// directly under the issue, not just inside the agent panel).
    ///
    /// Linear enforces URL-uniqueness across the entire workspace, so this
    /// is idempotent on `(issueId, url)` but *not* across issues. If the
    /// URL is already linked to a different issue (e.g. an earlier Linear
    /// binding for the same Moatless task), Linear returns an
    /// `INPUT_ERROR`. We treat that as success — the link is still visible
    /// on the original issue, which is what the caller wanted.
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

    /// Resolve a human name to a Linear entity UUID. Returns the first
    /// match (case-insensitive); `None` if nothing matches.
    pub async fn lookup(
        &self,
        kind: LookupKind,
        name: &str,
    ) -> Result<Option<LookupResult>, AdapterError> {
        let (collection, fields) = match kind {
            LookupKind::State => ("workflowStates", "id name type"),
            LookupKind::Label => ("issueLabels", "id name color"),
            LookupKind::Project => ("projects", "id name state"),
            LookupKind::User => ("users", "id name displayName email"),
            LookupKind::Team => ("teams", "id name key"),
        };
        let query = format!(
            "query($name: String!) {{ \
                {collection}(filter: {{ name: {{ eqIgnoreCase: $name }} }}, first: 5) {{ \
                    nodes {{ {fields} }} \
                }} \
            }}"
        );
        let value = self
            .graphql(serde_json::json!({
                "query": query,
                "variables": { "name": name }
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
            .ok_or_else(|| AdapterError::AdapterApi(format!("Linear {collection} missing id")))?
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
creator { id name displayName email } \
project { id name state } \
cycle { id number startsAt endsAt } \
parent { id identifier } \
labels { nodes { id name color } } \
team { id key name } \
createdAt updatedAt";

/// Convert a non-empty top-level `errors` array into `AdapterError`.
/// Shared by [`LinearAgentApi::graphql`] and callers (like
/// `attachment_link_url`) that need to peek at errors first before
/// surfacing the standard adapter error.
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
    Err(AdapterError::AdapterApi(format!(
        "Linear GraphQL errors: {errors:?}"
    )))
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
        return Err(AdapterError::AdapterApi(format!(
            "Linear {op_name} returned success=false"
        )));
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
        .ok_or_else(|| AdapterError::AdapterApi("Linear viewer missing".into()))?;
    serde_json::from_value(viewer.clone())
        .map_err(|e| AdapterError::provider_response_invalid("Linear viewer", e))
}

/// Decide whether a force-refresh can proceed from the stored token row.
/// Returns the refresh token + metadata to exchange, or
/// [`AdapterError::Unauthorized`] when the row is missing or has no refresh
/// token — the unrecoverable case that must surface to the auth-failure alert.
type TokenRow = Option<(Option<String>, Option<serde_json::Value>)>;

fn refresh_token_from_row(
    row: TokenRow,
) -> Result<(String, Option<serde_json::Value>), AdapterError> {
    let (refresh_token, metadata) = row.ok_or_else(|| {
        AdapterError::Unauthorized(
            "No Linear OAuth token registered — \
             run `moat adapters install --adapter linear` first"
                .to_string(),
        )
    })?;

    let Some(refresh_token) = refresh_token else {
        return Err(AdapterError::Unauthorized(
            "Linear access token rejected (401) and no refresh token stored — \
             re-install with `moat adapters install --adapter linear`"
                .to_string(),
        ));
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
        assert!(matches!(err, AdapterError::Unauthorized(_)));
    }

    #[test]
    fn force_refresh_unauthorized_when_no_refresh_token() {
        // Row exists but refresh_token is NULL → still unrecoverable.
        let err = refresh_token_from_row(Some((None, None))).unwrap_err();
        assert!(matches!(err, AdapterError::Unauthorized(_)));
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

#[cfg(test)]
mod error_source_tests {
    use std::error::Error as _;

    use axum::http::StatusCode;

    use super::*;
    use crate::adapters::error::ProviderDecodeError;
    use crate::adapters::test_support::{stub_provider, UNREACHABLE_BASE};

    /// A pool that is never connected — `graphql_with_token` takes the token as
    /// an argument, so it never touches the database.
    fn offline_pool() -> DbPool {
        DbPool::connect_lazy("postgres://linear:linear@127.0.0.1:1/linear").expect("lazy pool")
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

        let AdapterError::ProviderUnreachable {
            ref operation,
            ref source,
        } = err
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

        let AdapterError::ProviderResponseInvalid {
            ref operation,
            ref source,
        } = err
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

        assert!(matches!(err, AdapterError::Unauthorized(_)), "got {err:?}");
        assert!(err.source().is_none());
    }
}
