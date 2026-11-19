//! Linear as an issue tracker for a person working outside any Task.
//!
//! A scope is a team key (`SP`) and an issue key its identifier (`SP-12`).
//! Every call goes out as the connection's app; a write names the person
//! through `createAsUser`.

use super::SUBJECT_KIND_ISSUE;
use super::adapter::LinearAdapter;
use super::agent_api::LinearAgentApi;
use super::reference_resolution::LINEAR_PROVIDER;
use moatless_adapter_model::binding::{BindReason, BindScope, TASK_OWNED_BINDING_ORIGIN};
use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
use moatless_api_schemas::linear::{
    LinearConnectionParams, LinearIssueCreateRequest, LinearIssueUpdateRequest,
};
use moatless_loops::bindings::operations::Bindings;
use moatless_loops::bindings::own_events::OwnEvents;
use moatless_ports::identity::{ProviderIdentityLink, UserDisplay};
use moatless_ports::task::TaskReader;
use moatless_types::access_control::ScopeContext;
use moatless_types::adapter::{AdapterConnection, AdapterKind};
use moatless_types::issue_tracker::{
    CreatedIssue, IssueActor, IssueAssigneeCandidate, IssueCandidates, IssueComment,
    IssueCommentPage, IssueDetail, IssueDraft, IssueEdit, IssueEvent, IssueInvolvement, IssueLabel,
    IssueLinkedPullRequest, IssueListQuery, IssuePage, IssueSort, IssueState, IssueStateFilter,
    IssueSummary, IssueTrackerError, IssueTrackerTeam, IssueTrackerWorkspace,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

const COMMENTS_PAGE: i64 = 50;
const CLOSED_STATE_TYPES: [&str; 3] = ["completed", "canceled", "duplicate"];
/// The order Linear lists a workflow in; within a type, `position` decides.
const STATE_TYPE_ORDER: [&str; 7] = [
    "triage",
    "backlog",
    "unstarted",
    "started",
    "completed",
    "canceled",
    "duplicate",
];

/// Each fragment a query may spread, by name. [`with_fragments`] appends only
/// the ones a query reaches: Linear refuses a document with an unused one.
const FRAGMENTS: [(&str, &str); 7] = [
    (
        "U",
        "fragment U on User { id name displayName email avatarUrl }",
    ),
    (
        "B",
        "fragment B on ActorBot { name userDisplayName avatarUrl }",
    ),
    ("S", "fragment S on WorkflowState { id name type position }"),
    ("L", "fragment L on IssueLabel { id name color }"),
    (
        "C",
        "fragment C on Comment { id body createdAt url user { ...U } botActor { ...B } }",
    ),
    (
        "I",
        "fragment I on Issue {
  id identifier title url description createdAt updatedAt completedAt canceledAt
  state { ...S } team { key } creator { ...U } botActor { ...B } assignee { ...U }
  labels { nodes { ...L } }
}",
    ),
    ("P", "fragment P on PageInfo { hasNextPage endCursor }"),
];

const LIST_QUERY: &str = "
query($first: Int!, $after: String, $filter: IssueFilter!, $orderBy: PaginationOrderBy!) {
  issues(first: $first, after: $after, filter: $filter, orderBy: $orderBy) {
    nodes { ...I } pageInfo { ...P }
  }
}";

const DETAIL_QUERY: &str = "
query($id: String!, $comments: Int!) {
  issue(id: $id) {
    ...I
    team { states(first: 100) { nodes { ...S } } }
    comments(first: $comments, orderBy: createdAt) { nodes { ...C } pageInfo { ...P } }
    attachments(first: 50) { nodes { url title metadata } }
    history(first: 100, orderBy: createdAt) {
      nodes {
        id createdAt actor { ...U } botActor { ...B }
        fromState { ...S } toState { ...S }
        fromAssignee { ...U } toAssignee { ...U }
        addedLabels { ...L } removedLabels { ...L }
        fromTitle toTitle
      }
    }
  }
}";

const COMMENTS_QUERY: &str = "
query($id: String!, $first: Int!, $after: String) {
  issue(id: $id) {
    team { key }
    comments(first: $first, after: $after, orderBy: createdAt) { nodes { ...C } pageInfo { ...P } }
  }
}";

const USER_QUERY: &str = "query($id: String!) { user(id: $id) { ...U } }";

const TEAM_EXISTS_QUERY: &str = "
query($key: String!) { teams(first: 1, filter: { key: { eq: $key } }) { nodes { id } } }";

const ORGANIZATION_QUERY: &str = "
query { organization { name urlKey } teams(first: 250) { nodes { key name } } }";

