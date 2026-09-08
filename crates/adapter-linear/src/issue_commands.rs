//! Authorized provider operations and ordered mutation effects.

use std::sync::Arc;

use moatless_api_schemas::linear::{
    LinearCommentCreateRequest, LinearConnectionParams, LinearGraphqlRequest,
    LinearIssueCreateRequest, LinearIssueListParams, LinearIssueListResponse,
    LinearIssueRelateRequest, LinearIssueResponse, LinearIssueUpdateRequest,
    LinearLabelMutationRequest, LinearLabelOp, LinearLookupParams, LinearLookupResponse,
    LinearOkResponse,
};
use serde_json::{json, Value};
use tracing::warn;

use moatless_adapter_model::binding::TASK_OWNED_BINDING_ORIGIN;
use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
use moatless_core::config::Config;
use moatless_loops::adapters::access::ensure_task_access_only;
use moatless_loops::bindings::dao::TaskBindingDao;
use moatless_loops::bindings::runtime::upsert_binding_into;
use moatless_ports::identity::{ProviderIdentityLink, UserDisplay};
use moatless_ports::task::TaskReader;
use moatless_types::adapter::AdapterConnection;

use super::adapter::LinearAdapter;
use super::agent_api::{LinearAgentApi, LookupKind};
use super::auth::{require_linear, resolve_linear_api};
use super::reference_resolution::*;
use super::SUBJECT_KIND_ISSUE;

// ---------------------------------------------------------------------------
// Issue: GET / CREATE / UPDATE
// ---------------------------------------------------------------------------

pub(crate) async fn get_issue(
    linear: Option<Arc<LinearAdapter>>,
    ctx: moatless_types::access_control::ScopeContext,
    id: String,
    params: LinearConnectionParams,
) -> Result<LinearIssueResponse, AdapterError> {
    let linear = require_linear(linear)?;
    let (_connection, api) =
        resolve_linear_api(&linear, &ctx, params.connection_id.as_deref()).await?;
    let data = api.get_issue(&id).await?;
    Ok(LinearIssueResponse {
        data,
        binding_id: None,
    })
}

