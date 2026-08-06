//! Typed `/api/v1/linear/*` API surface backed by the workspace OAuth token.
//!
//! These endpoints let an authenticated CLI/operator create and update issues,
//! comment, and resolve names to UUIDs without holding a personal
//! `LINEAR_API_KEY`. All operations are attributed to the installed Linear
//! agent (the `actor=app` OAuth viewer), not the calling user.
//!
//! When a `task_id` is provided the handlers record a Moatless attachment on
//! the issue for traceability. They create a reply `task_bindings` row only on
//! explicit opt-in (`bind = true`) — or, for a comment, when the issue is
//! already bound — so that merely filing a ticket does not silently turn it
//! into a permanent reply channel.
//!
//! A raw GraphQL passthrough (`POST /api/v1/linear/graphql`) exists as the
//! safety valve for operations not yet wrapped by a typed endpoint.

use axum::extract::{Path, Query, State};
use axum::Json;
use moatless_api_schemas::linear::{
    LinearCommentCreateRequest, LinearConnectionParams, LinearGraphqlRequest,
    LinearIssueCreateRequest, LinearIssueResponse, LinearIssueUpdateRequest,
    LinearLabelMutationRequest, LinearLabelOp, LinearLookupParams, LinearLookupResponse,
    LinearOkResponse,
};
use serde_json::{json, Value};
use tracing::warn;

use crate::adapters::common::{ensure_task_access, resolve_connection};
use crate::adapters::error::AdapterError;
use crate::adapters::models::{AdapterConnection, TASK_OWNED_BINDING_ORIGIN};
use crate::adapters::AdapterKind;
use crate::bindings::runtime::upsert_binding;
use crate::extractors::AuthUser;
use crate::state::AppState;
use moatless_backend_core::scope::ScopeContext;

use super::agent_api::{LinearAgentApi, LookupKind};
use super::SUBJECT_KIND_ISSUE;

const LINEAR_PROVIDER: &str = "linear";

// ---------------------------------------------------------------------------
// Issue: GET / CREATE / UPDATE
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/api/v1/linear/issues/{id}",
    tag = "linear",
    params(
        ("id" = String, Path, description = "Issue identifier (SP-123) or UUID"),
        LinearConnectionParams,
    ),
    responses(
        (status = 200, description = "Issue", body = LinearIssueResponse),
        (status = 404, description = "Not found", body = moatless_core::error::ErrorBody),
    )
)]
pub async fn get_issue(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Query(params): Query<LinearConnectionParams>,
) -> Result<Json<LinearIssueResponse>, AdapterError> {
    let ctx = auth.scope_context();
    let (_connection, api) =
        resolve_linear_api(&state, &ctx, params.connection_id.as_deref()).await?;
    let data = api.get_issue(&id).await?;
    Ok(Json(LinearIssueResponse {
        data,
        binding_id: None,
    }))
}