const TEAM_QUERY: &str = "
query($key: String!) {
  teams(first: 1, filter: { key: { eq: $key } }) {
    nodes {
      id key
      states(first: 100) { nodes { ...S } }
      labels(first: 250) { nodes { ...L description } pageInfo { ...P } }
      members(first: 250) { nodes { ...U } pageInfo { ...P } }
    }
  }
  issueLabels(first: 250, filter: { team: { null: true } }) {
    nodes { ...L description } pageInfo { ...P }
  }
}";

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LinearUser {
    id: String,
    name: Option<String>,
    display_name: Option<String>,
    email: Option<String>,
    avatar_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LinearBot {
    name: Option<String>,
    user_display_name: Option<String>,
    avatar_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct LinearState {
    id: String,
    name: String,
    #[serde(rename = "type")]
    kind: String,
    position: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
struct LinearLabel {
    id: String,
    name: String,
    color: Option<String>,
    #[serde(default)]
    description: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct Nodes<T> {
    nodes: Vec<T>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageInfo {
    has_next_page: bool,
    end_cursor: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Page<T> {
    nodes: Vec<T>,
    page_info: PageInfo,
}

impl<T> Page<T> {
    fn next_cursor(&self) -> Option<String> {
        self.page_info
            .has_next_page
            .then(|| self.page_info.end_cursor.clone())
            .flatten()
    }
}

#[derive(Debug, Clone, Deserialize)]
struct TeamKey {
    key: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LinearIssue {
    identifier: String,
    title: String,
    url: String,
    description: Option<String>,
    created_at: String,
    updated_at: String,
    completed_at: Option<String>,
    canceled_at: Option<String>,
    state: LinearState,
    team: TeamKey,
    creator: Option<LinearUser>,
    bot_actor: Option<LinearBot>,
    assignee: Option<LinearUser>,
    labels: Nodes<LinearLabel>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LinearComment {
    id: String,
    body: String,
    created_at: String,
    url: Option<String>,
    user: Option<LinearUser>,
    bot_actor: Option<LinearBot>,
}

#[derive(Debug, Clone, Deserialize)]
struct LinearAttachment {
    url: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    metadata: Value,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LinearHistoryEntry {
    id: String,
    created_at: String,
    actor: Option<LinearUser>,
    bot_actor: Option<LinearBot>,
    from_state: Option<LinearState>,
    to_state: Option<LinearState>,
    from_assignee: Option<LinearUser>,
    to_assignee: Option<LinearUser>,
    #[serde(default)]
    added_labels: Option<Vec<LinearLabel>>,
    #[serde(default)]
    removed_labels: Option<Vec<LinearLabel>>,
    from_title: Option<String>,
    to_title: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct DetailTeam {
    states: Nodes<LinearState>,
}

/// What the detail query reads beside the issue's own fields. Read from the
/// same object as [`LinearIssue`] in a second pass, rather than flattened:
/// both claim `team`.
#[derive(Debug, Clone, Deserialize)]
struct DetailExtras {
    team: DetailTeam,
    comments: Page<LinearComment>,
    attachments: Nodes<LinearAttachment>,
    history: Nodes<LinearHistoryEntry>,
}

struct LinearIssueDetail {
    issue: LinearIssue,
    extras: DetailExtras,
}

#[derive(Debug, Clone, Deserialize)]
struct LinearTeam {
    states: Nodes<LinearState>,
    labels: Page<LinearLabel>,
    members: Page<LinearUser>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TeamData {
    teams: Nodes<LinearTeam>,
    issue_labels: Page<LinearLabel>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Organization {
    name: String,
    url_key: String,
}

#[derive(Debug, Clone, Deserialize)]
struct NamedTeam {
    key: String,
    name: String,
}

#[derive(Debug, Clone, Deserialize)]
struct OrganizationData {
    organization: Organization,
    teams: Nodes<NamedTeam>,
}

/// A team with the labels it offers, its own and the workspace's.
struct ResolvedTeam {
    team: LinearTeam,
    workspace_labels: Page<LinearLabel>,
}

/// The ports the write paths shared with `/api/v1/linear/*` take.
pub struct LinearIssueTrackerPorts {
    pub tasks: Arc<dyn TaskReader>,
    pub users: Arc<dyn UserDisplay>,
    pub identity: Arc<dyn ProviderIdentityLink>,
    pub task_bindings: Arc<Bindings>,
    pub own_events: Arc<OwnEvents>,
}

/// Linear issues read and written for a person, through the Linear connection
/// that reaches the team they name.
pub struct LinearIssueTracker {
    linear: Arc<LinearAdapter>,
    ports: LinearIssueTrackerPorts,
}

impl LinearIssueTracker {
    pub fn new(linear: Arc<LinearAdapter>, ports: LinearIssueTrackerPorts) -> Self {
        Self { linear, ports }
    }

    /// The Linear user the caller is, through their linked identity or their
    /// email, as the workspace that has team `scope` knows them.
    pub async fn viewer(
        &self,
        ctx: &ScopeContext,
        scope: &str,
    ) -> Result<Option<IssueAssigneeCandidate>, IssueTrackerError> {
        let Ok(user_id) = ctx.user_id.parse::<uuid::Uuid>() else {
            return Ok(None);
        };
        let (_, api) = self.api_for_team(ctx, scope).await?;
        let Some(resolved) = super::reference_resolution::linear_user_of(
            &api,
            self.ports.users.as_ref(),
            self.ports.identity.as_ref(),
            &user_id,
        )
        .await
        .map_err(tracker_error)?
        else {
            return Ok(None);
        };
        let user: LinearUser =
            run(&api, USER_QUERY, json!({ "id": resolved.id }), "/data/user").await?;
        Ok(Some(IssueAssigneeCandidate {
            id: user.id.clone(),
            actor: actor(&user),
        }))
    }

    /// One page of `query.scope`'s issues. `viewer_id` is the caller's Linear
    /// user id ([`Self::viewer`]), which an involvement other than `All` needs.
    pub async fn list(
        &self,
        ctx: &ScopeContext,
        query: &IssueListQuery,
        viewer_id: Option<&str>,
    ) -> Result<IssuePage, IssueTrackerError> {
        let (_, api) = self.api_for_team(ctx, &query.scope).await?;
        let filter = list_filter(query, viewer_id)?;
        let page: Page<LinearIssue> = run(
            &api,
            LIST_QUERY,
            json!({
                "first": query.limit,
                "after": query.after,
                "filter": filter,
                "orderBy": match query.sort {
                    IssueSort::Updated => "updatedAt",
                    IssueSort::Created => "createdAt",
                },
            }),
            "/data/issues",
        )
        .await?;
        Ok(IssuePage {
            next_cursor: page.next_cursor(),
            items: page
                .nodes
                .into_iter()
                .map(|issue| summary(issue, 0))
                .collect(),
        })
    }

    /// The issue `key` in `scope`, with its first page of comments.
    pub async fn detail(
        &self,
        ctx: &ScopeContext,
        scope: &str,
        key: &str,
    ) -> Result<IssueDetail, IssueTrackerError> {
        let identifier = parse_identifier(key).ok_or(IssueTrackerError::NotFound)?;
        let (_, api) = self.api_for_team(ctx, scope).await?;
        let raw: Value = run(
            &api,
            DETAIL_QUERY,
            json!({ "id": identifier, "comments": COMMENTS_PAGE }),
            "/data/issue",
        )
        .await?;
        if !in_team(&raw, scope) {
            return Err(IssueTrackerError::NotFound);
        }
        let detail = LinearIssueDetail {
            issue: decode(raw.clone(), "issue")?,
            extras: decode(raw, "issue detail")?,
        };
        Ok(detail_of(detail))
    }

    /// Comments past `cursor`, oldest first.
    pub async fn comments(
        &self,
        ctx: &ScopeContext,
        scope: &str,
        key: &str,
        cursor: &str,
    ) -> Result<IssueCommentPage, IssueTrackerError> {
        let identifier = parse_identifier(key).ok_or(IssueTrackerError::NotFound)?;
        let (_, api) = self.api_for_team(ctx, scope).await?;
        let raw: Value = run(
            &api,
            COMMENTS_QUERY,
            json!({ "id": identifier, "first": COMMENTS_PAGE, "after": cursor }),
            "/data/issue",
        )
        .await?;
        if !in_team(&raw, scope) {
            return Err(IssueTrackerError::NotFound);
        }
        let page: Page<LinearComment> = decode(raw["comments"].clone(), "comments")?;
        Ok(IssueCommentPage {
            next_cursor: page.next_cursor(),
            comments: page.nodes.into_iter().map(comment).collect(),
        })
    }

    /// The labels and members `scope` offers.
    pub async fn candidates(
        &self,
        ctx: &ScopeContext,
        scope: &str,
    ) -> Result<IssueCandidates, IssueTrackerError> {
        let (_, api) = self.api_for_team(ctx, scope).await?;
        let resolved = team(&api, scope).await?;
        Ok(candidates(resolved))
    }

    /// File a new issue in `scope`, attributed to the caller.
    pub async fn create(
        &self,
        ctx: &ScopeContext,
        login: &str,
        scope: &str,
        draft: IssueDraft,
    ) -> Result<CreatedIssue, IssueTrackerError> {
        if draft.assignees.len() > 1 {
            return Err(one_assignee());
        }
        let (connection, _) = self.api_for_team(ctx, scope).await?;
        let created = super::issue_commands::create_issue(
            Some(self.linear.clone()),
            self.ports.tasks.clone(),
            self.ports.users.clone(),
            self.ports.identity.clone(),
            self.ports.task_bindings.clone(),
            ctx.clone(),
            login.to_string(),
            params(&connection),
            LinearIssueCreateRequest {
                title: draft.title,
                description: Some(draft.body).filter(|body| !body.trim().is_empty()),
                team: Some(team_key(scope)),
                labels: draft.labels,
                assignee: draft.assignees.into_iter().next(),
                state: None,
                project: None,
                delegate: None,
                reporter: None,
                priority: None,
                estimate: None,
                due_date: None,
                parent: None,
                cycle: None,
                template_id: None,
                task_id: None,
                bind: None,
            },
        )
        .await
        .map_err(tracker_error)?;
        let field = |name: &str| {
            created
                .data
                .get(name)
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| IssueTrackerError::Failed(format!("issueCreate returned no {name}")))
        };
        Ok(CreatedIssue {
            key: field("identifier")?,
            url: field("url")?,
        })
    }

    /// Comment on `key`, shown in Linear under the caller's Moatless name.
    pub async fn comment(
        &self,
        ctx: &ScopeContext,
        login: &str,
        scope: &str,
        key: &str,
        body: &str,
    ) -> Result<(), IssueTrackerError> {
        let (_, api) = self.api_for_team(ctx, scope).await?;
        let issue_id = team_issue_id(&api, scope, key).await?;
        let name = super::reference_resolution::creator_name(
            self.ports.users.as_ref(),
            &ctx.user_id,
            login,
        )
        .await;
        api.create_comment_as(&issue_id, body, &name)
            .await
            .map(|_| ())
            .map_err(tracker_error)
    }

    /// Rewrite the fields `edit` names on `key`.
    pub async fn update(
        &self,
        ctx: &ScopeContext,
        scope: &str,
        key: &str,
        edit: IssueEdit,
    ) -> Result<(), IssueTrackerError> {
        let (_, api) = self.api_for_team(ctx, scope).await?;
        let issue_id = team_issue_id(&api, scope, key).await?;
        let mut input = serde_json::Map::new();
        if let Some(title) = edit.title {
            input.insert("title".into(), json!(title));
        }
        if let Some(body) = edit.body {
            input.insert("description".into(), json!(body));
        }
        if edit.labels.is_some() || edit.assignees.is_some() {
            let resolved = team(&api, scope).await?;
            if let Some(labels) = edit.labels.as_deref() {
                input.insert("labelIds".into(), json!(label_ids(&resolved, labels)?));
            }
            if let Some(assignees) = edit.assignees.as_deref() {
                input.insert(
                    "assigneeId".into(),
                    json!(assignee_id(&resolved, assignees)?),
                );
            }
        }
        if input.is_empty() {
            return Err(IssueTrackerError::Refused(
                "Nothing to change was given.".to_string(),
            ));
        }
        api.update_issue(&issue_id, Value::Object(input))
            .await
            .map(|_| ())
            .map_err(tracker_error)
    }

    /// Move `key` to the team workflow state named `state`.
    pub async fn set_state(
        &self,
        ctx: &ScopeContext,
        scope: &str,
        key: &str,
        state: &str,
    ) -> Result<(), IssueTrackerError> {
        let (connection, api) = self.api_for_team(ctx, scope).await?;
        team_issue_id(&api, scope, key).await?;
        let resolved = team(&api, scope).await?;
        let wanted = resolved
            .team
            .states
            .nodes
            .iter()
            .find(|candidate| candidate.name == state)
            .ok_or_else(|| {
                IssueTrackerError::Refused(format!(
                    "This team's workflow has no \"{state}\" state."
                ))
            })?;
        super::issue_commands::update_issue(
            Some(self.linear.clone()),
            self.ports.identity.clone(),
            self.ports.tasks.clone(),
            self.ports.own_events.clone(),
            ctx.clone(),
            key.to_string(),
            params(&connection),
            LinearIssueUpdateRequest {
                state: Some(wanted.id.clone()),
                ..Default::default()
            },
        )
        .await
        .map(|_| ())
        .map_err(tracker_error)
    }

    /// Join `task_id` to `key` as a person linked them, so the Task hears
    /// replies to its own comments there and nothing else
    /// ([`BindReason::Contributor`]). A Task already bound to the issue keeps
    /// its Binding as it is, switched back on if it was parked.
    pub async fn link(
        &self,
        ctx: &ScopeContext,
        scope: &str,
        key: &str,
        task_id: &str,
    ) -> Result<(), IssueTrackerError> {
        let identifier = parse_identifier(key).ok_or(IssueTrackerError::NotFound)?;
        let (connection, api) = self.api_for_team(ctx, scope).await?;
        let issue = api.get_issue(&identifier).await.map_err(tracker_error)?;
        if !in_team(&issue, scope) {
            return Err(IssueTrackerError::NotFound);
        }
        let issue_id = issue
            .get("id")
            .and_then(Value::as_str)
            .ok_or(IssueTrackerError::NotFound)?;
        let bindings = &self.ports.task_bindings;
        let existing = bindings
            .for_task_and_subject(task_id, LINEAR_PROVIDER, SUBJECT_KIND_ISSUE, issue_id)
            .await
            .map_err(store_error)?;
        if existing.is_some() {
            bindings
                .activate(task_id, LINEAR_PROVIDER, SUBJECT_KIND_ISSUE, issue_id)
                .await
                .map_err(store_error)?;
            return Ok(());
        }
        let field = |name: &str| issue.get(name).and_then(Value::as_str).unwrap_or_default();
        bindings
            .bind(
                &connection,
                None,
                task_id,
                LINEAR_PROVIDER,
                SUBJECT_KIND_ISSUE,
                issue_id,
                json!({
                    "issue_id": issue_id,
                    "issue_identifier": field("identifier"),
                    "url": field("url"),
                    "issue_title": field("title"),
                }),
                // Not started by an event on the issue, so closing the issue
                // leaves the Task open (`close.rs`).
                TASK_OWNED_BINDING_ORIGIN,
                None,
                &BindScope::because(BindReason::Contributor),
            )
            .await
            .map(|_| ())
            .map_err(tracker_error)
    }

    /// Release `task_id` from the issue its Binding names as `key`, deleting
    /// the Binding whatever made it. Idempotent.
    pub async fn unlink(&self, task_id: &str, key: &str) -> Result<(), IssueTrackerError> {
        let identifier = parse_identifier(key).ok_or(IssueTrackerError::NotFound)?;
        let bindings = &self.ports.task_bindings;
        let bound = bindings.for_task(task_id).await.map_err(store_error)?;
        for binding in bound.iter().filter(|binding| {
            binding.subject_kind == SUBJECT_KIND_ISSUE
                && super::status::binding_label_and_url(binding).0 == identifier
        }) {
            bindings
                .unbind(
                    task_id,
                    LINEAR_PROVIDER,
                    SUBJECT_KIND_ISSUE,
                    &binding.subject_key,
                )
                .await
                .map_err(store_error)?;
        }
        Ok(())
    }

    /// The Tasks with an active Binding on `key`, oldest first. Unfiltered by
    /// access: the caller narrows them to the ones the person may read.
    pub async fn linked_tasks(
        &self,
        ctx: &ScopeContext,
        scope: &str,
        key: &str,
    ) -> Result<Vec<String>, IssueTrackerError> {
        let (_, api) = self.api_for_team(ctx, scope).await?;
        let issue_id = team_issue_id(&api, scope, key).await?;
        let bound = self
            .ports
            .task_bindings
            .for_subject(LINEAR_PROVIDER, SUBJECT_KIND_ISSUE, &issue_id)
            .await
            .map_err(store_error)?;
        let mut tasks: Vec<String> = Vec::new();
        for binding in bound.into_iter().filter(|binding| binding.active) {
            if !tasks.contains(&binding.task_id) {
                tasks.push(binding.task_id);
            }
        }
        Ok(tasks)
    }

    /// Every Linear workspace a connection the caller can see reaches, with
    /// its teams. One that cannot be read is logged and left out.
    pub async fn workspaces(
        &self,
        ctx: &ScopeContext,
    ) -> Result<Vec<IssueTrackerWorkspace>, IssueTrackerError> {
        let mut workspaces = Vec::new();
        for connection in self.connections(ctx).await? {
            let read = async {
                let api = self
                    .linear
                    .agent_api_for_connection(&connection)
                    .await
                    .map_err(tracker_error)?;
                run::<OrganizationData>(&api, ORGANIZATION_QUERY, json!({}), "/data").await
            };
            match read.await {
                Ok(data) => workspaces.push(IssueTrackerWorkspace {
                    url_key: data.organization.url_key,
                    name: data.organization.name,
                    teams: data
                        .teams
                        .nodes
                        .into_iter()
                        .map(|team| IssueTrackerTeam {
                            key: team.key,
                            name: team.name,
                        })
                        .collect(),
                }),
                Err(error) => tracing::warn!(
                    ?error,
                    connection_id = %connection.id,
                    "Linear workspace could not be read"
                ),
            }
        }
        Ok(workspaces)
    }

    async fn connections(
        &self,
        ctx: &ScopeContext,
    ) -> Result<Vec<AdapterConnection>, IssueTrackerError> {
        let reader = self
            .linear
            .require_connection_reader()
            .map_err(|_| IssueTrackerError::Unauthenticated)?;
        reader
            .list(ctx, Some(AdapterKind::Linear))
            .await
            .map_err(store_error)
    }

    /// The Linear connection that reaches `scope`, and a client for it.
    ///
    /// The one the caller can see, or with several, the first whose
    /// workspace has a team keyed `scope`: T3 names a team, never a
    /// connection.
    async fn api_for_team(
        &self,
        ctx: &ScopeContext,
        scope: &str,
    ) -> Result<(AdapterConnection, LinearAgentApi), IssueTrackerError> {
        let connections = self.connections(ctx).await?;
        let single = connections.len() == 1;
        for connection in connections {
            let api = self
                .linear
                .agent_api_for_connection(&connection)
                .await
                .map_err(tracker_error)?;
            if single || has_team(&api, scope).await {
                return Ok((connection, api));
            }
        }
        Err(IssueTrackerError::Unauthenticated)
    }
}

/// The connection a reused `/api/v1/linear/*` operation must run against.
fn params(connection: &AdapterConnection) -> LinearConnectionParams {
    LinearConnectionParams {
        connection_id: Some(connection.id.clone()),
    }
}

async fn run<T: serde::de::DeserializeOwned>(
    api: &LinearAgentApi,
    query: &str,
    variables: Value,
    pointer: &str,
) -> Result<T, IssueTrackerError> {
    let value = api
        .graphql(json!({ "query": with_fragments(query), "variables": variables }))
        .await
        .map_err(tracker_error)?;
    match value.pointer(pointer) {
        None | Some(Value::Null) => Err(IssueTrackerError::NotFound),
        Some(found) => decode(found.clone(), pointer),
    }
}

/// `query` followed by every fragment it spreads, directly or through another.
fn with_fragments(query: &str) -> String {
    let mut document = query.to_string();
    let mut included = Vec::new();
    loop {
        let spread = FRAGMENTS.iter().find(|(name, _)| {
            !included.contains(name)
                && document
                    .match_indices("...")
                    .any(|(at, _)| document[at + 3..].starts_with(&format!("{name} ")))
        });
        let Some((name, definition)) = spread else {
            return document;
        };
        included.push(name);
        document.push('\n');
        document.push_str(definition);
    }
}

fn decode<T: serde::de::DeserializeOwned>(
    value: Value,
    what: &str,
) -> Result<T, IssueTrackerError> {
    serde_json::from_value(value)
        .map_err(|error| IssueTrackerError::Failed(format!("Linear {what}: {error}")))
}

/// Whether an issue object read from Linear sits in team `scope`: an
/// identifier from another team, or one that moved away, still resolves.
fn in_team(issue: &Value, scope: &str) -> bool {
    issue.pointer("/team/key").and_then(Value::as_str) == Some(team_key(scope).as_str())
}

async fn team(api: &LinearAgentApi, scope: &str) -> Result<ResolvedTeam, IssueTrackerError> {
    let data: TeamData = run(api, TEAM_QUERY, json!({ "key": team_key(scope) }), "/data").await?;
    let team = data.teams.nodes.into_iter().next().ok_or_else(|| {
        IssueTrackerError::Refused(format!("No Linear team has the key \"{scope}\"."))
    })?;
    Ok(ResolvedTeam {
        team,
        workspace_labels: data.issue_labels,
    })
}

async fn has_team(api: &LinearAgentApi, scope: &str) -> bool {
    let teams: Result<Nodes<Value>, _> = run(
        api,
        TEAM_EXISTS_QUERY,
        json!({ "key": team_key(scope) }),
        "/data/teams",
    )
    .await;
    teams.is_ok_and(|teams| !teams.nodes.is_empty())
}

/// The UUID of `key`, not found unless it sits in `scope` ([`in_team`]).
async fn team_issue_id(
    api: &LinearAgentApi,
    scope: &str,
    key: &str,
) -> Result<String, IssueTrackerError> {
    let identifier = parse_identifier(key).ok_or(IssueTrackerError::NotFound)?;
    let issue = api.get_issue(&identifier).await.map_err(tracker_error)?;
    if !in_team(&issue, scope) {
        return Err(IssueTrackerError::NotFound);
    }
    issue
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or(IssueTrackerError::NotFound)
}

fn team_key(scope: &str) -> String {
    scope.trim().to_uppercase()
}

/// `SP-12`, or `None` for anything that is not a Linear identifier.
pub fn parse_identifier(key: &str) -> Option<String> {
    let key = key.trim();
    let (team, number) = key.split_once('-')?;
    let team_ok = team.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && team.chars().all(|c| c.is_ascii_alphanumeric());
    let number_ok = !number.is_empty()
        && !number.starts_with('0')
        && number.chars().all(|c| c.is_ascii_digit());
    (team_ok && number_ok).then(|| key.to_uppercase())
}

fn store_error(error: impl std::fmt::Display) -> IssueTrackerError {
    IssueTrackerError::Failed(error.to_string())
}

fn one_assignee() -> IssueTrackerError {
    IssueTrackerError::Refused("A Linear issue has one assignee at most.".to_string())
}

fn tracker_error(error: AdapterError) -> IssueTrackerError {
    match error {
        AdapterError::Shared(SharedAdapterError::NotFound(_)) => IssueTrackerError::NotFound,
        AdapterError::Shared(SharedAdapterError::AdapterNotConfigured(_)) => {
            IssueTrackerError::Unauthenticated
        }
        AdapterError::Shared(SharedAdapterError::BadRequest(message)) => {
            IssueTrackerError::Refused(message)
        }
        other => IssueTrackerError::Failed(other.to_string()),
    }
}

fn list_filter(
    query: &IssueListQuery,
    viewer_id: Option<&str>,
) -> Result<Value, IssueTrackerError> {
    let mut filter = serde_json::Map::new();
    filter.insert(
        "team".into(),
        json!({ "key": { "eq": team_key(&query.scope) } }),
    );
    let involved = match query.involvement {
        IssueInvolvement::All => None,
        involvement => {
            let viewer_id = viewer_id.ok_or_else(|| {
                IssueTrackerError::Refused(
                    "Your Moatless account is not linked to a Linear user.".to_string(),
                )
            })?;
            let who = json!({ "id": { "eq": viewer_id } });
            Some(match involvement {
                IssueInvolvement::Assigned => ("assignee", who),
                IssueInvolvement::Authored => ("creator", who),
                _ => ("subscribers", json!({ "some": who })),
            })
        }
    };
    if let Some((field, who)) = involved {
        filter.insert(field.into(), who);
    }
    match query.state {
        IssueStateFilter::All => {}
        IssueStateFilter::Open => {
            filter.insert(
                "state".into(),
                json!({ "type": { "nin": CLOSED_STATE_TYPES } }),
            );
        }
        IssueStateFilter::Closed => {
            filter.insert(
                "state".into(),
                json!({ "type": { "in": CLOSED_STATE_TYPES } }),
            );
        }
    }
    // Each its own clause, so an assignee narrows "assigned to me" rather
    // than replacing it.
    let mut clauses: Vec<Value> = query
        .labels
        .iter()
        .map(|name| json!({ "labels": { "some": { "name": { "eq": name } } } }))
        .collect();
    if let Some(assignee) = query.assignee.as_deref() {
        clauses.push(json!({ "assignee": { "displayName": { "eq": assignee } } }));
    }
    if !clauses.is_empty() {
        filter.insert("and".into(), Value::Array(clauses));
    }
    if let Some(text) = query.query.as_deref() {
        let mut any = vec![
            json!({ "title": { "containsIgnoreCase": text } }),
            json!({ "description": { "containsIgnoreCase": text } }),
        ];
        if let Some(number) = issue_number(text) {
            any.push(json!({ "number": { "eq": number } }));
        }
        filter.insert("or".into(), Value::Array(any));
    }
    Ok(Value::Object(filter))
}

/// The number `SP-12`, `#12` or `12` asks for.
fn issue_number(text: &str) -> Option<u64> {
    let text = text.trim();
    let digits = text
        .strip_prefix('#')
        .or_else(|| text.split_once('-').map(|(_, number)| number))
        .unwrap_or(text);
    digits.parse().ok()
}

fn state(state: &LinearState) -> IssueState {
    let closed = CLOSED_STATE_TYPES.contains(&state.kind.as_str());
    IssueState {
        name: state.name.clone(),
        closed,
        close_reason: closed.then(|| state.kind.clone()),
    }
}

/// The login is Linear's handle (`displayName`, unique in a workspace): what
/// people see and what filters name.
fn actor(user: &LinearUser) -> IssueActor {
    IssueActor {
        login: user
            .display_name
            .clone()
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| user.id.clone()),
        name: user.name.clone(),
        avatar_url: user.avatar_url.clone(),
        is_bot: false,
    }
}

/// The person, else the integration that acted, which Linear reports beside
/// an empty user.
fn actor_of(user: Option<&LinearUser>, bot: Option<&LinearBot>) -> Option<IssueActor> {
    if let Some(user) = user {
        return Some(actor(user));
    }
    let bot = bot?;
    let name = bot.user_display_name.clone().or_else(|| bot.name.clone())?;
    Some(IssueActor {
        login: bot.name.clone().unwrap_or_else(|| name.clone()),
        name: Some(name),
        avatar_url: bot.avatar_url.clone(),
        is_bot: true,
    })
}

fn user_name(user: &LinearUser) -> String {
    user.display_name
        .clone()
        .or_else(|| user.name.clone())
        .or_else(|| user.email.clone())
        .unwrap_or_else(|| user.id.clone())
}

fn label(label: &LinearLabel) -> IssueLabel {
    IssueLabel {
        name: label.name.clone(),
        color: label.color.clone(),
        description: label.description.clone(),
    }
}

fn summary(issue: LinearIssue, comment_count: u64) -> IssueSummary {
    IssueSummary {
        author: actor_of(issue.creator.as_ref(), issue.bot_actor.as_ref()),
        state: state(&issue.state),
        closed_at: issue.completed_at.or(issue.canceled_at),
        assignees: issue.assignee.as_ref().map(actor).into_iter().collect(),
        labels: issue.labels.nodes.iter().map(label).collect(),
        scope: issue.team.key,
        key: issue.identifier,
        title: issue.title,
        url: issue.url,
        created_at: issue.created_at,
        updated_at: issue.updated_at,
        comment_count,
    }
}

fn comment(comment: LinearComment) -> IssueComment {
    IssueComment {
        author: actor_of(comment.user.as_ref(), comment.bot_actor.as_ref()),
        id: comment.id,
        body: comment.body,
        created_at: comment.created_at,
        url: comment.url,
    }
}

fn in_workflow_order(mut states: Vec<LinearState>) -> Vec<LinearState> {
    let rank = |kind: &str| {
        STATE_TYPE_ORDER
            .iter()
            .position(|known| *known == kind)
            .unwrap_or(STATE_TYPE_ORDER.len())
    };
    states.sort_by(|a, b| {
        rank(&a.kind).cmp(&rank(&b.kind)).then(
            a.position
                .unwrap_or(0.0)
                .total_cmp(&b.position.unwrap_or(0.0)),
        )
    });
    states
}

fn events(history: &[LinearHistoryEntry]) -> Vec<IssueEvent> {
    let mut events = Vec::new();
    for entry in history {
        let actor = actor_of(entry.actor.as_ref(), entry.bot_actor.as_ref());
        let mut add = |kind: &'static str, detail: Option<String>, suffix: String| {
            events.push(IssueEvent {
                id: format!("{}:{suffix}", entry.id),
                kind,
                actor: actor.clone(),
                created_at: entry.created_at.clone(),
                detail,
            });
        };
        let moved_to = entry
            .to_state
            .as_ref()
            .filter(|to| entry.from_state.as_ref().map(|from| &from.id) != Some(&to.id));
        if let Some(to) = moved_to {
            let was_closed = entry
                .from_state
                .as_ref()
                .is_some_and(|from| CLOSED_STATE_TYPES.contains(&from.kind.as_str()));
            let is_closed = CLOSED_STATE_TYPES.contains(&to.kind.as_str());
            let kind = match (was_closed, is_closed) {
                (false, true) => "closed",
                (true, false) => "reopened",
                _ => "state-changed",
            };
            add(kind, Some(to.name.clone()), "state".to_string());
        }
        for added in entry.added_labels.iter().flatten() {
            add(
                "labeled",
                Some(added.name.clone()),
                format!("label+{}", added.id),
            );
        }
        for removed in entry.removed_labels.iter().flatten() {
            add(
                "unlabeled",
                Some(removed.name.clone()),
                format!("label-{}", removed.id),
            );
        }
        let from_id = entry.from_assignee.as_ref().map(|user| &user.id);
        let to_id = entry.to_assignee.as_ref().map(|user| &user.id);
        if let Some(from) = entry
            .from_assignee
            .as_ref()
            .filter(|from| Some(&from.id) != to_id)
        {
            add(
                "unassigned",
                Some(user_name(from)),
                "unassigned".to_string(),
            );
        }
        if let Some(to) = entry
            .to_assignee
            .as_ref()
            .filter(|to| Some(&to.id) != from_id)
        {
            add("assigned", Some(user_name(to)), "assigned".to_string());
        }
        if let Some(title) = entry
            .to_title
            .as_ref()
            .filter(|title| entry.from_title.as_ref() != Some(*title))
        {
            add("renamed", Some(title.clone()), "title".to_string());
        }
    }
    events
}

/// Linear's GitHub integration records a pull request as an attachment.
fn linked_pull_requests(attachments: &[LinearAttachment]) -> Vec<IssueLinkedPullRequest> {
    let mut seen = std::collections::HashSet::new();
    attachments
        .iter()
        .filter_map(|attachment| {
            let rest = attachment.url.strip_prefix("https://github.com/")?;
            let mut parts = rest.split('/');
            let (owner, repo, pull, number) =
                (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
            let number: u64 = number
                .split(['?', '#'])
                .next()?
                .parse()
                .ok()
                .filter(|n| *n > 0)?;
            if pull != "pull" || !seen.insert(attachment.url.clone()) {
                return None;
            }
            let status = attachment.metadata.get("status").and_then(Value::as_str);
            let title = attachment.title.trim();
            Some(IssueLinkedPullRequest {
                repository: format!("{owner}/{repo}"),
                number,
                title: if title.is_empty() {
                    attachment.url.clone()
                } else {
                    title.to_string()
                },
                url: attachment.url.clone(),
                state: match status {
                    Some("merged") => "merged",
                    Some("closed") => "closed",
                    _ => "open",
                },
                is_draft: status == Some("draft")
                    || attachment.metadata.get("draft").and_then(Value::as_bool) == Some(true),
                // Linear's attachment does not say whether merging closes the
                // issue; that is a team automation setting.
                closes_issue: false,
            })
        })
        .collect()
}

fn detail_of(detail: LinearIssueDetail) -> IssueDetail {
    let LinearIssueDetail {
        issue,
        extras:
            DetailExtras {
                team: workflow,
                comments,
                attachments,
                history,
            },
    } = detail;
    let body = issue.description.clone().unwrap_or_default();
    let comments_cursor = comments.next_cursor();
    let comment_count = comments.nodes.len() as u64;
    IssueDetail {
        summary: summary(issue, comment_count),
        body,
        states: in_workflow_order(workflow.states.nodes)
            .iter()
            .map(state)
            .collect(),
        comments: comments.nodes.into_iter().map(comment).collect(),
        comments_cursor,
        events: events(&history.nodes),
        linked_pull_requests: linked_pull_requests(&attachments.nodes),
    }
}

fn candidates(resolved: ResolvedTeam) -> IssueCandidates {
    let ResolvedTeam {
        team,
        workspace_labels,
    } = resolved;
    let truncated = team.labels.page_info.has_next_page
        || team.members.page_info.has_next_page
        || workspace_labels.page_info.has_next_page;
    IssueCandidates {
        labels: team
            .labels
            .nodes
            .iter()
            .chain(workspace_labels.nodes.iter())
            .map(label)
            .collect(),
        assignees: team
            .members
            .nodes
            .iter()
            .map(|user| IssueAssigneeCandidate {
                id: user.id.clone(),
                actor: actor(user),
            })
            .collect(),
        truncated,
    }
}

/// Label names to ids, within the team's and the workspace's labels.
fn label_ids(resolved: &ResolvedTeam, names: &[String]) -> Result<Vec<String>, IssueTrackerError> {
    names
        .iter()
        .map(|name| {
            resolved
                .team
                .labels
                .nodes
                .iter()
                .chain(resolved.workspace_labels.nodes.iter())
                .find(|label| label.name.eq_ignore_ascii_case(name.trim()))
                .map(|label| label.id.clone())
                .ok_or_else(|| {
                    IssueTrackerError::Refused(format!(
                        "Linear has no label named \"{name}\" for this team."
                    ))
                })
        })
        .collect()
}

/// The one assignee `assignees` names, by id, email or handle; `None` clears.
fn assignee_id(
    resolved: &ResolvedTeam,
    assignees: &[String],
) -> Result<Option<String>, IssueTrackerError> {
    let [wanted] = assignees else {
        return if assignees.is_empty() {
            Ok(None)
        } else {
            Err(one_assignee())
        };
    };
    resolved
        .team
        .members
        .nodes
        .iter()
        .find(|user| {
            user.id == *wanted
                || user
                    .email
                    .as_deref()
                    .is_some_and(|email| email.eq_ignore_ascii_case(wanted))
                || user
                    .display_name
                    .as_deref()
                    .is_some_and(|name| name.eq_ignore_ascii_case(wanted))
        })
        .map(|user| Some(user.id.clone()))
        .ok_or_else(|| {
            IssueTrackerError::Refused(format!("\"{wanted}\" is not a member of this Linear team."))
        })
}

#[cfg(test)]
#[path = "issue_tracker_tests.rs"]
mod tests;