#[expect(
    clippy::too_many_arguments,
    reason = "the authorized mutation receives its provider, binding, task and \
              caller-identity dependencies separately, matching the handler that \
              extracts them"
)]
pub(crate) async fn create_issue(
    linear: Option<Arc<LinearAdapter>>,
    tasks: Arc<dyn TaskReader>,
    users: Arc<dyn UserDisplay>,
    identity: Arc<dyn ProviderIdentityLink>,
    task_bindings: Arc<dyn TaskBindingDao>,
    ctx: moatless_types::access_control::ScopeContext,
    login: String,
    params: LinearConnectionParams,
    req: LinearIssueCreateRequest,
) -> Result<LinearIssueResponse, AdapterError> {
    let linear = require_linear(linear)?;
    let (connection, api) =
        resolve_linear_api(&linear, &ctx, params.connection_id.as_deref()).await?;

    if let Some(task_id) = req.task_id.as_deref() {
        ensure_task_access_only(tasks.as_ref(), &ctx, task_id).await?;
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
    let state_id = resolve_optional(
        &api,
        LookupKind::State,
        req.state.as_deref(),
        Some(&team_id),
    )
    .await?;
    let project_id = resolve_optional(
        &api,
        LookupKind::Project,
        req.project.as_deref(),
        Some(&team_id),
    )
    .await?;
    // Explicit assignee hard-fails on miss; when omitted, default to the
    // caller (best-effort — never fails issue creation).
    let assignee_id = match req.assignee.as_deref() {
        Some(value) => Some(
            resolve_user_required(&api, identity.as_ref(), value)
                .await?
                .id,
        ),
        None => {
            default_assignee_from_caller(
                users.as_ref(),
                identity.as_ref(),
                &api,
                &ctx.user_id,
                &login,
            )
            .await
        }
    };
    let delegate_id = resolve_delegate(&api, identity.as_ref(), req.delegate.as_deref()).await?;
    // Reporter (records who requested the issue) hard-fails on an explicit miss.
    let reporter = match req.reporter.as_deref() {
        Some(value) => Some(resolve_user_required(&api, identity.as_ref(), value).await?),
        None => None,
    };
    let parent_id = resolve_optional_issue(&api, req.parent.as_deref()).await?;
    let cycle_id = resolve_cycle_id(&api, Some(&team_id), req.cycle.as_deref()).await?;
    let mut label_ids: Vec<String> = Vec::with_capacity(req.labels.len());
    for name in &req.labels {
        label_ids.push(resolve_required(&api, LookupKind::Label, name, Some(&team_id)).await?);
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
    if let Some(d) = delegate_id {
        input["delegateId"] = d;
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
        attach_moatless_task(
            linear.require_config()?,
            tasks.as_ref(),
            &api,
            &issue,
            task_id,
        )
        .await;
        if req.bind == Some(true) {
            Some(create_issue_binding(task_bindings.as_ref(), &connection, task_id, &issue).await?)
        } else {
            None
        }
    } else {
        None
    };

    Ok(LinearIssueResponse {
        data: issue,
        binding_id,
    })
}

pub(crate) async fn update_issue(
    linear: Option<Arc<LinearAdapter>>,
    identity: Arc<dyn ProviderIdentityLink>,
    ctx: moatless_types::access_control::ScopeContext,
    id: String,
    params: LinearConnectionParams,
    req: LinearIssueUpdateRequest,
) -> Result<LinearIssueResponse, AdapterError> {
    let linear = require_linear(linear)?;
    let (_connection, api) =
        resolve_linear_api(&linear, &ctx, params.connection_id.as_deref()).await?;

    // State, project and cycle all resolve against the issue's own team, so one
    // fetch serves all three. Only a name reads it: each resolver returns a UUID
    // untouched, so an update that passes ids makes no extra request.
    let resolves_a_name = [
        req.state.as_deref(),
        req.project.as_deref(),
        req.cycle.as_deref(),
    ]
    .into_iter()
    .flatten()
    .any(|value| !looks_like_uuid(value));
    let team_id = if resolves_a_name {
        Some(issue_team_id(&api, &id).await?)
    } else {
        None
    };
    let state_id = resolve_optional(
        &api,
        LookupKind::State,
        req.state.as_deref(),
        team_id.as_deref(),
    )
    .await?;
    let project_id = resolve_optional(
        &api,
        LookupKind::Project,
        req.project.as_deref(),
        team_id.as_deref(),
    )
    .await?;
    let assignee_id = resolve_assignee(&api, identity.as_ref(), req.assignee.as_deref()).await?;
    let delegate_id = resolve_delegate(&api, identity.as_ref(), req.delegate.as_deref()).await?;

    let parent_id = match req.parent.as_deref() {
        Some("null") | Some("") => Some(Value::Null),
        other => resolve_optional_issue(&api, other)
            .await?
            .map(Value::String),
    };

    let cycle_id = resolve_cycle_id(&api, team_id.as_deref(), req.cycle.as_deref()).await?;

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
    if let Some(d) = delegate_id {
        input["delegateId"] = d;
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
        return Err(SharedAdapterError::BadRequest(
            "Update request had no fields to change".into(),
        )
        .into());
    }

    let issue = api.update_issue(&id, input).await?;
    Ok(LinearIssueResponse {
        data: issue,
        binding_id: None,
    })
}

// ---------------------------------------------------------------------------
// Comments + labels
// ---------------------------------------------------------------------------

pub(crate) async fn create_comment(
    linear: Option<Arc<LinearAdapter>>,
    tasks: Arc<dyn TaskReader>,
    task_bindings: Arc<dyn TaskBindingDao>,
    ctx: moatless_types::access_control::ScopeContext,
    id: String,
    params: LinearConnectionParams,
    req: LinearCommentCreateRequest,
) -> Result<LinearIssueResponse, AdapterError> {
    let linear = require_linear(linear)?;
    let (connection, api) =
        resolve_linear_api(&linear, &ctx, params.connection_id.as_deref()).await?;

    if let Some(task_id) = req.task_id.as_deref() {
        ensure_task_access_only(tasks.as_ref(), &ctx, task_id).await?;
    }

    // commentCreate requires the issue UUID, not the SP-123 identifier.
    let issue = api.get_issue(&id).await?;
    let issue_uuid = issue
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| SharedAdapterError::NotFound(format!("Linear issue {id} missing id")))?
        .to_string();

    let body = match req.task_id.as_deref() {
        Some(task_id) => {
            match moatless_loops::bindings::footer::task_comment_footer(
                tasks.as_ref(),
                &linear.require_config()?.auth.frontend_url,
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
        attach_moatless_task(
            linear.require_config()?,
            tasks.as_ref(),
            &api,
            &issue,
            task_id,
        )
        .await;

        // A comment refreshes the binding when the issue is already a reply
        // target (the agent is continuing an established thread). A proactive
        // comment on an unbound issue only binds when the caller opts in.
        let already_bound = task_bindings
            .find_by_task_id_and_subject(task_id, LINEAR_PROVIDER, SUBJECT_KIND_ISSUE, &issue_uuid)
            .await?
            .is_some();

        if req.bind == Some(true) || already_bound {
            Some(create_issue_binding(task_bindings.as_ref(), &connection, task_id, &issue).await?)
        } else {
            None
        }
    } else {
        None
    };

    Ok(LinearIssueResponse {
        data: comment,
        binding_id,
    })
}

pub(crate) async fn mutate_label(
    linear: Option<Arc<LinearAdapter>>,
    ctx: moatless_types::access_control::ScopeContext,
    id: String,
    params: LinearConnectionParams,
    req: LinearLabelMutationRequest,
) -> Result<LinearOkResponse, AdapterError> {
    let linear = require_linear(linear)?;
    let (_connection, api) =
        resolve_linear_api(&linear, &ctx, params.connection_id.as_deref()).await?;

    let issue = api.get_issue(&id).await?;
    let issue_uuid = issue
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| SharedAdapterError::NotFound(format!("Linear issue {id} missing id")))?
        .to_string();
    let label_id = resolve_required(
        &api,
        LookupKind::Label,
        &req.label,
        Some(&team_id_of(&issue)?),
    )
    .await?;

    match req.op {
        LinearLabelOp::Add => api.issue_add_label(&issue_uuid, &label_id).await?,
        LinearLabelOp::Remove => api.issue_remove_label(&issue_uuid, &label_id).await?,
    }
    Ok(LinearOkResponse { ok: true })
}

// ---------------------------------------------------------------------------
// Issue: RELATE / LIST
// ---------------------------------------------------------------------------

pub(crate) async fn relate_issue(
    linear: Option<Arc<LinearAdapter>>,
    ctx: moatless_types::access_control::ScopeContext,
    id: String,
    params: LinearConnectionParams,
    req: LinearIssueRelateRequest,
) -> Result<LinearIssueResponse, AdapterError> {
    let linear = require_linear(linear)?;
    let (_connection, api) =
        resolve_linear_api(&linear, &ctx, params.connection_id.as_deref()).await?;

    let issue_uuid = resolve_optional_issue(&api, Some(&id))
        .await?
        .ok_or_else(|| SharedAdapterError::NotFound(format!("Linear issue {id} not found")))?;
    let related_uuid = resolve_optional_issue(&api, Some(&req.related_issue))
        .await?
        .ok_or_else(|| {
            SharedAdapterError::NotFound(format!("Linear issue {} not found", req.related_issue))
        })?;

    let relation = api
        .issue_relate(&issue_uuid, &related_uuid, req.relation.as_linear())
        .await?;
    Ok(LinearIssueResponse {
        data: relation,
        binding_id: None,
    })
}

pub(crate) async fn list_issues(
    linear: Option<Arc<LinearAdapter>>,
    identity: Arc<dyn ProviderIdentityLink>,
    ctx: moatless_types::access_control::ScopeContext,
    params: LinearIssueListParams,
) -> Result<LinearIssueListResponse, AdapterError> {
    let linear = require_linear(linear)?;
    let (_connection, api) =
        resolve_linear_api(&linear, &ctx, params.connection_id.as_deref()).await?;

    let mut filter = serde_json::Map::new();
    if let Some(query) = params.query.as_deref().filter(|q| !q.is_empty()) {
        filter.insert("searchableContent".into(), json!({ "contains": query }));
    }
    // Scopes the project, state and label names below as well as the issues
    // themselves: without a team, `state: "Done"` resolves to whichever team
    // Linear returns first and the list comes back empty.
    let team_id = match params.team.as_deref() {
        Some(team) => Some(resolve_team_id(&api, team).await?),
        None => None,
    };
    if let Some(team_id) = team_id.as_deref() {
        filter.insert("team".into(), json!({ "id": { "eq": team_id } }));
    }
    if let Some(project) = params.project.as_deref() {
        let project_id =
            resolve_required(&api, LookupKind::Project, project, team_id.as_deref()).await?;
        filter.insert("project".into(), json!({ "id": { "eq": project_id } }));
    }
    // `state` and `open_only` both constrain the workflow state, so they share
    // one filter object — two `insert`s under the same key would drop the first.
    let mut state_filter = serde_json::Map::new();
    if let Some(state) = params.state.as_deref() {
        let state_id =
            resolve_required(&api, LookupKind::State, state, team_id.as_deref()).await?;
        state_filter.insert("id".into(), json!({ "eq": state_id }));
    }
    // `open_only` filters on the state's *type*, not its name: every team names
    // its states differently, but the four types are fixed by Linear.
    if params.open_only.unwrap_or(false) {
        state_filter.insert("type".into(), json!({ "nin": ["completed", "canceled"] }));
    }
    if !state_filter.is_empty() {
        filter.insert("state".into(), Value::Object(state_filter));
    }
    if let Some(label) = params.label.as_deref() {
        let label_id =
            resolve_required(&api, LookupKind::Label, label, team_id.as_deref()).await?;
        filter.insert("labels".into(), json!({ "id": { "eq": label_id } }));
    }
    if let Some(assignee) = params.assignee.as_deref() {
        let user = resolve_user_required(&api, identity.as_ref(), assignee).await?;
        filter.insert("assignee".into(), json!({ "id": { "eq": user.id } }));
    }
    if let Some(delegate) = params.delegate.as_deref() {
        let user = resolve_user_required(&api, identity.as_ref(), delegate).await?;
        filter.insert("delegate".into(), json!({ "id": { "eq": user.id } }));
    }

    let first = params.limit.unwrap_or(50).clamp(1, 250);
    let data = api.list_issues(Value::Object(filter), first).await?;
    Ok(LinearIssueListResponse { data })
}

// ---------------------------------------------------------------------------
// GraphQL passthrough + lookup
// ---------------------------------------------------------------------------

pub(crate) async fn graphql_passthrough(
    linear: Option<Arc<LinearAdapter>>,
    ctx: moatless_types::access_control::ScopeContext,
    params: LinearConnectionParams,
    req: LinearGraphqlRequest,
) -> Result<Value, AdapterError> {
    let linear = require_linear(linear)?;
    let (_connection, api) =
        resolve_linear_api(&linear, &ctx, params.connection_id.as_deref()).await?;

    let mut body = json!({ "query": req.query });
    if let Some(vars) = req.variables {
        body["variables"] = vars;
    }
    let value = api.graphql(body).await?;
    Ok(value)
}

pub(crate) async fn lookup(
    linear: Option<Arc<LinearAdapter>>,
    ctx: moatless_types::access_control::ScopeContext,
    params: LinearLookupParams,
    conn_params: LinearConnectionParams,
) -> Result<LinearLookupResponse, AdapterError> {
    let linear = require_linear(linear)?;
    let (_connection, api) =
        resolve_linear_api(&linear, &ctx, conn_params.connection_id.as_deref()).await?;

    let kind = LookupKind::parse(&params.kind).ok_or_else(|| {
        SharedAdapterError::BadRequest(format!(
            "Unknown lookup kind `{}` (use state, label, project, user, team)",
            params.kind
        ))
    })?;
    let team_id = match params.team.as_deref() {
        Some(team) => Some(resolve_team_id(&api, team).await?),
        None => None,
    };
    let result = api.lookup(kind, &params.name, team_id.as_deref()).await?;
    Ok(match result {
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
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Persist a `task_bindings` row pointing the given task at the Linear
/// issue. Used for both create and comment auto-binding.
async fn create_issue_binding(
    task_bindings: &dyn TaskBindingDao,
    connection: &AdapterConnection,
    task_id: &str,
    issue: &Value,
) -> Result<String, AdapterError> {
    let issue_uuid = issue
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            SharedAdapterError::AdapterApi("Linear issue missing id for binding".into())
        })?
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

    let binding = upsert_binding_into(
        task_bindings,
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
/// Idempotent on `(issue_id, url)` — safe to call multiple times. Failures
/// are logged and swallowed so they don't block the surrounding mutation
/// (the issue is already created / commented on).
pub(crate) async fn attach_moatless_task(
    config: &Config,
    tasks: &dyn TaskReader,
    api: &LinearAgentApi,
    issue: &Value,
    task_id: &str,
) {
    let Some(issue_uuid) = issue.get("id").and_then(Value::as_str) else {
        warn!(task_id = %task_id, "attach_moatless_task: issue missing id");
        return;
    };
    super::task_links::attach_parent_task_link(config, tasks, api, issue_uuid, task_id).await;
}