#[utoipa::path(
    post,
    path = "/api/v1/linear/issues",
    tag = "linear",
    params(LinearConnectionParams),
    request_body = LinearIssueCreateRequest,
    responses(
        (status = 200, description = "Issue created", body = LinearIssueResponse),
        (status = 400, description = "Bad request", body = moatless_core::error::ErrorBody),
    )
)]
pub async fn create_issue(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(params): Query<LinearConnectionParams>,
    Json(req): Json<LinearIssueCreateRequest>,
) -> Result<Json<LinearIssueResponse>, AdapterError> {
    let ctx = auth.scope_context();
    let (connection, api) =
        resolve_linear_api(&state, &ctx, params.connection_id.as_deref()).await?;

    if let Some(task_id) = req.task_id.as_deref() {
        ensure_task_access(&state, &ctx, task_id).await?;
    }

    // Resolve names → UUIDs.
    let team_id = match req.team.as_deref() {
        Some(t) => Some(resolve_team_id(&api, t).await?),
        None => None,
    };
    let team_id = match team_id {
        Some(id) => id,
        None => default_team_id(&api).await?,
    };
    let state_id = resolve_optional(&api, LookupKind::State, req.state.as_deref()).await?;
    let project_id = resolve_optional(&api, LookupKind::Project, req.project.as_deref()).await?;
    // Explicit assignee hard-fails on miss; when omitted, default to the
    // calling user's email (best-effort — never fails issue creation).
    let assignee_id = match req.assignee.as_deref() {
        Some(value) => Some(resolve_user_required(&api, value).await?.id),
        None => default_assignee_from_caller(&state, &api, &auth.user_id.to_string()).await,
    };
    // Reporter (records who requested the issue) hard-fails on an explicit miss.
    let reporter = match req.reporter.as_deref() {
        Some(value) => Some(resolve_user_required(&api, value).await?),
        None => None,
    };
    let parent_id = resolve_optional_issue(&api, req.parent.as_deref()).await?;
    let cycle_id = resolve_cycle_id(&api, &team_id, req.cycle.as_deref()).await?;
    let mut label_ids: Vec<String> = Vec::with_capacity(req.labels.len());
    for name in &req.labels {
        label_ids.push(resolve_required(&api, LookupKind::Label, name).await?);
    }

    let mut input = json!({
        "teamId": team_id,
        "title": req.title,
    });
    // Prepend a `Reported by <profile-url>` line (renders as a real user
    // mention) whether or not a description was supplied. Also subscribe the
    // reporter so they get notified and the issue is filterable by them.
    let description = match (reporter.as_ref(), req.description) {
        (Some(r), Some(d)) => Some(format!("Reported by {}\n\n{d}", r.url)),
        (Some(r), None) => Some(format!("Reported by {}", r.url)),
        (None, d) => d,
    };
    if let Some(d) = description {
        input["description"] = json!(d);
    }
    if let Some(r) = reporter.as_ref() {
        input["subscriberIds"] = json!([r.id]);
    }
    if let Some(s) = state_id {
        input["stateId"] = json!(s);
    }
    if let Some(p) = project_id {
        input["projectId"] = json!(p);
    }
    if let Some(a) = assignee_id {
        input["assigneeId"] = json!(a);
    }
    if let Some(p) = req.priority {
        input["priority"] = json!(p);
    }
    if let Some(e) = req.estimate {
        input["estimate"] = json!(e);
    }
    if let Some(d) = req.due_date {
        input["dueDate"] = json!(d);
    }
    if let Some(p) = parent_id {
        input["parentId"] = json!(p);
    }
    if let Some(c) = cycle_id {
        input["cycleId"] = json!(c);
    }
    if !label_ids.is_empty() {
        input["labelIds"] = json!(label_ids);
    }
    if let Some(t) = req.template_id {
        input["templateId"] = json!(t);
    }

    let issue = api.create_issue(input).await?;

    // Filing an issue records a traceability attachment but does not make it a
    // reply target unless the caller explicitly opts in with `bind = true`.
    let binding_id = if let Some(task_id) = req.task_id.as_deref() {
        attach_moatless_task(&state, &api, &issue, task_id).await;
        if req.bind == Some(true) {
            Some(create_issue_binding(&state, &connection, task_id, &issue).await?)
        } else {
            None
        }
    } else {
        None
    };

    Ok(Json(LinearIssueResponse {
        data: issue,
        binding_id,
    }))
}

