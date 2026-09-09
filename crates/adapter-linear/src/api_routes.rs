//! Typed provider HTTP extraction, response adaptation, and OpenAPI metadata.

use super::adapter::LinearAdapter;
use axum::Json;
use axum::extract::{Path, Query, State};
use moatless_adapter_model::error::AdapterError;
use moatless_api_schemas::linear::{
    LinearCommentCreateRequest, LinearConnectionParams, LinearGraphqlRequest,
    LinearIssueCreateRequest, LinearIssueListParams, LinearIssueListResponse,
    LinearIssueRelateRequest, LinearIssueResponse, LinearIssueUpdateRequest,
    LinearLabelMutationRequest, LinearLookupParams, LinearLookupResponse, LinearOkResponse,
};
use moatless_core::http_auth::AuthUser;
use moatless_loops::bindings::operations::Bindings;
use moatless_ports::identity::{ProviderIdentityLink, UserDisplay};
use moatless_ports::task::TaskReader;
use serde_json::Value;
#[cfg(test)]
use serde_json::json;
use std::sync::Arc;
use tracing::warn;

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
        (status = 404, description = "Not found", body = moatless_api_schemas::error::ErrorBody),
    )
)]
pub async fn get_issue(
    State(linear): State<Option<Arc<LinearAdapter>>>,
    auth: AuthUser,
    Path(id): Path<String>,
    Query(params): Query<LinearConnectionParams>,
) -> Result<Json<LinearIssueResponse>, AdapterError> {
    super::issue_commands::get_issue(linear, auth.scope_context(), id, params)
        .await
        .map(Json)
}

#[utoipa::path(
    post,
    path = "/api/v1/linear/issues",
    tag = "linear",
    params(LinearConnectionParams),
    request_body = LinearIssueCreateRequest,
    responses(
        (status = 200, description = "Issue created", body = LinearIssueResponse),
        (status = 400, description = "Bad request", body = moatless_api_schemas::error::ErrorBody),
    )
)]
#[expect(
    clippy::too_many_arguments,
    reason = "an axum handler's width is its extractor list, and each State names \
              exactly one port the create path uses; bundling them would re-bag \
              the AppState these handlers exist to unbag"
)]
pub async fn create_issue(
    State(linear): State<Option<Arc<LinearAdapter>>>,
    State(tasks): State<Arc<dyn TaskReader>>,
    State(users): State<Arc<dyn UserDisplay>>,
    State(identity): State<Arc<dyn ProviderIdentityLink>>,
    State(task_bindings): State<Arc<Bindings>>,
    auth: AuthUser,
    Query(params): Query<LinearConnectionParams>,
    Json(req): Json<LinearIssueCreateRequest>,
) -> Result<Json<LinearIssueResponse>, AdapterError> {
    let ctx = auth.scope_context();
    super::issue_commands::create_issue(
        linear,
        tasks,
        users,
        identity,
        task_bindings,
        ctx,
        auth.login,
        params,
        req,
    )
    .await
    .map(Json)
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
        (status = 400, description = "Bad request", body = moatless_api_schemas::error::ErrorBody),
    )
)]
pub async fn update_issue(
    State(linear): State<Option<Arc<LinearAdapter>>>,
    State(identity): State<Arc<dyn ProviderIdentityLink>>,
    auth: AuthUser,
    Path(id): Path<String>,
    Query(params): Query<LinearConnectionParams>,
    Json(req): Json<LinearIssueUpdateRequest>,
) -> Result<Json<LinearIssueResponse>, AdapterError> {
    super::issue_commands::update_issue(linear, identity, auth.scope_context(), id, params, req)
        .await
        .map(Json)
}

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
        (status = 400, description = "Bad request", body = moatless_api_schemas::error::ErrorBody),
    )
)]
pub async fn create_comment(
    State(linear): State<Option<Arc<LinearAdapter>>>,
    State(tasks): State<Arc<dyn TaskReader>>,
    State(task_bindings): State<Arc<Bindings>>,
    auth: AuthUser,
    Path(id): Path<String>,
    Query(params): Query<LinearConnectionParams>,
    Json(req): Json<LinearCommentCreateRequest>,
) -> Result<Json<LinearIssueResponse>, AdapterError> {
    super::issue_commands::create_comment(
        linear,
        tasks,
        task_bindings,
        auth.scope_context(),
        id,
        params,
        req,
    )
    .await
    .map(Json)
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
        (status = 400, description = "Bad request", body = moatless_api_schemas::error::ErrorBody),
    )
)]
pub async fn mutate_label(
    State(linear): State<Option<Arc<LinearAdapter>>>,
    auth: AuthUser,
    Path(id): Path<String>,
    Query(params): Query<LinearConnectionParams>,
    Json(req): Json<LinearLabelMutationRequest>,
) -> Result<Json<LinearOkResponse>, AdapterError> {
    super::issue_commands::mutate_label(linear, auth.scope_context(), id, params, req)
        .await
        .map(Json)
}

