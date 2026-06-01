//! Linear API schemas — request/response DTOs for the typed `/api/v1/linear/*`
//! endpoints. Mutating endpoints accept names OR UUIDs for state/label/project/
//! assignee/cycle; the backend resolves names via Linear's GraphQL API before
//! issuing the underlying mutation. Responses pass through the Linear payload
//! verbatim as `serde_json::Value`.

use serde::{Deserialize, Serialize};

/// `POST /api/v1/linear/issues` — create an issue.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct LinearIssueCreateRequest {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Team key (e.g. `SP`) or UUID. Optional — defaults to the only team
    /// the connection's user can see when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team: Option<String>,
    /// State name (e.g. `Ready`) or UUID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// Project name or UUID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    /// Label names or UUIDs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    /// Assignee email, display name, or UUID. `"me"` resolves to the
    /// authenticated agent (the workspace OAuth viewer).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
    /// Real Linear user to record as the issue reporter. Accepts email,
    /// display name, UUID, or "me". Added as a subscriber and mentioned in
    /// the description (Linear has no settable creator/reporter field).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reporter: Option<String>,
    /// Linear priority: 0 No-priority, 1 Urgent, 2 High, 3 Normal, 4 Low.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<i32>,
    /// T-shirt estimate: 1 (XS), 2 (S), 3 (M), 5 (L), 8 (XL).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimate: Option<f64>,
    /// Due date in `YYYY-MM-DD` format.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due_date: Option<String>,
    /// Parent issue identifier (`SP-123`) or UUID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// Cycle number (`42`) or `"active"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cycle: Option<String>,
    /// Optional template UUID applied before the explicit fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_id: Option<String>,
    /// Task to bind the created issue to. When provided, a `task_bindings`
    /// row is inserted with `subject_kind = "linear_issue"` so subsequent
    /// `POST /tasks/{id}/reply` posts a comment on this issue.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
}

/// `PATCH /api/v1/linear/issues/:id` — update an issue.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct LinearIssueUpdateRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimate: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cycle: Option<String>,
    /// Parent issue identifier or UUID. Pass `"null"` (literal string) or
    /// the JSON `null` value to clear the parent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
}

/// `POST /api/v1/linear/issues/:id/comments` — post a comment.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct LinearCommentCreateRequest {
    pub body: String,
    /// Task to bind the parent issue to. See [`LinearIssueCreateRequest::task_id`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
}

/// `POST /api/v1/linear/issues/:id/labels` — non-destructive add/remove.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct LinearLabelMutationRequest {
    /// Label name or UUID.
    pub label: String,
    /// `add` or `remove`.
    pub op: LinearLabelOp,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum LinearLabelOp {
    Add,
    Remove,
}

/// `POST /api/v1/linear/graphql` — raw passthrough.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct LinearGraphqlRequest {
    pub query: String,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variables: Option<serde_json::Value>,
}

/// Common envelope for all typed responses. `data` is the raw Linear
/// subobject (e.g. issue, comment); `binding_id` is set only when the
/// request included a `task_id` and a `task_bindings` row was inserted.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct LinearIssueResponse {
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub data: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding_id: Option<String>,
}

/// Response for label add/remove (no body content).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct LinearOkResponse {
    pub ok: bool,
}

/// Response for `/lookup`. Returns at most one match (the first
/// case-insensitive name hit).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct LinearLookupResponse {
    pub id: Option<String>,
    pub name: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<serde_json::Value>,
}

/// Query params for `GET /api/v1/linear/lookup`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::IntoParams))]
#[serde(rename_all = "camelCase")]
pub struct LinearLookupParams {
    /// One of: `state`, `label`, `project`, `user`, `team`.
    pub kind: String,
    pub name: String,
}

/// A comment on a Linear issue, fetched via GraphQL. Used both by the CLI
/// surface and the Agents runtime when assembling inbound context.
#[derive(Debug, Clone, Deserialize)]
pub struct LinearIssueComment {
    pub id: String,
    pub body: String,
    #[serde(rename = "createdAt")]
    pub created_at: chrono::DateTime<chrono::Utc>,
    #[serde(default)]
    pub user: Option<LinearCommentUser>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LinearCommentUser {
    pub id: String,
    pub name: String,
}

/// Query params for `GET /api/v1/linear/issues/:id`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(utoipa::IntoParams))]
#[serde(rename_all = "camelCase")]
pub struct LinearConnectionParams {
    /// Override the default Linear connection. Required when the caller has
    /// access to more than one Linear workspace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection_id: Option<String>,
}