#[utoipa::path(
    patch,
    path = "/api/v1/linear/issues/{id}",
    tag = "linear",
    params(
        ("id" = String, Path, description = "Issue identifier or UUID"),
        LinearConnectionParams,
    ),
    request_body = LinearIssueUpdateRequest,
    responses(
        (status = 200, description = "Issue updated", body = LinearIssueResponse),
        (status = 400, description = "Bad request", body = moatless_core::error::ErrorBody),
    )
)]
pub async fn update_issue(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Query(params): Query<LinearConnectionParams>,
    Json(req): Json<LinearIssueUpdateRequest>,
) -> Result<Json<LinearIssueResponse>, AdapterError> {
    let ctx = auth.scope_context();
    let (_connection, api) =
        resolve_linear_api(&state, &ctx, params.connection_id.as_deref()).await?;

    // Resolve names. The team comes from the issue itself if a cycle is set.
    let state_id = resolve_optional(&api, LookupKind::State, req.state.as_deref()).await?;
    let project_id = resolve_optional(&api, LookupKind::Project, req.project.as_deref()).await?;
    let assignee_id = resolve_assignee(&api, req.assignee.as_deref()).await?;

    let parent_id = match req.parent.as_deref() {
        Some("null") | Some("") => Some(Value::Null),
        other => resolve_optional_issue(&api, other)
            .await?
            .map(Value::String),
    };

    let cycle_id = match req.cycle.as_deref() {
        Some(c) => {
            // Need the team for a cycle lookup — fetch the issue to derive it.
            let issue = api.get_issue(&id).await?;
            let team_id = issue
                .pointer("/team/id")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    AdapterError::AdapterApi(
                        "Issue is missing team.id (cannot resolve cycle)".into(),
                    )
                })?
                .to_string();
            resolve_cycle_id(&api, &team_id, Some(c)).await?
        }
        None => None,
    };

    let mut input = json!({});
    if let Some(t) = req.title {
        input["title"] = json!(t);
    }
    if let Some(d) = req.description {
        input["description"] = json!(d);
    }
    if let Some(s) = state_id {
        input["stateId"] = json!(s);
    }
    if let Some(p) = project_id {
        input["projectId"] = json!(p);
    }
    if let Some(a) = assignee_id {
        input["assigneeId"] = json!(a);
    }
    if let Some(p) = req.priority {
        input["priority"] = json!(p);
    }
    if let Some(e) = req.estimate {
        input["estimate"] = json!(e);
    }
    if let Some(d) = req.due_date {
        input["dueDate"] = json!(d);
    }
    if let Some(c) = cycle_id {
        input["cycleId"] = json!(c);
    }
    if let Some(p) = parent_id {
        input["parentId"] = p;
    }

    if input.as_object().is_none_or(serde_json::Map::is_empty) {
        return Err(AdapterError::BadRequest(
            "Update request had no fields to change".into(),
        ));
    }

    let issue = api.update_issue(&id, input).await?;
    Ok(Json(LinearIssueResponse {
        data: issue,
        binding_id: None,
    }))
}

// ---------------------------------------------------------------------------
// Comments + labels
// ---------------------------------------------------------------------------

#[utoipa::path(
    post,
    path = "/api/v1/linear/issues/{id}/comments",
    tag = "linear",
    params(
        ("id" = String, Path, description = "Issue identifier or UUID"),
        LinearConnectionParams,
    ),
    request_body = LinearCommentCreateRequest,
    responses(
        (status = 200, description = "Comment posted", body = LinearIssueResponse),
        (status = 400, description = "Bad request", body = moatless_core::error::ErrorBody),
    )
)]
pub async fn create_comment(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Query(params): Query<LinearConnectionParams>,
    Json(req): Json<LinearCommentCreateRequest>,
) -> Result<Json<LinearIssueResponse>, AdapterError> {
    let ctx = auth.scope_context();
    let (connection, api) =
        resolve_linear_api(&state, &ctx, params.connection_id.as_deref()).await?;

    if let Some(task_id) = req.task_id.as_deref() {
        ensure_task_access(&state, &ctx, task_id).await?;
    }

    // commentCreate requires the issue UUID, not the SP-123 identifier.
    let issue = api.get_issue(&id).await?;
    let issue_uuid = issue
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| AdapterError::NotFound(format!("Linear issue {id} missing id")))?
        .to_string();

    let body = match req.task_id.as_deref() {
        Some(task_id) => {
            match crate::bindings::footer::task_comment_footer(
                state.tasks.as_ref(),
                &state.config.auth.frontend_url,
                task_id,
            )
            .await
            {
                Some(footer) => format!("{}{}", req.body, footer),
                None => req.body.clone(),
            }
        }
        None => req.body.clone(),
    };
    let comment = api.create_comment(&issue_uuid, &body).await?;

    let binding_id = if let Some(task_id) = req.task_id.as_deref() {
        attach_moatless_task(&state, &api, &issue, task_id).await;

        // A comment refreshes the binding when the issue is already a reply
        // target (the agent is continuing an established thread). A proactive
        // comment on an unbound issue only binds when the caller opts in.
        let already_bound = state
            .task_bindings
            .find_by_task_id_and_subject(task_id, LINEAR_PROVIDER, SUBJECT_KIND_ISSUE, &issue_uuid)
            .await?
            .is_some();

        if req.bind == Some(true) || already_bound {
            Some(create_issue_binding(&state, &connection, task_id, &issue).await?)
        } else {
            None
        }
    } else {
        None
    };

    Ok(Json(LinearIssueResponse {
        data: comment,
        binding_id,
    }))
}

