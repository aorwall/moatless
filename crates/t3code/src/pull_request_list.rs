//! `pullRequests.list`: the pull requests of every GitHub repository each
//! readable project places, read from GitHub with the viewer's credentials.

use std::collections::{BTreeMap, HashSet};

use futures::StreamExt;
use moatless_types::access_control::ScopeContext;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::ports::{
    PullRequestError, PullRequestListItem, PullRequestListPage, PullRequestListQuery,
    PullRequestReviewSource,
};
use crate::pull_request_review::{PullRequestReviewError, ReviewPorts, non_empty_or};
use crate::workspace_listing::WorkspaceListing;

const HOST: &str = moatless_workspace_types::repository::GITHUB_HOST;
/// Rows per repository when the page names no limit, as upstream's server.
const DEFAULT_LIMIT: u32 = 99;
const MAX_LIMIT: u32 = 500;
/// Repositories read at once.
const CONCURRENCY: usize = 8;
/// Why `involvement` cannot be served: it names the viewer's own GitHub login.
const NO_OWN_LOGIN: &str =
    "Connect or reconnect your GitHub account to list the pull requests you author or review.";

/// One project's repository, and where its listing resumes.
struct Source {
    project_id: String,
    project_title: String,
    repository: String,
    cursor: Option<String>,
}

/// The contract's `PullRequestListInput`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PullRequestListInput {
    state: Option<String>,
    involvement: Option<String>,
    filters: Option<PullRequestListFilters>,
    project_id: Option<String>,
    project_ids: Option<Vec<String>>,
    host: Option<String>,
    limit: Option<u64>,
    /// Keyed `"<host> <repository>"`.
    cursors: Option<BTreeMap<String, String>>,
    query: Option<String>,
}

/// The contract's `PullRequestListFilters`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PullRequestListFilters {
    draft: Option<String>,
    review: Option<String>,
    checks: Option<String>,
    labels: Option<Vec<Vec<String>>>,
    excluded_labels: Option<Vec<String>>,
    author: Option<String>,
}

/// The listing `input` asks for.
pub(crate) async fn list(
    ports: &ReviewPorts<'_>,
    viewer: &ScopeContext,
    input: &PullRequestListInput,
) -> Result<Value, PullRequestReviewError> {
    let host_named = text(input.host.as_deref());
    if host_named.as_deref().is_some_and(|host| host != HOST) {
        return Ok(result(
            &BTreeMap::new(),
            0,
            Vec::new(),
            Vec::new(),
            Map::new(),
        ));
    }
    let viewer_login = ports
        .auth
        .github_identity(viewer)
        .await
        .and_then(|identity| identity.account)
        .filter(|login| !login.trim().is_empty());
    let query = PullRequestListQuery {
        viewer_login: viewer_login.clone(),
        ..list_query(input)
    };
    let limit = input.limit.map_or(DEFAULT_LIMIT, |limit| {
        limit.clamp(1, u64::from(MAX_LIMIT)) as u32
    });
    let (sources, mut errors) = sources(
        ports.workspaces,
        ports.host,
        viewer,
        &projects_named(input),
        input.cursors.as_ref(),
    )
    .await?;

    let reads = futures::stream::iter(
        sources
            .iter()
            .map(|source| read(ports.host, viewer, &query, limit, source))
            .collect::<Vec<_>>(),
    )
    .buffer_unordered(CONCURRENCY)
    .collect::<Vec<_>>()
    .await;

    let mut entries = Vec::new();
    let mut next_cursors = Map::new();
    for (source, page) in reads {
        match page {
            Ok(PullRequestListPage { items, next_cursor }) => {
                entries.extend(
                    items
                        .into_iter()
                        .filter_map(|item| entry(source, &item, viewer_login.as_deref())),
                );
                if let Some(cursor) = next_cursor {
                    next_cursors.insert(format!("{HOST} {}", source.repository), json!(cursor));
                }
            }
            Err(error) => errors.push(source_error(
                &source.project_id,
                &source.project_title,
                &error,
            )),
        }
    }
    entries.sort_by(|a, b| b["updatedAt"].as_str().cmp(&a["updatedAt"].as_str()));
    let viewers = viewer_login.map_or_default(|login| BTreeMap::from([(HOST.to_string(), login)]));
    Ok(result(
        &viewers,
        sources.len(),
        entries,
        errors,
        next_cursors,
    ))
}

