//! Authorized provider operations and ordered mutation effects.

use std::sync::Arc;

use moatless_api_schemas::linear::{
    LinearCommentCreateRequest, LinearConnectionParams, LinearGraphqlRequest,
    LinearIssueCreateRequest, LinearIssueListParams, LinearIssueListResponse,
    LinearIssueRelateRequest, LinearIssueResponse, LinearIssueUpdateRequest,
    LinearLabelMutationRequest, LinearLabelOp, LinearLookupParams, LinearLookupResponse,
    LinearOkResponse,
};
use serde_json::{Value, json};
use tracing::warn;

use moatless_adapter_model::binding::TASK_OWNED_BINDING_ORIGIN;
use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
use moatless_core::config::Config;
use moatless_loops::adapters::access::ensure_task_access_only;
use moatless_loops::bindings::operations::Bindings;
use moatless_ports::identity::{ProviderIdentityLink, UserDisplay};
use moatless_ports::task::TaskReader;
use moatless_types::adapter::AdapterConnection;

use super::SUBJECT_KIND_ISSUE;
use super::adapter::LinearAdapter;
use super::agent_api::{LinearAgentApi, LookupKind};
use super::auth::{require_linear, resolve_linear_api};
use super::reference_resolution::*;

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
    let data = api.get_issue_with_comments(&id).await?;
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
    task_bindings: Arc<Bindings>,
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
    if let Some(s) = state_id.as_deref() {
        input["stateId"] = json!(s);
    }
    if let Some(p) = project_id {
        input["projectId"] = json!(p);
    }
    if let Some(a) = assignee_id.as_deref() {
        input["assigneeId"] = json!(a);
    }
    if let Some(d) = delegate_id.as_ref() {
        input["delegateId"] = d.clone();
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
    if let Some(requested) = state_id.as_deref() {
        require_applied(&issue, "state", "/state", Some(requested), "")?;
    }
    // Only an assignee the caller named: the default from the caller's own
    // identity is best-effort by design and must not fail the creation.
    if let (Some(_), Some(requested)) = (req.assignee.as_deref(), assignee_id.as_deref()) {
        require_applied(
            &issue,
            "assignee",
            "/assignee",
            Some(requested),
            DROPPED_ASSIGNEE,
        )?;
    }
    if let Some(requested) = delegate_id.as_ref() {
        require_applied(&issue, "delegate", "/delegate", requested.as_str(), "")?;
    }

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

    let description = description_to_write(req.description.as_deref(), req.clear_description)?;

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
    if let Some(d) = description {
        input["description"] = json!(d);
    }
    if let Some(s) = state_id.as_deref() {
        input["stateId"] = json!(s);
    }
    if let Some(p) = project_id {
        input["projectId"] = json!(p);
    }
    if let Some(a) = assignee_id.as_deref() {
        input["assigneeId"] = json!(a);
    }
    if let Some(d) = delegate_id.as_ref() {
        input["delegateId"] = d.clone();
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
    if let Some(requested) = state_id.as_deref() {
        require_applied(&issue, "state", "/state", Some(requested), "")?;
    }
    if let Some(requested) = assignee_id.as_deref() {
        require_applied(
            &issue,
            "assignee",
            "/assignee",
            Some(requested),
            DROPPED_ASSIGNEE,
        )?;
    }
    if let Some(requested) = delegate_id.as_ref() {
        require_applied(&issue, "delegate", "/delegate", requested.as_str(), "")?;
    }
    Ok(LinearIssueResponse {
        data: issue,
        binding_id: None,
    })
}

/// What Linear does with an `assigneeId` it will not honour, for the error a
/// dropped assignee raises.
const DROPPED_ASSIGNEE: &str = " Linear accepts an `assigneeId` for a user who is not an \
                                assignable member of the issue's team and then drops it, \
                                which is what an OAuth app actor such as `moatless` is. \
                                Hand work to an app agent with `delegate` instead.";

/// The description an update writes, or an error rather than an accidental
/// blanking.
///
/// `--description "$(cat some-file)"` expands to an empty string when the file
/// is gone, and Linear reads that as "erase the description". Erasing needs
/// `clear_description`, so it is never something a failed substitution does.
fn description_to_write(
    description: Option<&str>,
    clear: Option<bool>,
) -> Result<Option<String>, AdapterError> {
    let clear = clear.unwrap_or(false);
    match (description, clear) {
        (Some(text), true) if !text.trim().is_empty() => Err(SharedAdapterError::BadRequest(
            "`description` and `clearDescription` ask for different things — pass one".into(),
        )
        .into()),
        (_, true) => Ok(Some(String::new())),
        (Some(text), false) if text.trim().is_empty() => Err(SharedAdapterError::BadRequest(
            "`description` was empty, which would erase the issue's description. \
             Pass the text to write, or `clearDescription` to blank it on purpose."
                .into(),
        )
        .into()),
        (Some(text), false) => Ok(Some(text.to_string())),
        (None, false) => Ok(None),
    }
}

/// Fail when Linear reported success and the field did not land as asked —
/// `requested` is the id to find there, or `None` to find it empty.
///
/// `issueUpdate` answers `success: true` for an id it drops, so the issue it
/// returns is the only proof. `subobject` points at the field in it, `/state`;
/// `hint` names a known cause.
fn require_applied(
    issue: &Value,
    field: &str,
    subobject: &str,
    requested: Option<&str>,
    hint: &str,
) -> Result<(), AdapterError> {
    let applied = issue
        .pointer(&format!("{subobject}/id"))
        .and_then(Value::as_str);
    if applied == requested {
        return Ok(());
    }
    let identifier = issue
        .get("identifier")
        .and_then(Value::as_str)
        .unwrap_or("the issue");
    let now = match applied {
        Some(id) => {
            let name = issue
                .pointer(&format!("{subobject}/name"))
                .or_else(|| issue.pointer(&format!("{subobject}/displayName")))
                .and_then(Value::as_str)
                .unwrap_or("?");
            format!("`{name}` ({id})")
        }
        None => "nothing".to_string(),
    };
    let detail = match requested {
        Some(id) => format!("has {field} {now}, not the requested `{id}`"),
        None => format!("still has {field} {now} after a request to clear it"),
    };
    Err(SharedAdapterError::AdapterApi(format!(
        "Linear reported success but {identifier} {detail}.{hint} Other fields in the same \
         call may have been applied."
    ))
    .into())
}

// ---------------------------------------------------------------------------
// Comments + labels
// ---------------------------------------------------------------------------

pub(crate) async fn create_comment(
    linear: Option<Arc<LinearAdapter>>,
    tasks: Arc<dyn TaskReader>,
    task_bindings: Arc<Bindings>,
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
            .for_task_and_subject(task_id, LINEAR_PROVIDER, SUBJECT_KIND_ISSUE, &issue_uuid)
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
        let state_id = resolve_required(&api, LookupKind::State, state, team_id.as_deref()).await?;
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
        let label_id = resolve_required(&api, LookupKind::Label, label, team_id.as_deref()).await?;
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

/// Persist a `task_bindings` row pointing the given task at the Linear issue.
/// Used for both create and comment auto-binding.
async fn create_issue_binding(
    task_bindings: &Bindings,
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

    let binding = task_bindings
        .bind(
            connection,
            None,
            task_id,
            LINEAR_PROVIDER,
            SUBJECT_KIND_ISSUE,
            &issue_uuid,
            destination_config,
            // The task filed this issue, so its binding is a producer one — that
            // is what makes it hear replies inside threads on its own issue.
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
///
/// Idempotent on `(issue_id, url)` — safe to call multiple times. Failures are
/// logged and swallowed so they don't block the surrounding mutation.
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

#[cfg(test)]
mod tests {
    use super::*;

    const REQUESTED: &str = "96aece86-e8ab-40fb-a33b-fd9fabc7d679";
    const CANCELED: &str = "23123317-0000-0000-0000-000000000000";

    fn issue_in_state(id: &str, name: &str) -> Value {
        json!({ "identifier": "SP-2982", "state": { "id": id, "name": name } })
    }

    #[test]
    fn a_state_that_landed_passes() {
        let issue = issue_in_state(REQUESTED, "In Review");
        require_applied(&issue, "state", "/state", Some(REQUESTED), "").expect("the state landed");
    }

    /// The corruption this check exists for: the issue came back in a state
    /// nobody asked for, and the caller has to hear it rather than read a
    /// success line.
    #[test]
    fn a_state_that_landed_elsewhere_fails_and_names_it() {
        let issue = issue_in_state(CANCELED, "Canceled");
        let err = require_applied(&issue, "state", "/state", Some(REQUESTED), "")
            .expect_err("a different state must not report success");
        let message = err.to_string();
        assert!(message.contains("SP-2982"), "{message}");
        assert!(message.contains("Canceled"), "{message}");
        assert!(message.contains(REQUESTED), "{message}");
    }

    /// Linear drops an id it will not apply and answers `success: true` with
    /// the field still empty.
    #[test]
    fn a_dropped_field_fails() {
        let issue = json!({ "identifier": "SP-2982", "state": null });
        let err = require_applied(&issue, "state", "/state", Some(REQUESTED), "")
            .expect_err("a dropped id must not report success");
        assert!(err.to_string().contains("has state nothing"), "{err}");
    }

    /// `--assignee moatless` named the OAuth app actor, which Linear accepted
    /// and dropped. The error has to say why, since retrying with the UUID
    /// fails the same way.
    #[test]
    fn a_dropped_assignee_names_the_app_actor() {
        let issue = json!({ "identifier": "SP-2982", "assignee": null });
        let requested = Some(REQUESTED);
        let err = require_applied(&issue, "assignee", "/assignee", requested, DROPPED_ASSIGNEE)
            .expect_err("an unassigned issue must not report a successful assignment");
        let message = err.to_string();
        assert!(message.contains("OAuth app actor"), "{message}");
        assert!(message.contains("delegate"), "{message}");
    }

    #[test]
    fn an_assignee_that_landed_passes() {
        let issue = json!({
            "identifier": "SP-2982",
            "assignee": { "id": REQUESTED, "displayName": "Albert" },
        });
        require_applied(
            &issue,
            "assignee",
            "/assignee",
            Some(REQUESTED),
            DROPPED_ASSIGNEE,
        )
        .expect("the assignee landed");
    }

    /// `--delegate null` releases a lane's claim, and a delegate Linear kept
    /// is that claim still held — the one case where the field landing on no
    /// id is the success.
    #[test]
    fn a_delegate_that_linear_kept_fails_the_clear() {
        let issue = json!({
            "identifier": "SP-2982",
            "delegate": { "id": REQUESTED, "displayName": "moatless" },
        });
        let err = require_applied(&issue, "delegate", "/delegate", None, "")
            .expect_err("a retained delegate must not report a successful clear");
        let message = err.to_string();
        assert!(message.contains("still has delegate"), "{message}");
        assert!(message.contains("moatless"), "{message}");
    }

    #[test]
    fn a_delegate_that_was_cleared_passes() {
        let issue = json!({ "identifier": "SP-2982", "delegate": null });
        require_applied(&issue, "delegate", "/delegate", None, "").expect("the delegate cleared");
    }

    #[test]
    fn a_description_is_written_as_given() {
        let written = description_to_write(Some("the writeup"), None).expect("text is writable");
        assert_eq!(written.as_deref(), Some("the writeup"));
    }

    /// The loss this rejection exists for: `--description "$(cat /tmp/gone)"`
    /// expands to an empty string and Linear erases the description.
    #[test]
    fn an_empty_description_is_refused() {
        for empty in ["", "   ", "\n"] {
            let err = description_to_write(Some(empty), None)
                .expect_err("an empty description must not erase one");
            assert!(err.to_string().contains("clearDescription"), "{err}");
        }
    }

    #[test]
    fn clearing_is_how_a_description_is_erased() {
        let written = description_to_write(None, Some(true)).expect("clearing is allowed");
        assert_eq!(written.as_deref(), Some(""));
    }

    #[test]
    fn text_and_clearing_together_is_refused() {
        let err = description_to_write(Some("the writeup"), Some(true))
            .expect_err("the two ask for different things");
        assert!(err.to_string().contains("pass one"), "{err}");
    }

    #[test]
    fn no_description_leaves_the_field_alone() {
        let written = description_to_write(None, None).expect("an absent field is not a change");
        assert!(written.is_none());
    }
}