#[utoipa::path(
    post,
    path = "/api/v1/linear/issues/{id}/labels",
    tag = "linear",
    params(
        ("id" = String, Path, description = "Issue identifier or UUID"),
        LinearConnectionParams,
    ),
    request_body = LinearLabelMutationRequest,
    responses(
        (status = 200, description = "Label mutated", body = LinearOkResponse),
        (status = 400, description = "Bad request", body = moatless_core::error::ErrorBody),
    )
)]
pub async fn mutate_label(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Query(params): Query<LinearConnectionParams>,
    Json(req): Json<LinearLabelMutationRequest>,
) -> Result<Json<LinearOkResponse>, AdapterError> {
    let ctx = auth.scope_context();
    let (_connection, api) =
        resolve_linear_api(&state, &ctx, params.connection_id.as_deref()).await?;

    let issue = api.get_issue(&id).await?;
    let issue_uuid = issue
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| AdapterError::NotFound(format!("Linear issue {id} missing id")))?
        .to_string();
    let label_id = resolve_required(&api, LookupKind::Label, &req.label).await?;

    match req.op {
        LinearLabelOp::Add => api.issue_add_label(&issue_uuid, &label_id).await?,
        LinearLabelOp::Remove => api.issue_remove_label(&issue_uuid, &label_id).await?,
    }
    Ok(Json(LinearOkResponse { ok: true }))
}

// ---------------------------------------------------------------------------
// GraphQL passthrough + lookup
// ---------------------------------------------------------------------------

#[utoipa::path(
    post,
    path = "/api/v1/linear/graphql",
    tag = "linear",
    params(LinearConnectionParams),
    request_body = LinearGraphqlRequest,
    responses((status = 200, description = "Raw GraphQL response", body = Object))
)]
pub async fn graphql_passthrough(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(params): Query<LinearConnectionParams>,
    Json(req): Json<LinearGraphqlRequest>,
) -> Result<Json<Value>, AdapterError> {
    let ctx = auth.scope_context();
    let (_connection, api) =
        resolve_linear_api(&state, &ctx, params.connection_id.as_deref()).await?;

    // Callers (esp. agents) sometimes pass the full `{"query": ..., "variables": ...}`
    // envelope as the `query` field. Peel one layer so Linear doesn't see the
    // envelope text as the GraphQL document.
    let (query_text, inner_vars) = unwrap_graphql_envelope(&req.query);

    let mut body = json!({ "query": query_text });
    if let Some(vars) = req.variables.or(inner_vars) {
        body["variables"] = vars;
    }
    let value = api.graphql(body).await?;
    Ok(Json(value))
}

/// Tolerate callers that wrap a full `{query, variables}` envelope inside the
/// `query` field. Returns the inner GraphQL text and any inner variables.
fn unwrap_graphql_envelope(raw: &str) -> (String, Option<Value>) {
    let trimmed = raw.trim_start();
    if !trimmed.starts_with('{') {
        return (raw.to_string(), None);
    }
    let Ok(Value::Object(map)) = serde_json::from_str::<Value>(trimmed) else {
        return (raw.to_string(), None);
    };
    let Some(inner) = map.get("query").and_then(Value::as_str) else {
        return (raw.to_string(), None);
    };
    warn!(
        "linear graphql passthrough: caller wrapped a JSON envelope inside `query`; \
         unwrapping. Fix the caller to send raw GraphQL text in `query`."
    );
    (inner.to_string(), map.get("variables").cloned())
}