/// One repository's page. A named function rather than a closure, so the
/// future's borrows are not inferred per call (`FnOnce` is not general enough).
async fn read<'a>(
    host: &dyn PullRequestReviewSource,
    viewer: &ScopeContext,
    query: &PullRequestListQuery,
    limit: u32,
    source: &'a Source,
) -> (&'a Source, Result<PullRequestListPage, PullRequestError>) {
    if query.involvement.is_some() && query.viewer_login.is_none() {
        let refusal = PullRequestError::Unavailable(NO_OWN_LOGIN.to_string());
        return (source, Err(refusal));
    }
    let page = host
        .list(
            viewer,
            &source.project_id,
            &source.repository,
            query,
            limit,
            source.cursor.as_deref(),
        )
        .await;
    (source, page)
}

/// The `projectIds` and `projectId` the listing is narrowed to; empty for
/// every readable project.
fn projects_named(input: &PullRequestListInput) -> Vec<String> {
    let mut named = input.project_ids.clone().unwrap_or_default();
    named.extend(text(input.project_id.as_deref()));
    named
}

/// The repositories to read — each project's placed ones, then those its
/// readable Tasks work in, as [`crate::pull_request_review::reach`] opens —
/// each under the first project naming it, and an `errors` entry for each
/// project whose placed repositories could not be resolved. A continuation
/// reads only the repositories it names.
async fn sources(
    workspaces: &dyn WorkspaceListing,
    host: &dyn PullRequestReviewSource,
    viewer: &ScopeContext,
    named: &[String],
    cursors: Option<&BTreeMap<String, String>>,
) -> Result<(Vec<Source>, Vec<Value>), PullRequestReviewError> {
    let projects: Vec<(String, String)> = workspaces
        .list_readable_including(viewer, &[])
        .await?
        .into_iter()
        .filter(|listed| listed.readable)
        .filter(|listed| named.is_empty() || named.contains(&listed.workspace.id))
        .map(|listed| {
            let title = listed.workspace.name.trim().to_string();
            (listed.workspace.id, title)
        })
        .collect();
    // `buffered` keeps the projects' order, so the first project naming a
    // repository is the one that lists it.
    let resolved = futures::stream::iter(projects.into_iter().map(|(project_id, title)| async {
        let repositories = match host.project_repositories(viewer, &project_id).await {
            // A failed Task lookup costs only the Tasks' repositories, never
            // the placed ones.
            Ok(mut repositories) => match host.task_repositories(viewer, &project_id).await {
                Ok(worked_in) => {
                    repositories.extend(worked_in);
                    Ok(repositories)
                }
                Err(error) => {
                    tracing::warn!(?error, project_id, "Task repositories read failed");
                    Ok(repositories)
                }
            },
            Err(error) => Err(error),
        };
        (project_id, title, repositories)
    }))
    .buffered(CONCURRENCY)
    .collect::<Vec<_>>()
    .await;

    let mut seen = HashSet::new();
    let mut sources = Vec::new();
    let mut errors = Vec::new();
    for (project_id, title, repositories) in resolved {
        let repositories = match repositories {
            Ok(repositories) => repositories,
            Err(error) => {
                errors.push(source_error(&project_id, &title, &error));
                continue;
            }
        };
        for repository in repositories {
            if !seen.insert(repository.to_ascii_lowercase()) {
                continue;
            }
            let cursor = match cursors {
                Some(cursors) => match cursors.get(&format!("{HOST} {repository}")) {
                    Some(cursor) => Some(cursor.clone()),
                    None => continue,
                },
                None => None,
            };
            sources.push(Source {
                project_title: if title.is_empty() {
                    repository.clone()
                } else {
                    title.clone()
                },
                project_id: project_id.clone(),
                repository,
                cursor,
            });
        }
    }
    Ok((sources, errors))
}

