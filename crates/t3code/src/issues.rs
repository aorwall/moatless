//! `issues.*` and `issueTrackers.list` over the Linear team each Workspace
//! declares (`Workspace::linear_team`), read and written through this
//! deployment's Linear app ([`IssueTrackerSource`]), never a credential of
//! the viewer's. A Workspace is a T3 project, and readable is writable.

use std::collections::BTreeMap;

use moatless_types::access_control::ScopeContext;
use moatless_types::issue_tracker::{
    IssueAssigneeCandidate, IssueCandidates, IssueComment, IssueDetail, IssueDraft, IssueEdit,
    IssueEvent, IssueInvolvement, IssueLabel, IssueLinkedPullRequest, IssueListQuery, IssuePage,
    IssueSort, IssueState, IssueStateFilter, IssueSummary, IssueTrackerError, IssueTrackerTeam,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::dao::ConversationDao;
use crate::ports::IssueTrackerSource;
use crate::pull_request_conversation::actor_json;
use crate::pull_request_review::rfc3339;
use crate::workspace_listing::WorkspaceListing;

/// The one tracker this server serves, as `IssueTrackerKind` spells it.
pub(crate) const LINEAR_TRACKER: &str = "linear";
/// The host every Linear issue URL names.
pub(crate) const LINEAR_HOST: &str = "linear.app";
/// Rows per team when a listing names no limit.
const DEFAULT_LIMIT: u32 = 50;

/// What the issue methods read through.
pub(crate) struct IssuePorts<'a> {
    pub(crate) workspaces: &'a dyn WorkspaceListing,
    pub(crate) tracker: &'a dyn IssueTrackerSource,
    pub(crate) conversations: &'a dyn ConversationDao,
}

pub(crate) fn issue_ports(state: &crate::state::UiRpcState) -> IssuePorts<'_> {
    IssuePorts {
        workspaces: state.reads.workspaces(),
        tracker: state.issues.as_ref(),
        conversations: state.reads.conversations(),
    }
}

/// Why an issue method failed.
#[derive(Debug)]
pub(crate) enum IssueFailure {
    /// No readable project reads the named team, or the tracker is not Linear.
    NotBound,
    /// A request field is missing or out of range; the message says which.
    Invalid(String),
    Tracker(IssueTrackerError),
    Store(sqlx::Error),
}

impl From<IssueTrackerError> for IssueFailure {
    fn from(error: IssueTrackerError) -> Self {
        Self::Tracker(error)
    }
}

impl From<sqlx::Error> for IssueFailure {
    fn from(error: sqlx::Error) -> Self {
        Self::Store(error)
    }
}

/// Every request naming an issue or a team: `IssueRef`, `IssueScopeRef` and
/// the inputs that extend them. Each method reads the fields it needs.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct IssueRequest {
    project_id: String,
    tracker: String,
    scope: String,
    #[serde(default)]
    key: String,
    cursor: Option<String>,
    title: Option<String>,
    body: Option<String>,
    labels: Option<Vec<String>>,
    assignees: Option<Vec<String>>,
    state: Option<String>,
}

/// The contract's `ThreadIssueKey`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ThreadIssueKeyInput {
    tracker: String,
    scope: String,
    key: String,
}

/// The contract's `IssueListInput`.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct IssueListInput {
    state: Option<String>,
    involvement: Option<String>,
    project_id: Option<String>,
    host: Option<String>,
    limit: Option<u32>,
    /// Keyed `"<host> <scope>"`.
    cursors: Option<BTreeMap<String, String>>,
    query: Option<String>,
    labels: Option<Vec<String>>,
    assignee: Option<String>,
    sort: Option<String>,
}

/// A readable project and the team it declares.
#[derive(Clone, Debug)]
struct Project {
    id: String,
    title: String,
    team: String,
}

/// Every readable project declaring a Linear team, by name; only `named` when
/// given.
async fn projects(
    ports: &IssuePorts<'_>,
    viewer: &ScopeContext,
    named: Option<&str>,
) -> Result<Vec<Project>, sqlx::Error> {
    Ok(ports
        .workspaces
        .list_readable_including(viewer, &[])
        .await?
        .into_iter()
        .filter(|listed| listed.readable)
        .filter(|listed| named.is_none_or(|id| listed.workspace.id == id))
        .filter_map(|listed| {
            let team = listed
                .workspace
                .linear_team
                .as_deref()
                .map(str::trim)
                .filter(|team| !team.is_empty())?
                .to_ascii_uppercase();
            let title = listed.workspace.name.trim();
            Some(Project {
                title: if title.is_empty() {
                    team.clone()
                } else {
                    title.to_string()
                },
                id: listed.workspace.id,
                team,
            })
        })
        .collect())
}