#[utoipa::path(
    get,
    path = "/api/v1/linear/lookup",
    tag = "linear",
    params(LinearLookupParams, LinearConnectionParams),
    responses(
        (status = 200, description = "Lookup result", body = LinearLookupResponse),
        (status = 400, description = "Bad kind", body = moatless_core::error::ErrorBody),
    )
)]
pub async fn lookup(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(params): Query<LinearLookupParams>,
    Query(conn_params): Query<LinearConnectionParams>,
) -> Result<Json<LinearLookupResponse>, AdapterError> {
    let ctx = auth.scope_context();
    let (_connection, api) =
        resolve_linear_api(&state, &ctx, conn_params.connection_id.as_deref()).await?;

    let kind = LookupKind::parse(&params.kind).ok_or_else(|| {
        AdapterError::BadRequest(format!(
            "Unknown lookup kind `{}` (use state, label, project, user, team)",
            params.kind
        ))
    })?;
    let result = api.lookup(kind, &params.name).await?;
    Ok(Json(match result {
        Some(r) => LinearLookupResponse {
            id: Some(r.id),
            name: Some(r.name),
            raw: Some(r.raw),
        },
        None => LinearLookupResponse {
            id: None,
            name: None,
            raw: None,
        },
    }))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Resolve the caller's Linear connection (defaulting to the single
/// accessible one) and build a [`LinearAgentApi`] for that workspace.
async fn resolve_linear_api(
    state: &AppState,
    ctx: &ScopeContext,
    connection_id: Option<&str>,
) -> Result<(AdapterConnection, LinearAgentApi), AdapterError> {
    let connection = resolve_connection(state, ctx, AdapterKind::Linear, connection_id).await?;
    let linear = state
        .provider_handles
        .linear
        .as_ref()
        .ok_or_else(|| AdapterError::AdapterNotConfigured("linear".into()))?;
    let api = linear.agent_api_for_connection(&connection).await?;
    Ok((connection, api))
}

/// Heuristic: a Linear UUID is 36 chars with dashes at positions 8/13/18/23.
/// Identifiers like `SP-123` start with letters and a dash. Anything that
/// parses as a UUID is treated as already-resolved.
fn looks_like_uuid(s: &str) -> bool {
    uuid::Uuid::parse_str(s).is_ok()
}

/// Resolve a value that may be either a UUID (passthrough) or a name
/// (lookup). Returns `Ok(None)` for `None` input.
async fn resolve_optional(
    api: &LinearAgentApi,
    kind: LookupKind,
    value: Option<&str>,
) -> Result<Option<String>, AdapterError> {
    let Some(value) = value else { return Ok(None) };
    if looks_like_uuid(value) {
        return Ok(Some(value.to_string()));
    }
    let lookup = api
        .lookup(kind, value)
        .await?
        .ok_or_else(|| AdapterError::NotFound(format!("Linear {kind:?} `{value}` not found")))?;
    Ok(Some(lookup.id))
}

/// Required variant of [`resolve_optional`].
async fn resolve_required(
    api: &LinearAgentApi,
    kind: LookupKind,
    value: &str,
) -> Result<String, AdapterError> {
    resolve_optional(api, kind, Some(value))
        .await?
        .ok_or_else(|| AdapterError::Internal("resolve_optional returned None unexpectedly".into()))
}

/// Resolve a team value (key like `SP`, or UUID). Lookup uses the team's
/// `name` field, so a team key like `SP` is checked via a key filter first.
async fn resolve_team_id(api: &LinearAgentApi, value: &str) -> Result<String, AdapterError> {
    if looks_like_uuid(value) {
        return Ok(value.to_string());
    }
    // Try by key first (e.g. `SP`), then fall back to name lookup.
    let by_key = api
        .graphql(json!({
            "query": "query($key: String!) { teams(filter: { key: { eqIgnoreCase: $key } }, first: 1) { nodes { id key name } } }",
            "variables": { "key": value }
        }))
        .await?;
    if let Some(id) = by_key
        .pointer("/data/teams/nodes/0/id")
        .and_then(Value::as_str)
    {
        return Ok(id.to_string());
    }
    let lookup = api
        .lookup(LookupKind::Team, value)
        .await?
        .ok_or_else(|| AdapterError::NotFound(format!("Linear team `{value}` not found")))?;
    Ok(lookup.id)
}

/// Return the default team id when the caller didn't specify one. If
/// exactly one team is accessible we use it; otherwise surface the list
/// and require a `--team` flag.
async fn default_team_id(api: &LinearAgentApi) -> Result<String, AdapterError> {
    let value = api
        .graphql(json!({
            "query": "query { teams(first: 5) { nodes { id key name } } }"
        }))
        .await?;
    let nodes = value
        .pointer("/data/teams/nodes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    match nodes.len() {
        0 => Err(AdapterError::BadRequest(
            "No Linear teams accessible — pass `team`".into(),
        )),
        1 => nodes[0]
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| AdapterError::AdapterApi("Linear team missing id".into())),
        _ => {
            let keys: Vec<String> = nodes
                .iter()
                .filter_map(|n| n.get("key").and_then(Value::as_str).map(str::to_string))
                .collect();
            Err(AdapterError::BadRequest(format!(
                "Multiple Linear teams accessible: [{}]. Pass `team` to choose.",
                keys.join(", ")
            )))
        }
    }
}