fn list_query(input: &PullRequestListInput) -> PullRequestListQuery {
    let filters = input.filters.as_ref();
    let filter = |field: fn(&PullRequestListFilters) -> Option<&str>| text(filters.and_then(field));
    PullRequestListQuery {
        state: text(input.state.as_deref()).unwrap_or_else(|| "open".to_string()),
        involvement: text(input.involvement.as_deref()),
        viewer_login: None,
        text: text(input.query.as_deref()),
        draft: filter(|filters| filters.draft.as_deref()),
        review: filter(|filters| filters.review.as_deref()),
        checks: filter(|filters| filters.checks.as_deref()),
        labels: filters
            .and_then(|filters| filters.labels.clone())
            .unwrap_or_default(),
        excluded_labels: filters
            .and_then(|filters| filters.excluded_labels.clone())
            .unwrap_or_default(),
        author: filter(|filters| filters.author.as_deref()),
    }
}

/// `item` as a `PullRequestListEntry`, or `None` when the host left out a
/// field the contract requires.
fn entry(source: &Source, item: &PullRequestListItem, viewer: Option<&str>) -> Option<Value> {
    if item.number == 0 || item.url.trim().is_empty() || item.created_at.trim().is_empty() {
        return None;
    }
    let author = item.author_login.as_deref();
    let viewer_review_requested = viewer.is_some_and(|viewer| {
        author != Some(viewer)
            && item
                .requested_reviewers
                .iter()
                .any(|login| login.eq_ignore_ascii_case(viewer))
    });
    let mut entry = json!({
        "provider": "github",
        "host": HOST,
        "projectId": source.project_id,
        "projectTitle": source.project_title,
        "repository": source.repository,
        "number": item.number,
        "title": non_empty_or(&item.title, &format!("#{}", item.number)),
        "url": item.url,
        "author": author.map(|login| json!({
            "login": login,
            "name": null,
            "avatarUrl": item.author_avatar_url,
            "isBot": item.author_is_bot,
        })),
        "headBranch": non_empty_or(&item.head_ref, "unknown"),
        "baseBranch": non_empty_or(&item.base_ref, "unknown"),
        "state": item.state,
        "isDraft": item.is_draft,
        "mergeability": item.mergeability,
        "additions": item.additions,
        "deletions": item.deletions,
        "createdAt": item.created_at,
        "updatedAt": non_empty_or(&item.updated_at, &item.created_at),
        "viewerReviewRequested": viewer_review_requested,
        "labels": item.labels.iter().map(|(name, color)| json!({
            "name": name,
            "color": color,
        })).collect::<Vec<_>>(),
    });
    if let Some(decision) = &item.review_decision {
        entry["reviewDecision"] = json!(decision);
    }
    if let Some(checks) = &item.checks_state {
        entry["checksState"] = json!(checks);
    }
    Some(entry)
}

fn result(
    viewers: &BTreeMap<String, String>,
    repositories: usize,
    entries: Vec<Value>,
    errors: Vec<Value>,
    next_cursors: Map<String, Value>,
) -> Value {
    let providers = if repositories == 0 {
        Vec::new()
    } else {
        vec![json!({
            "host": HOST,
            "kind": "github",
            "searchesOnHost": true,
            "actions": [],
            "projectCount": repositories,
            "configured": true,
            "detail": null,
        })]
    };
    json!({
        "viewers": viewers,
        "providers": providers,
        "entries": entries,
        "errors": errors,
        "truncated": !next_cursors.is_empty(),
        "nextCursors": next_cursors,
    })
}

/// An `errors` entry for a project whose pull requests could not be read.
fn source_error(project_id: &str, project_title: &str, error: &PullRequestError) -> Value {
    let message = match error {
        PullRequestError::Unavailable(message) => message.clone(),
        PullRequestError::NotFound => "The repository was not found.".to_string(),
        PullRequestError::Failed(cause) => {
            tracing::error!(%cause, project_id, "Pull request listing failed");
            "The repository's pull requests could not be read.".to_string()
        }
    };
    json!({
        "projectId": project_id,
        "projectTitle": project_title,
        "message": message,
    })
}

/// `value` trimmed, or `None` when that leaves nothing.
fn text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
#[path = "pull_request_list_tests.rs"]
pub(crate) mod tests;