#[utoipa::path(
    post,
    path = "/api/v1/linear/issues/{id}/relations",
    tag = "linear",
    params(
        ("id" = String, Path, description = "Issue identifier or UUID"),
        LinearConnectionParams,
    ),
    request_body = LinearIssueRelateRequest,
    responses(
        (status = 200, description = "Relation created", body = LinearIssueResponse),
        (status = 400, description = "Bad request", body = moatless_api_schemas::error::ErrorBody),
    )
)]
pub async fn relate_issue(
    State(linear): State<Option<Arc<LinearAdapter>>>,
    auth: AuthUser,
    Path(id): Path<String>,
    Query(params): Query<LinearConnectionParams>,
    Json(req): Json<LinearIssueRelateRequest>,
) -> Result<Json<LinearIssueResponse>, AdapterError> {
    super::issue_commands::relate_issue(linear, auth.scope_context(), id, params, req)
        .await
        .map(Json)
}

#[utoipa::path(
    get,
    path = "/api/v1/linear/issues",
    tag = "linear",
    params(LinearIssueListParams),
    responses(
        (status = 200, description = "Matching issues", body = LinearIssueListResponse),
        (status = 400, description = "Bad request", body = moatless_api_schemas::error::ErrorBody),
    )
)]
pub async fn list_issues(
    State(linear): State<Option<Arc<LinearAdapter>>>,
    State(identity): State<Arc<dyn ProviderIdentityLink>>,
    auth: AuthUser,
    Query(params): Query<LinearIssueListParams>,
) -> Result<Json<LinearIssueListResponse>, AdapterError> {
    super::issue_commands::list_issues(linear, identity, auth.scope_context(), params)
        .await
        .map(Json)
}

#[utoipa::path(
    post,
    path = "/api/v1/linear/graphql",
    tag = "linear",
    params(LinearConnectionParams),
    request_body = LinearGraphqlRequest,
    responses((status = 200, description = "Raw GraphQL response", body = Object))
)]
pub async fn graphql_passthrough(
    State(linear): State<Option<Arc<LinearAdapter>>>,
    auth: AuthUser,
    Query(params): Query<LinearConnectionParams>,
    Json(req): Json<LinearGraphqlRequest>,
) -> Result<Json<Value>, AdapterError> {
    let (query, variables) = unwrap_graphql_envelope(&req.query);
    let req = LinearGraphqlRequest {
        query,
        variables: req.variables.or(variables),
    };
    super::issue_commands::graphql_passthrough(linear, auth.scope_context(), params, req)
        .await
        .map(Json)
}

#[utoipa::path(
    get,
    path = "/api/v1/linear/lookup",
    tag = "linear",
    params(LinearLookupParams, LinearConnectionParams),
    responses(
        (status = 200, description = "Lookup result", body = LinearLookupResponse),
        (status = 400, description = "Bad kind", body = moatless_api_schemas::error::ErrorBody),
    )
)]
pub async fn lookup(
    State(linear): State<Option<Arc<LinearAdapter>>>,
    auth: AuthUser,
    Query(params): Query<LinearLookupParams>,
    Query(conn_params): Query<LinearConnectionParams>,
) -> Result<Json<LinearLookupResponse>, AdapterError> {
    super::issue_commands::lookup(linear, auth.scope_context(), params, conn_params)
        .await
        .map(Json)
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