/// The project a request names, when the viewer may read it and it declares a
/// team, with `team` set to the request's scope.
///
/// A project lends its Linear connection to any team, as T3's
/// `resolveIssueLinkTarget` expects, so an OP issue opens in an SP project;
/// Linear itself answers `NotFound` for a key outside the scope.
async fn project_for(
    ports: &IssuePorts<'_>,
    viewer: &ScopeContext,
    request: &IssueRequest,
) -> Result<Project, IssueFailure> {
    if request.tracker != LINEAR_TRACKER {
        return Err(IssueFailure::NotBound);
    }
    let project = projects(ports, viewer, Some(request.project_id.trim()))
        .await?
        .into_iter()
        .next()
        .ok_or(IssueFailure::NotBound)?;
    lent(project, &request.scope)
}

/// A readable project lending its connection to team `scope`, for a read that
/// names no project: its own team's first, else any.
async fn any_project_for(
    ports: &IssuePorts<'_>,
    viewer: &ScopeContext,
    tracker: &str,
    scope: &str,
) -> Result<Project, IssueFailure> {
    if tracker != LINEAR_TRACKER {
        return Err(IssueFailure::NotBound);
    }
    let mut readable = projects(ports, viewer, None).await?;
    if readable.is_empty() {
        return Err(IssueFailure::NotBound);
    }
    let own = readable
        .iter()
        .position(|project| project.team.eq_ignore_ascii_case(scope.trim()))
        .unwrap_or(0);
    lent(readable.swap_remove(own), scope)
}

fn lent(project: Project, scope: &str) -> Result<Project, IssueFailure> {
    let team = scope.trim().to_ascii_uppercase();
    if team.is_empty() {
        return Err(IssueFailure::Invalid(
            "An issue reference needs a scope.".to_string(),
        ));
    }
    Ok(Project { team, ..project })
}

fn key_of(request: &IssueRequest) -> Result<&str, IssueFailure> {
    Some(request.key.trim())
        .filter(|key| !key.is_empty())
        .ok_or_else(|| IssueFailure::Invalid("An issue reference needs a key.".to_string()))
}

/// `issues.list`: one slice of each readable project's team, newest first.
pub(crate) async fn list(
    ports: &IssuePorts<'_>,
    viewer: &ScopeContext,
    input: &IssueListInput,
) -> Result<Value, IssueFailure> {
    let host_named = input.host.as_deref().map(str::trim);
    if host_named.is_some_and(|host| host != LINEAR_HOST) {
        return Ok(list_result(&[], None, Vec::new(), Vec::new(), Map::new()));
    }
    // One project per team: the first by name reads it.
    let mut teams: Vec<(Project, Option<String>)> = Vec::new();
    for project in projects(ports, viewer, input.project_id.as_deref()).await? {
        if teams.iter().any(|(read, _)| read.team == project.team) {
            continue;
        }
        let cursor = match input.cursors.as_ref() {
            Some(cursors) => match cursors.get(&scope_cursor_key(&project.team)) {
                Some(cursor) => Some(cursor.clone()),
                None => continue,
            },
            None => None,
        };
        teams.push((project, cursor));
    }
    let query = list_query(input)?;
    let viewer_user = match (query.involvement, teams.first()) {
        (IssueInvolvement::All, _) | (_, None) => None,
        (_, Some((project, _))) => ports.tracker.viewer(viewer, &project.team).await?,
    };
    let viewer_id = viewer_user.as_ref().map(|user| user.id.clone());

    let mut pending = Vec::with_capacity(teams.len());
    for (project, cursor) in &teams {
        let query = IssueListQuery {
            scope: project.team.clone(),
            after: cursor.clone(),
            ..query.clone()
        };
        let viewer_id = viewer_id.clone();
        pending.push(async move {
            let page = ports.tracker.list(viewer, &query, viewer_id).await;
            (project, page)
        });
    }
    let reads = futures::future::join_all(pending).await;

    let mut entries = Vec::new();
    let mut errors = Vec::new();
    let mut next_cursors = Map::new();
    for (project, page) in reads {
        match page {
            Ok(IssuePage { items, next_cursor }) => {
                entries.extend(items.iter().map(|issue| {
                    let mut entry = summary_json(issue);
                    entry["projectId"] = json!(project.id);
                    entry["projectTitle"] = json!(project.title);
                    entry
                }));
                if let Some(cursor) = next_cursor {
                    next_cursors.insert(scope_cursor_key(&project.team), json!(cursor));
                }
            }
            Err(error) => errors.push(json!({
                "projectId": project.id,
                "projectTitle": project.title,
                "message": list_error_message(&error),
            })),
        }
    }
    let field = match query.sort {
        IssueSort::Updated => "updatedAt",
        IssueSort::Created => "createdAt",
    };
    entries.sort_by(|a, b| b[field].as_str().cmp(&a[field].as_str()));
    let projects: Vec<Project> = teams.into_iter().map(|(project, _)| project).collect();
    Ok(list_result(
        &projects,
        viewer_user.map(|user| user.actor.login),
        entries,
        errors,
        next_cursors,
    ))
}