/// A resolved Linear user with the fields needed to wire it into an issue as
/// assignee (`id`) or reporter (`id` for the subscriber, `url` for the
/// description mention).
struct ResolvedUser {
    id: String,
    url: String,
}

/// Build a [`ResolvedUser`] from a GraphQL user node selecting `id url`.
fn user_from_node(node: &Value) -> Option<ResolvedUser> {
    let id = node.get("id").and_then(Value::as_str)?.to_string();
    let url = node
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    Some(ResolvedUser { id, url })
}

/// Resolve a user reference (`me`, UUID, email, or display name) to a Linear
/// user. Returns `Ok(None)` when no user matches — callers decide whether a
/// miss is fatal (explicit `--assignee`/`--reporter`) or skippable (the
/// best-effort assignee default).
async fn resolve_user_opt(
    api: &LinearAgentApi,
    value: &str,
) -> Result<Option<ResolvedUser>, AdapterError> {
    if value == "me" {
        let viewer = api
            .graphql(json!({ "query": "query { viewer { id url } }" }))
            .await?;
        return Ok(viewer.pointer("/data/viewer").and_then(user_from_node));
    }
    if looks_like_uuid(value) {
        let by_id = api
            .graphql(json!({
                "query": "query($id: String!) { user(id: $id) { id url } }",
                "variables": { "id": value }
            }))
            .await?;
        return Ok(by_id.pointer("/data/user").and_then(user_from_node));
    }
    // Try email match first, then fall back to display-name lookup.
    let by_email = api
        .graphql(json!({
            "query": "query($email: String!) { users(filter: { email: { eq: $email } }, first: 1) { nodes { id url } } }",
            "variables": { "email": value }
        }))
        .await?;
    if let Some(user) = by_email
        .pointer("/data/users/nodes/0")
        .and_then(user_from_node)
    {
        return Ok(Some(user));
    }
    let Some(lookup) = api.lookup(LookupKind::User, value).await? else {
        return Ok(None);
    };
    let by_id = api
        .graphql(json!({
            "query": "query($id: String!) { user(id: $id) { id url } }",
            "variables": { "id": lookup.id }
        }))
        .await?;
    Ok(by_id.pointer("/data/user").and_then(user_from_node))
}

/// Required variant of [`resolve_user_opt`] — hard-fails with `NotFound` when
/// no user matches.
async fn resolve_user_required(
    api: &LinearAgentApi,
    value: &str,
) -> Result<ResolvedUser, AdapterError> {
    resolve_user_opt(api, value)
        .await?
        .ok_or_else(|| AdapterError::NotFound(format!("Linear user `{value}` not found")))
}

