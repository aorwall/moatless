//! Typed `/api/v1/linear/*` API surface backed by the workspace OAuth token.
//!
//! These endpoints let an authenticated CLI/operator create and update issues,
//! comment, and resolve names to UUIDs without holding a personal
//! `LINEAR_API_KEY`. All operations are attributed to the installed Linear
//! agent (the `actor=app` OAuth viewer), not the calling user.
//!
//! The handlers also auto-create `task_bindings` rows when a `task_id` is
//! provided on issue create / comment so subsequent `POST /tasks/{id}/reply`
//! routes back to the same Linear issue via `commentCreate`.
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
use crate::adapters::models::AdapterConnection;
use crate::adapters::AdapterKind;
use crate::bindings::runtime::upsert_binding;
use crate::extractors::AuthUser;
use crate::scope::ScopeContext;
use crate::state::AppState;

use super::adapter::LinearAdapter;
use super::agent_api::{LinearAgentApi, LookupKind};
use super::SUBJECT_KIND_ISSUE;

/// Origin recorded on `task_bindings.origin` for bindings created by these
/// CLI-driven endpoints. Distinct from the inbound-webhook `ingress`.
const ORIGIN_TASK_OWNED: &str = "task_owned";

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
        (status = 404, description = "Not found", body = crate::error::ErrorBody),
    )
)]
pub async fn get_issue(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Query(params): Query<LinearConnectionParams>,
) -> Result<Json<LinearIssueResponse>, AdapterError> {
    let ctx = auth.scope_context();
    let (_connection, api) = resolve_linear_api(&state, &ctx, params.connection_id.as_deref()).await?;
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
        (status = 400, description = "Bad request", body = crate::error::ErrorBody),
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
    let assignee_id = resolve_assignee(&api, req.assignee.as_deref()).await?;
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

    let binding_id = if let Some(task_id) = req.task_id.as_deref() {
        Some(create_issue_binding(&state, &connection, task_id, &issue).await?)
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
        (status = 400, description = "Bad request", body = crate::error::ErrorBody),
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
        other => resolve_optional_issue(&api, other).await?.map(Value::String),
    };

    let cycle_id = match req.cycle.as_deref() {
        Some(c) => {
            // Need the team for a cycle lookup — fetch the issue to derive it.
            let issue = api.get_issue(&id).await?;
            let team_id = issue
                .pointer("/team/id")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    AdapterError::AdapterApi("Issue is missing team.id (cannot resolve cycle)".into())
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
        (status = 400, description = "Bad request", body = crate::error::ErrorBody),
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

    let comment = api.create_comment(&issue_uuid, &req.body).await?;

    let binding_id = if let Some(task_id) = req.task_id.as_deref() {
        Some(create_issue_binding(&state, &connection, task_id, &issue).await?)
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
        (status = 400, description = "Bad request", body = crate::error::ErrorBody),
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

    let mut body = json!({ "query": req.query });
    if let Some(vars) = req.variables {
        body["variables"] = vars;
    }
    let value = api.graphql(body).await?;
    Ok(Json(value))
}

#[utoipa::path(
    get,
    path = "/api/v1/linear/lookup",
    tag = "linear",
    params(LinearLookupParams, LinearConnectionParams),
    responses(
        (status = 200, description = "Lookup result", body = LinearLookupResponse),
        (status = 400, description = "Bad kind", body = crate::error::ErrorBody),
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
    let providers = state
        .adapters
        .as_ref()
        .ok_or_else(|| AdapterError::AdapterNotConfigured("linear".into()))?;
    let provider = providers
        .get(AdapterKind::Linear)
        .ok_or_else(|| AdapterError::AdapterNotConfigured("linear".into()))?;
    let linear = provider
        .as_any()
        .downcast_ref::<LinearAdapter>()
        .ok_or_else(|| AdapterError::Internal("Linear adapter downcast failed".into()))?;
    let api = linear.agent_api_for_org(&connection.scope_value);
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
    let lookup = api.lookup(kind, value).await?.ok_or_else(|| {
        AdapterError::NotFound(format!("Linear {kind:?} `{value}` not found"))
    })?;
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
    if let Some(id) = by_key.pointer("/data/teams/nodes/0/id").and_then(Value::as_str) {
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

/// Resolve assignee: `me` → viewer.id; UUID → passthrough; email or display
/// name → user lookup.
async fn resolve_assignee(
    api: &LinearAgentApi,
    value: Option<&str>,
) -> Result<Option<String>, AdapterError> {
    let Some(value) = value else { return Ok(None) };
    if value == "me" {
        return Ok(Some(api.viewer().await?.id));
    }
    if looks_like_uuid(value) {
        return Ok(Some(value.to_string()));
    }
    // Try email match first, then fall back to display-name lookup.
    let by_email = api
        .graphql(json!({
            "query": "query($email: String!) { users(filter: { email: { eq: $email } }, first: 1) { nodes { id } } }",
            "variables": { "email": value }
        }))
        .await?;
    if let Some(id) = by_email
        .pointer("/data/users/nodes/0/id")
        .and_then(Value::as_str)
    {
        return Ok(Some(id.to_string()));
    }
    let lookup = api
        .lookup(LookupKind::User, value)
        .await?
        .ok_or_else(|| AdapterError::NotFound(format!("Linear user `{value}` not found")))?;
    Ok(Some(lookup.id))
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
        AdapterError::BadRequest(format!("Invalid cycle value `{value}` (use a number or `active`)"))
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
        ORIGIN_TASK_OWNED,
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