fn scope_cursor_key(team: &str) -> String {
    format!("{LINEAR_HOST} {team}")
}

fn list_query(input: &IssueListInput) -> Result<IssueListQuery, IssueFailure> {
    let invalid = |field: &str, value: &str| {
        IssueFailure::Invalid(format!("`{value}` is not a valid {field}."))
    };
    let state = match input.state.as_deref().unwrap_or("open") {
        "all" => IssueStateFilter::All,
        "open" => IssueStateFilter::Open,
        "closed" => IssueStateFilter::Closed,
        other => return Err(invalid("state", other)),
    };
    let involvement = match input.involvement.as_deref().unwrap_or("all") {
        "all" => IssueInvolvement::All,
        "assigned" => IssueInvolvement::Assigned,
        "authored" => IssueInvolvement::Authored,
        "mentioned" => IssueInvolvement::Mentioned,
        other => return Err(invalid("involvement", other)),
    };
    let sort = match input.sort.as_deref().unwrap_or("updated") {
        "updated" => IssueSort::Updated,
        "created" => IssueSort::Created,
        other => return Err(invalid("sort", other)),
    };
    let text = |value: Option<&str>| {
        value
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    Ok(IssueListQuery {
        scope: String::new(),
        state,
        involvement,
        query: text(input.query.as_deref()),
        labels: input
            .labels
            .iter()
            .flatten()
            .filter_map(|label| text(Some(label)))
            .collect(),
        assignee: text(input.assignee.as_deref()),
        sort,
        limit: input
            .limit
            .map_or(DEFAULT_LIMIT, |limit| limit.clamp(1, 200)),
        after: None,
    })
}

fn list_result(
    projects: &[Project],
    viewer_login: Option<String>,
    entries: Vec<Value>,
    errors: Vec<Value>,
    next_cursors: Map<String, Value>,
) -> Value {
    let trackers = if projects.is_empty() {
        Vec::new()
    } else {
        vec![json!({
            "tracker": LINEAR_TRACKER,
            "host": LINEAR_HOST,
            "projectCount": projects.len(),
            "capabilities": capabilities(),
            "configured": true,
            "detail": Value::Null,
        })]
    };
    let viewers: Map<String, Value> = viewer_login
        .map_or_default(|login| Map::from_iter([(LINEAR_HOST.to_string(), json!(login))]));
    json!({
        "viewers": viewers,
        "trackers": trackers,
        "entries": entries,
        "errors": errors,
        "nextCursors": next_cursors,
        "scopes": projects
            .iter()
            .map(|project| json!({
                "projectId": project.id,
                "tracker": LINEAR_TRACKER,
                "host": LINEAR_HOST,
                "scope": project.team,
            }))
            .collect::<Vec<_>>(),
    })
}

fn list_error_message(error: &IssueTrackerError) -> String {
    match error {
        IssueTrackerError::NotFound => "Linear has no such team.".to_string(),
        IssueTrackerError::Unauthenticated => {
            "No Linear connection reaches this project's team.".to_string()
        }
        IssueTrackerError::Refused(message) => message.clone(),
        IssueTrackerError::Failed(cause) => {
            tracing::warn!(%cause, "Linear issue listing failed");
            "Linear could not be read.".to_string()
        }
    }
}

/// `issues.detail`.
pub(crate) async fn detail(
    ports: &IssuePorts<'_>,
    viewer: &ScopeContext,
    request: &IssueRequest,
) -> Result<Value, IssueFailure> {
    let project = project_for(ports, viewer, request).await?;
    let key = key_of(request)?;
    let detail = ports.tracker.detail(viewer, &project.team, key).await?;
    // Only who the viewer is in Linear; a miss leaves comment edits unoffered.
    let login = match ports.tracker.viewer(viewer, &project.team).await {
        Ok(user) => user.map(|user| user.actor.login),
        Err(error) => {
            tracing::debug!(?error, "Linear viewer could not be read");
            None
        }
    };
    Ok(detail_json(&project, &detail, login))
}

/// `issues.comments`.
pub(crate) async fn comments(
    ports: &IssuePorts<'_>,
    viewer: &ScopeContext,
    request: &IssueRequest,
) -> Result<Value, IssueFailure> {
    let project = project_for(ports, viewer, request).await?;
    let key = key_of(request)?;
    let cursor = request
        .cursor
        .as_deref()
        .map(str::trim)
        .filter(|cursor| !cursor.is_empty())
        .ok_or_else(|| IssueFailure::Invalid("A comments page needs a cursor.".to_string()))?;
    let page = ports
        .tracker
        .comments(viewer, &project.team, key, cursor)
        .await?;
    Ok(json!({
        "comments": page.comments.iter().map(comment_json).collect::<Vec<_>>(),
        "nextCursor": page.next_cursor,
    }))
}

/// `issues.templates`: Linear's templates fill fields a composer here has no
/// place for, so none are offered.
pub(crate) async fn templates(
    ports: &IssuePorts<'_>,
    viewer: &ScopeContext,
    request: &IssueRequest,
) -> Result<Value, IssueFailure> {
    project_for(ports, viewer, request).await?;
    Ok(json!({ "templates": [], "blankIssuesEnabled": true }))
}

/// `issues.candidates`.
pub(crate) async fn candidates(
    ports: &IssuePorts<'_>,
    viewer: &ScopeContext,
    request: &IssueRequest,
) -> Result<Value, IssueFailure> {
    let project = project_for(ports, viewer, request).await?;
    let candidates = ports.tracker.candidates(viewer, &project.team).await?;
    Ok(candidates_json(&candidates))
}

/// `issues.create`.
pub(crate) async fn create(
    ports: &IssuePorts<'_>,
    viewer: &ScopeContext,
    login: &str,
    request: IssueRequest,
) -> Result<Value, IssueFailure> {
    let project = project_for(ports, viewer, &request).await?;
    let title = request
        .title
        .as_deref()
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .ok_or_else(|| IssueFailure::Invalid("An issue needs a title.".to_string()))?
        .to_string();
    let draft = IssueDraft {
        title,
        body: request.body.unwrap_or_default(),
        labels: request.labels.unwrap_or_default(),
        assignees: request.assignees.unwrap_or_default(),
    };
    let created = ports
        .tracker
        .create(viewer, login, &project.team, draft)
        .await?;
    Ok(json!({
        "key": created.key,
        "displayKey": created.key,
        "url": created.url,
    }))
}

/// `issues.comment`.
pub(crate) async fn comment(
    ports: &IssuePorts<'_>,
    viewer: &ScopeContext,
    login: &str,
    request: &IssueRequest,
) -> Result<Value, IssueFailure> {
    let project = project_for(ports, viewer, request).await?;
    let key = key_of(request)?;
    let body = request
        .body
        .as_deref()
        .filter(|body| !body.trim().is_empty())
        .ok_or_else(|| IssueFailure::Invalid("A comment needs a body.".to_string()))?;
    ports
        .tracker
        .comment(viewer, login, &project.team, key, body)
        .await?;
    Ok(Value::Null)
}

/// `issues.update`.
pub(crate) async fn update(
    ports: &IssuePorts<'_>,
    viewer: &ScopeContext,
    request: IssueRequest,
) -> Result<Value, IssueFailure> {
    let project = project_for(ports, viewer, &request).await?;
    let key = key_of(&request)?.to_string();
    let edit = IssueEdit {
        title: request.title,
        body: request.body,
        labels: request.labels,
        assignees: request.assignees,
    };
    if edit == IssueEdit::default() {
        return Err(IssueFailure::Invalid(
            "An update needs a field to change.".to_string(),
        ));
    }
    ports
        .tracker
        .update(viewer, &project.team, &key, edit)
        .await?;
    Ok(Value::Null)
}

/// `issues.setState`.
pub(crate) async fn set_state(
    ports: &IssuePorts<'_>,
    viewer: &ScopeContext,
    request: &IssueRequest,
) -> Result<Value, IssueFailure> {
    let project = project_for(ports, viewer, request).await?;
    let key = key_of(request)?;
    let state = request
        .state
        .as_deref()
        .map(str::trim)
        .filter(|state| !state.is_empty())
        .ok_or_else(|| IssueFailure::Invalid("A state change needs a state.".to_string()))?;
    ports
        .tracker
        .set_state(viewer, &project.team, key, state)
        .await?;
    Ok(Value::Null)
}

/// `issues.linkedThreads`: the readable Tasks bound to the issue.
pub(crate) async fn linked_threads(
    ports: &IssuePorts<'_>,
    viewer: &ScopeContext,
    input: &ThreadIssueKeyInput,
) -> Result<Value, IssueFailure> {
    let project = any_project_for(ports, viewer, &input.tracker, &input.scope).await?;
    let task_ids = ports
        .tracker
        .linked_tasks(viewer, &project.team, input.key.trim())
        .await?;
    let readable = if task_ids.is_empty() {
        Vec::new()
    } else {
        ports
            .conversations
            .readable_by_ids(viewer, &task_ids)
            .await?
    };
    let threads: Vec<Value> = readable
        .iter()
        .filter_map(|conversation| {
            Some(json!({
                "id": conversation.task_id,
                "projectId": conversation.workspace_id.as_deref()?,
                "title": conversation
                    .name
                    .as_deref()
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .unwrap_or("New thread"),
                "archivedAt": conversation.closed_at.map(rfc3339),
                "settled": conversation.settled_at.is_some(),
            }))
        })
        .collect();
    Ok(json!({ "threads": threads }))
}

/// `issueTrackers.list`: the Linear workspace this deployment's app reaches
/// and the team each readable project declares. Read-only: a connection is
/// made by installing the app, and a binding in the Workspace's config.
pub(crate) async fn trackers(
    ports: &IssuePorts<'_>,
    viewer: &ScopeContext,
) -> Result<Value, IssueFailure> {
    let workspaces = ports.tracker.workspaces(viewer).await?;
    let bindings: Map<String, Value> = projects(ports, viewer, None)
        .await?
        .into_iter()
        .map(|project| {
            let binding = json!({ "tracker": LINEAR_TRACKER, "scope": project.team });
            (project.id, binding)
        })
        .collect();
    let first = workspaces.first();
    let mut connection = json!({
        "tracker": LINEAR_TRACKER,
        "displayName": "Linear",
        "host": LINEAR_HOST,
        "scopeNoun": "team",
        "credential": { "label": "Moatless Linear app", "helpUrl": Value::Null },
        "connected": first.is_some(),
        "account": first.map(|workspace| workspace.name.clone()),
        "detail": if first.is_some() {
            Value::Null
        } else {
            json!("Install the Moatless Linear app to read Linear issues.")
        },
        "scopes": workspaces
            .iter()
            .flat_map(|workspace| &workspace.teams)
            .map(scope_json)
            .collect::<Vec<_>>(),
    });
    if let Some(workspace) = first {
        connection["workspace"] = json!(workspace.url_key);
    }
    Ok(json!({ "trackers": [connection], "bindings": bindings }))
}

fn scope_json(team: &IssueTrackerTeam) -> Value {
    json!({ "key": team.key, "name": team.name })
}

fn capabilities() -> Value {
    json!({
        "create": true,
        "edit": true,
        "comment": true,
        "labels": true,
        "assignees": true,
        "reactions": false,
        "templates": false,
        "linkedPullRequests": true,
        "events": true,
        "search": true,
    })
}

fn summary_json(issue: &IssueSummary) -> Value {
    json!({
        "tracker": LINEAR_TRACKER,
        "host": LINEAR_HOST,
        "scope": issue.scope,
        "key": issue.key,
        "displayKey": issue.key,
        "title": issue.title,
        "url": issue.url,
        "author": issue.author.as_ref().map(actor_json),
        "state": state_json(&issue.state),
        "createdAt": issue.created_at,
        "updatedAt": issue.updated_at,
        "closedAt": issue.closed_at,
        "assignees": issue.assignees.iter().map(actor_json).collect::<Vec<_>>(),
        "labels": issue.labels.iter().map(label_json).collect::<Vec<_>>(),
        "commentCount": issue.comment_count,
    })
}

fn detail_json(project: &Project, detail: &IssueDetail, viewer_login: Option<String>) -> Value {
    let mut value = summary_json(&detail.summary);
    let extra = json!({
        "projectId": project.id,
        "projectTitle": project.title,
        "writable": true,
        "capabilities": capabilities(),
        // Linear reports no per-issue permission and refuses with a message.
        "viewerPermissions": {
            "setState": true,
            "comment": true,
            "edit": true,
            "labels": true,
            "assignees": true,
        },
        "states": detail.states.iter().map(state_json).collect::<Vec<_>>(),
        "viewer": viewer_login,
        "body": detail.body,
        "comments": detail.comments.iter().map(comment_json).collect::<Vec<_>>(),
        "commentsCursor": detail.comments_cursor,
        "events": detail.events.iter().map(event_json).collect::<Vec<_>>(),
        "linkedPullRequests": detail
            .linked_pull_requests
            .iter()
            .map(linked_pull_request_json)
            .collect::<Vec<_>>(),
    });
    if let (Value::Object(value), Value::Object(extra)) = (&mut value, extra) {
        value.extend(extra);
    }
    value
}

fn state_json(state: &IssueState) -> Value {
    let mut value = json!({
        "category": if state.closed { "closed" } else { "open" },
        "name": state.name,
    });
    if let Some(reason) = state.close_reason.as_deref() {
        value["closeReason"] = json!(reason);
    }
    value
}

fn label_json(label: &IssueLabel) -> Value {
    json!({ "name": label.name, "color": label.color })
}

fn comment_json(comment: &IssueComment) -> Value {
    json!({
        "id": comment.id,
        "author": comment.author.as_ref().map(actor_json),
        "body": comment.body,
        "createdAt": comment.created_at,
        "url": comment.url,
    })
}

fn event_json(event: &IssueEvent) -> Value {
    json!({
        "id": event.id,
        "kind": event.kind,
        "actor": event.actor.as_ref().map(actor_json),
        "createdAt": event.created_at,
        "detail": event.detail.as_deref().map(str::trim).filter(|detail| !detail.is_empty()),
    })
}

fn linked_pull_request_json(pull_request: &IssueLinkedPullRequest) -> Value {
    json!({
        "repository": pull_request.repository,
        "number": pull_request.number,
        "title": pull_request.title,
        "url": pull_request.url,
        "state": pull_request.state,
        "isDraft": pull_request.is_draft,
        "closesIssue": pull_request.closes_issue,
    })
}

fn candidates_json(candidates: &IssueCandidates) -> Value {
    json!({
        "labels": candidates
            .labels
            .iter()
            .map(|label| json!({
                "name": label.name,
                "color": label.color,
                "description": label.description,
            }))
            .collect::<Vec<_>>(),
        "assignees": candidates.assignees.iter().map(assignee_json).collect::<Vec<_>>(),
        "truncated": candidates.truncated,
    })
}

fn assignee_json(candidate: &IssueAssigneeCandidate) -> Value {
    let mut value = actor_json(&candidate.actor);
    value["id"] = json!(candidate.id);
    value
}

/// The `IssueRpcError` member a failure answers as.
pub(crate) fn failure_json(operation: &str, failure: &IssueFailure) -> Value {
    let unavailable = |reason: &str| {
        json!({
            "_tag": "IssueUnavailableError",
            "reason": reason,
            "tracker": LINEAR_TRACKER,
        })
    };
    let operation_error = |detail: &str, reason: Option<&str>| {
        let mut value = json!({
            "_tag": "IssueOperationError",
            "operation": operation,
            "detail": detail,
        });
        if let Some(reason) = reason {
            value["reason"] = json!(reason);
        }
        value
    };
    match failure {
        IssueFailure::NotBound => unavailable("tracker-unsupported"),
        IssueFailure::Invalid(detail) => operation_error(detail, None),
        IssueFailure::Tracker(IssueTrackerError::Unauthenticated) => unavailable("unauthenticated"),
        IssueFailure::Tracker(IssueTrackerError::NotFound) => {
            operation_error("No such issue in this team.", Some("not-found"))
        }
        IssueFailure::Tracker(IssueTrackerError::Refused(message)) => {
            operation_error(message, None)
        }
        IssueFailure::Tracker(IssueTrackerError::Failed(cause)) => {
            tracing::warn!(%cause, operation, "Linear issue call failed");
            operation_error("Linear could not be reached.", None)
        }
        IssueFailure::Store(error) => {
            tracing::error!(%error, operation, "issue read failed on the store");
            operation_error("The request could not be completed.", None)
        }
    }
}

#[cfg(test)]
#[path = "issues_tests.rs"]
mod tests;