/// Resolve assignee: `me` → viewer.id; UUID → passthrough; email or display
/// name → user lookup. Hard-fails on miss (used by issue update and explicit
/// create assignees).
async fn resolve_assignee(
    api: &LinearAgentApi,
    value: Option<&str>,
) -> Result<Option<String>, AdapterError> {
    let Some(value) = value else { return Ok(None) };
    Ok(Some(resolve_user_required(api, value).await?.id))
}

/// Best-effort assignee default: resolve the calling moat user's email to a
/// Linear user. Returns `None` (and warns) when the caller has no email on
/// file or the email doesn't match a Linear member — never fails issue
/// creation.
async fn default_assignee_from_caller(
    state: &AppState,
    api: &LinearAgentApi,
    user_id: &str,
) -> Option<String> {
    let Ok(uid) = user_id.parse::<uuid::Uuid>() else {
        return None;
    };
    let user = match state.users.get_user(&uid).await {
        Ok(Some(u)) => u,
        Ok(None) => {
            warn!(
                user_id,
                "assignee default: caller not found; leaving issue unassigned"
            );
            return None;
        }
        Err(e) => {
            warn!(error = %e, user_id, "assignee default: failed to load caller; leaving issue unassigned");
            return None;
        }
    };
    let Some(email) = user.email else {
        warn!(
            user_id,
            "assignee default: caller has no email; leaving issue unassigned"
        );
        return None;
    };
    match resolve_user_opt(api, &email).await {
        Ok(Some(u)) => Some(u.id),
        Ok(None) => {
            warn!(
                user_id,
                email,
                "assignee default: no Linear user matches caller email; leaving issue unassigned"
            );
            None
        }
        Err(e) => {
            warn!(error = %e, user_id, email, "assignee default: lookup failed; leaving issue unassigned");
            None
        }
    }
}

/// Resolve a parent value: UUID → as-is; identifier (SP-123) → fetch the
/// issue and return its UUID.
async fn resolve_optional_issue(
    api: &LinearAgentApi,
    value: Option<&str>,
) -> Result<Option<String>, AdapterError> {
    let Some(value) = value else { return Ok(None) };
    if looks_like_uuid(value) {
        return Ok(Some(value.to_string()));
    }
    let issue = api.get_issue(value).await?;
    let id = issue
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| AdapterError::NotFound(format!("Linear issue `{value}` missing id")))?;
    Ok(Some(id.to_string()))
}

/// Resolve a cycle value: UUID → as-is; "active" → the team's active
/// cycle; numeric string → cycle number on the team.
async fn resolve_cycle_id(
    api: &LinearAgentApi,
    team_id: &str,
    value: Option<&str>,
) -> Result<Option<String>, AdapterError> {
    let Some(value) = value else { return Ok(None) };
    if looks_like_uuid(value) {
        return Ok(Some(value.to_string()));
    }
    if value == "active" {
        let active = api
            .graphql(json!({
                "query": "query($id: String!) { team(id: $id) { activeCycle { id } } }",
                "variables": { "id": team_id }
            }))
            .await?;
        return Ok(active
            .pointer("/data/team/activeCycle/id")
            .and_then(Value::as_str)
            .map(str::to_string));
    }
    let number: i64 = value.parse().map_err(|_| {
        AdapterError::BadRequest(format!(
            "Invalid cycle value `{value}` (use a number or `active`)"
        ))
    })?;
    let by_number = api
        .graphql(json!({
            "query": "query($team: ID!, $number: Float!) { cycles(filter: { team: { id: { eq: $team } }, number: { eq: $number } }, first: 1) { nodes { id number } } }",
            "variables": { "team": team_id, "number": number }
        }))
        .await?;
    Ok(by_number
        .pointer("/data/cycles/nodes/0/id")
        .and_then(Value::as_str)
        .map(str::to_string))
}

/// Persist a `task_bindings` row pointing the given task at the Linear
/// issue. Used for both create and comment auto-binding.
async fn create_issue_binding(
    state: &AppState,
    connection: &AdapterConnection,
    task_id: &str,
    issue: &Value,
) -> Result<String, AdapterError> {
    let issue_uuid = issue
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| AdapterError::AdapterApi("Linear issue missing id for binding".into()))?
        .to_string();
    let identifier = issue
        .get("identifier")
        .and_then(Value::as_str)
        .map(str::to_string);
    let url = issue.get("url").and_then(Value::as_str).map(str::to_string);

    let mut destination_config = json!({ "issue_id": issue_uuid });
    if let Some(ref ident) = identifier {
        destination_config["issue_identifier"] = json!(ident);
    }
    if let Some(ref u) = url {
        destination_config["url"] = json!(u);
    }

    let binding = upsert_binding(
        state,
        connection,
        None,
        task_id,
        LINEAR_PROVIDER,
        SUBJECT_KIND_ISSUE,
        &issue_uuid,
        destination_config,
        // The task filed this issue, so its binding is a producer one — that is
        // what makes it hear replies inside threads on its own issue.
        TASK_OWNED_BINDING_ORIGIN,
        None,
    )
    .await
    .inspect_err(|e| {
        warn!(
            error = %e,
            task_id = %task_id,
            issue = %issue_uuid,
            "Failed to upsert linear_issue binding"
        );
    })?;
    Ok(binding.id)
}

/// Attach the moatless task URL to the Linear issue as a "source" pill.
/// Idempotent on (issue_id, url) — safe to call multiple times. Failures
/// are logged and swallowed so they don't block the surrounding mutation
/// (the issue is already created / commented on).
pub(crate) async fn attach_moatless_task(
    state: &AppState,
    api: &LinearAgentApi,
    issue: &Value,
    task_id: &str,
) {
    let Some(issue_uuid) = issue.get("id").and_then(Value::as_str) else {
        warn!(task_id = %task_id, "attach_moatless_task: issue missing id");
        return;
    };
    super::issues::attach_parent_task_link(state, api, issue_uuid, task_id).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unwrap_envelope_passes_through_bare_graphql() {
        let (q, vars) = unwrap_graphql_envelope("query { viewer { name email } }");
        assert_eq!(q, "query { viewer { name email } }");
        assert!(vars.is_none());
    }

    #[test]
    fn unwrap_envelope_passes_through_mutation_with_braces() {
        let raw = "mutation { issueAddLabel(id: \"x\", labelId: \"y\") { success } }";
        let (q, vars) = unwrap_graphql_envelope(raw);
        assert_eq!(q, raw);
        assert!(vars.is_none());
    }

    #[test]
    fn unwrap_envelope_extracts_inner_query() {
        let raw = r#"{"query": "query { viewer { name } }"}"#;
        let (q, vars) = unwrap_graphql_envelope(raw);
        assert_eq!(q, "query { viewer { name } }");
        assert!(vars.is_none());
    }

    #[test]
    fn unwrap_envelope_extracts_inner_query_and_variables() {
        let raw = r#"{"query": "query($id: String!) { issue(id: $id) { id } }", "variables": {"id": "SP-1138"}}"#;
        let (q, vars) = unwrap_graphql_envelope(raw);
        assert_eq!(q, "query($id: String!) { issue(id: $id) { id } }");
        assert_eq!(vars.unwrap(), json!({ "id": "SP-1138" }));
    }

    #[test]
    fn unwrap_envelope_tolerates_leading_whitespace() {
        let raw = "  \n  {\"query\": \"query { viewer { id } }\"}";
        let (q, vars) = unwrap_graphql_envelope(raw);
        assert_eq!(q, "query { viewer { id } }");
        assert!(vars.is_none());
    }

    #[test]
    fn unwrap_envelope_leaves_non_envelope_object_alone() {
        // Looks like JSON but has no `query` field — not an envelope.
        let raw = r#"{"foo": "bar"}"#;
        let (q, vars) = unwrap_graphql_envelope(raw);
        assert_eq!(q, raw);
        assert!(vars.is_none());
    }
}
