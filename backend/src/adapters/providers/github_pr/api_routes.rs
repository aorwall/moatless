//! Typed `/api/v1/github/*` API surface for sandbox agents (via `moat gh`).
//!
//! Mirrors the `moat linear` pattern: instead of the agent shelling out to
//! `gh pr create` / `gh pr comment` and the backend reconstructing the side
//! effects by parsing the shell command post-hoc (fragile — see
//! `github_pr::shell_reconcile`), these endpoints make the side effects part of
//! the API call itself:
//! - task↔PR binding upsert (`connect_github_binding`)
//! - `Moatless task: <url>` footer baked into the PR body at creation
//! - webhook echo dedup (`recently_sent`) for the PR-opened / comment events
//!
//! Tokens come from the calling user's own GitHub row and nothing else — the
//! same rule the sandbox's `GITHUB_TOKEN` follows since SP-2457, so the
//! identity that pushed the branch is the identity that opens the PR. Neither
//! the task repository's persisted scope nor the global `GITHUB_CHANNEL_TOKEN`
//! is consulted, which is also what lets these routes reach a repository the
//! task isn't bound to: the user's own access is the only thing that decides.

use axum::Json;
use axum::extract::{Path, State};
use moatless_api_schemas::github::{
    GithubBindPrRequest, GithubBindPrResponse, GithubCommentRequest, GithubCommentResponse,
    GithubCreatePrRequest, GithubCreatePrResponse, GithubReplyRequest,
};
use serde_json::{Value, json};
use tracing::{info, warn};

use crate::adapters::common::ensure_task_access;
use crate::adapters::error::AdapterError;
use crate::adapters::footer::FOOTER_MARKER;
use crate::extractors::AuthUser;
use crate::state::AppState;

use super::api::{GithubPrApi, MAX_COMMENT_LENGTH};
use super::external_id;
use super::shell_reconcile::compose_footer;

#[utoipa::path(
    post,
    path = "/api/v1/github/repos/{owner}/{repo}/pulls",
    tag = "github",
    params(
        ("owner" = String, Path, description = "Repository owner (org or user)"),
        ("repo" = String, Path, description = "Repository name"),
    ),
    request_body = GithubCreatePrRequest,
    responses(
        (status = 200, description = "Pull request created", body = GithubCreatePrResponse),
        (status = 400, description = "Bad request / adapter not configured", body = crate::error::ErrorBody),
        (status = 404, description = "Task not found", body = crate::error::ErrorBody),
        (status = 502, description = "GitHub API error / no usable GitHub token", body = crate::error::ErrorBody),
    )
)]
pub async fn create_pr(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((owner, repo)): Path<(String, String)>,
    Json(req): Json<GithubCreatePrRequest>,
) -> Result<Json<GithubCreatePrResponse>, AdapterError> {
    let ctx = auth.scope_context();
    if let Some(task_id) = req.task_id.as_deref() {
        ensure_task_access(&state, &ctx, task_id).await?;
    }

    let api = resolve_github_api(&state, &auth.user_id.to_string(), &owner, &repo).await?;

    let base = match req.base.as_deref() {
        Some(b) => b.to_string(),
        None => api.get_repo_default_branch(&owner, &repo).await?,
    };

    let body = build_pr_body(
        req.body.as_deref().unwrap_or(""),
        task_footer_url(&state, req.task_id.as_deref()).as_deref(),
    );

    let pr = api
        .create_pr(
            &owner,
            &repo,
            &req.title,
            &body,
            &req.head,
            &base,
            req.draft.unwrap_or(false),
        )
        .await?;

    let number = pr
        .get("number")
        .and_then(Value::as_u64)
        .ok_or_else(|| AdapterError::AdapterApi("GitHub create_pr: missing number".into()))?;
    let html_url = pr
        .get("html_url")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let title = pr
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or(&req.title)
        .to_string();
    let pr_state = pr
        .get("state")
        .and_then(Value::as_str)
        .unwrap_or("open")
        .to_string();

    let binding_id = if let Some(task_id) = req.task_id.as_deref() {
        let bid = bind_task_to_pr(
            &state,
            &owner,
            &repo,
            number,
            task_id,
            &auth.user_id.to_string(),
        )
        .await;
        state
            .recently_sent
            .note(task_id, &external_id::pr_opened(number));
        bid
    } else {
        None
    };

    Ok(Json(GithubCreatePrResponse {
        number,
        html_url,
        title,
        state: pr_state,
        binding_id,
    }))
}

#[utoipa::path(
    post,
    path = "/api/v1/github/repos/{owner}/{repo}/issues/{number}/comments",
    tag = "github",
    params(
        ("owner" = String, Path, description = "Repository owner (org or user)"),
        ("repo" = String, Path, description = "Repository name"),
        ("number" = u64, Path, description = "PR or issue number"),
    ),
    request_body = GithubCommentRequest,
    responses(
        (status = 200, description = "Comment posted", body = GithubCommentResponse),
        (status = 400, description = "Bad request / adapter not configured", body = crate::error::ErrorBody),
        (status = 404, description = "Task not found", body = crate::error::ErrorBody),
        (status = 502, description = "GitHub API error / no usable GitHub token", body = crate::error::ErrorBody),
    )
)]
pub async fn post_comment(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((owner, repo, number)): Path<(String, String, u64)>,
    Json(req): Json<GithubCommentRequest>,
) -> Result<Json<GithubCommentResponse>, AdapterError> {
    let ctx = auth.scope_context();
    if let Some(task_id) = req.task_id.as_deref() {
        ensure_task_access(&state, &ctx, task_id).await?;
    }

    let api = resolve_github_api(&state, &auth.user_id.to_string(), &owner, &repo).await?;

    let body = body_with_task_footer(&state, req.task_id.as_deref(), &req.body).await;
    let comment_id = api.post_comment(&owner, &repo, number, &body).await?;

    let binding_id = if let Some(task_id) = req.task_id.as_deref() {
        // Note the echo dedup BEFORE the (slower) binding upsert so the
        // inbound webhook can't race past it.
        state
            .recently_sent
            .note(task_id, &external_id::issue_comment(comment_id));
        bind_task_to_pr(
            &state,
            &owner,
            &repo,
            number,
            task_id,
            &auth.user_id.to_string(),
        )
        .await
    } else {
        None
    };

    // GitHub redirects `/issues/<n>#issuecomment-<id>` to `/pull/<n>` for PRs,
    // so this URL form works for both PRs and issues without an extra fetch.
    let html_url =
        format!("https://github.com/{owner}/{repo}/issues/{number}#issuecomment-{comment_id}");

    Ok(Json(GithubCommentResponse {
        id: comment_id,
        html_url,
        binding_id,
    }))
}

#[utoipa::path(
    post,
    path = "/api/v1/github/repos/{owner}/{repo}/pulls/{number}/comments/{comment_id}/replies",
    tag = "github",
    params(
        ("owner" = String, Path, description = "Repository owner (org or user)"),
        ("repo" = String, Path, description = "Repository name"),
        ("number" = u64, Path, description = "PR number"),
        ("comment_id" = u64, Path, description = "Review comment id to reply to"),
    ),
    request_body = GithubReplyRequest,
    responses(
        (status = 200, description = "Threaded reply posted", body = GithubCommentResponse),
        (status = 400, description = "Bad request / adapter not configured", body = crate::error::ErrorBody),
        (status = 404, description = "Task not found", body = crate::error::ErrorBody),
        (status = 502, description = "GitHub API error / no usable GitHub token", body = crate::error::ErrorBody),
    )
)]
pub async fn reply_to_review_comment(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((owner, repo, number, comment_id)): Path<(String, String, u64, u64)>,
    Json(req): Json<GithubReplyRequest>,
) -> Result<Json<GithubCommentResponse>, AdapterError> {
    let ctx = auth.scope_context();
    if let Some(task_id) = req.task_id.as_deref() {
        ensure_task_access(&state, &ctx, task_id).await?;
    }

    let api = resolve_github_api(&state, &auth.user_id.to_string(), &owner, &repo).await?;

    let body = body_with_task_footer(&state, req.task_id.as_deref(), &req.body).await;
    let new_id = api
        .reply_to_review_comment(&owner, &repo, number, comment_id, &body)
        .await?;

    let binding_id = if let Some(task_id) = req.task_id.as_deref() {
        // Note the echo dedup BEFORE the (slower) binding upsert so the
        // inbound webhook can't race past it.
        state
            .recently_sent
            .note(task_id, &external_id::review_comment(new_id));
        bind_task_to_pr(
            &state,
            &owner,
            &repo,
            number,
            task_id,
            &auth.user_id.to_string(),
        )
        .await
    } else {
        None
    };

    let html_url = format!("https://github.com/{owner}/{repo}/pull/{number}#discussion_r{new_id}");

    Ok(Json(GithubCommentResponse {
        id: new_id,
        html_url,
        binding_id,
    }))
}

#[utoipa::path(
    post,
    path = "/api/v1/github/repos/{owner}/{repo}/pulls/{number}/bind",
    tag = "github",
    params(
        ("owner" = String, Path, description = "Repository owner (org or user)"),
        ("repo" = String, Path, description = "Repository name"),
        ("number" = u64, Path, description = "PR number"),
    ),
    request_body = GithubBindPrRequest,
    responses(
        (status = 200, description = "Pull request bound to task", body = GithubBindPrResponse),
        (status = 400, description = "Bad request / adapter not configured", body = crate::error::ErrorBody),
        (status = 404, description = "Task not found", body = crate::error::ErrorBody),
    )
)]
pub async fn bind_pr(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((owner, repo, number)): Path<(String, String, u64)>,
    Json(req): Json<GithubBindPrRequest>,
) -> Result<Json<GithubBindPrResponse>, AdapterError> {
    let ctx = auth.scope_context();
    ensure_task_access(&state, &ctx, &req.task_id).await?;

    let api = resolve_github_api(&state, &auth.user_id.to_string(), &owner, &repo).await?;
    let pr = api.get_pr(&owner, &repo, number).await?;
    let html_url = pr
        .html_url
        .unwrap_or_else(|| format!("https://github.com/{owner}/{repo}/pull/{number}"));

    let binding_id = bind_task_to_pr(
        &state,
        &owner,
        &repo,
        number,
        &req.task_id,
        &auth.user_id.to_string(),
    )
    .await;

    Ok(Json(GithubBindPrResponse {
        number,
        html_url,
        binding_id,
    }))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Resolve a GitHub API client from `user_id`'s own GitHub credential, minted
/// against `owner/repo_name`.
///
/// `user_id` is the task's run-as identity — the sandbox's `X-User-Token` JWT
/// and its `GITHUB_TOKEN` are cut from the same `grant.ctx.user_id` — so this
/// is the same authority `resolve_repository_github_token_with` applies to the
/// sandbox: their own row, no scope walk, no repository or global fallback.
/// Holding no row is "this user cannot reach GitHub", not a reason to borrow
/// someone else's token.
///
/// Which repository is requested therefore doesn't enter into it. A PR against
/// a repo the task isn't bound to resolves exactly like one against its own;
/// the task binding only decides where replies route back to.
async fn resolve_github_api(
    state: &AppState,
    user_id: &str,
    owner: &str,
    repo_name: &str,
) -> Result<GithubPrApi, AdapterError> {
    let token = crate::auth::credentials::resolve_run_as_user_github_token(
        state.provider_tokens.as_ref(),
        Some(state.provider_token_auth.as_ref()),
        state.github_app_service.as_deref(),
        state.github_apps.as_ref(),
        Some(state.secrets.as_ref()),
        user_id,
        owner,
        repo_name,
    )
    .await
    .map_err(|source| AdapterError::UserCredential {
        user_id: user_id.to_string(),
        source,
    })?
    .ok_or_else(|| {
        AdapterError::AdapterApi(format!(
            "User {user_id} has no GitHub credential — connect GitHub to act on {owner}/{repo_name}"
        ))
    })?;
    Ok(GithubPrApi::new(&token))
}

/// Build the task URL for the PR-body footer. Returns `None` when no task is
/// bound or the frontend URL isn't configured.
fn task_footer_url(state: &AppState, task_id: Option<&str>) -> Option<String> {
    let task_id = task_id?;
    let frontend_url = state.config.auth.frontend_url.trim_end_matches('/');
    if frontend_url.is_empty() {
        return None;
    }
    Some(format!("{frontend_url}/tasks/{task_id}"))
}

/// Compose the PR body, truncated to GitHub's limit, with the
/// `Moatless task:` footer baked in when a task URL is available.
fn build_pr_body(body: &str, task_url: Option<&str>) -> String {
    match task_url {
        Some(url) if !body.contains(FOOTER_MARKER) => {
            // Reserve room for `\n\n---\n<marker> <url>\n`.
            let reserve = url.len() + FOOTER_MARKER.len() + 8;
            let max_body = MAX_COMMENT_LENGTH.saturating_sub(reserve);
            compose_footer(truncate_on_char_boundary(body, max_body), url)
        }
        _ => truncate_on_char_boundary(body, MAX_COMMENT_LENGTH).to_string(),
    }
}

/// Truncate a comment body to GitHub's limit and append the "view task" footer
/// when a task is associated. The user body is truncated first, reserving room
/// for the footer so the total stays within `MAX_COMMENT_LENGTH`.
async fn body_with_task_footer(state: &AppState, task_id: Option<&str>, body: &str) -> String {
    let footer = match task_id {
        Some(tid) => crate::adapters::footer::task_comment_footer(state, tid).await,
        None => None,
    };
    match footer {
        Some(f) => {
            let max_body = MAX_COMMENT_LENGTH.saturating_sub(f.len());
            format!("{}{}", truncate_on_char_boundary(body, max_body), f)
        }
        None => truncate_on_char_boundary(body, MAX_COMMENT_LENGTH).to_string(),
    }
}

/// Truncate to at most `max` bytes without splitting a UTF-8 character.
fn truncate_on_char_boundary(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut idx = max;
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    &s[..idx]
}

/// Upsert the task↔PR binding. Failure is logged and swallowed — the PR /
/// comment already exists on GitHub, so the mutation must still succeed
/// (mirrors the post-persist hooks behavior).
async fn bind_task_to_pr(
    state: &AppState,
    owner: &str,
    repo: &str,
    pr_number: u64,
    task_id: &str,
    user_id: &str,
) -> Option<String> {
    let source_id = format!("{owner}/{repo}");
    let thread_properties = json!({
        "owner": owner,
        "repo": repo,
        "pr_number": pr_number,
    });
    match crate::loops::firing::listen::connect_github_binding(
        state,
        &source_id,
        task_id,
        thread_properties,
        Some(user_id),
    )
    .await
    {
        Ok((rule, binding)) => {
            info!(
                task_id,
                pr_number,
                subscription_id = rule.as_ref().map_or("<none>", |r| r.id.as_str()),
                binding_id = %binding.id,
                "Connected GitHub PR binding via typed API"
            );
            Some(binding.id)
        }
        Err(e) => {
            warn!(task_id, pr_number, error = %e, "Failed to connect GitHub PR binding via typed API");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_respects_char_boundaries() {
        assert_eq!(truncate_on_char_boundary("hello", 10), "hello");
        assert_eq!(truncate_on_char_boundary("hello", 3), "hel");
        // 'é' is 2 bytes — truncating mid-char backs off to the boundary.
        assert_eq!(truncate_on_char_boundary("é", 1), "");
        assert_eq!(truncate_on_char_boundary("aé", 2), "a");
    }

    #[test]
    fn pr_body_includes_footer_when_task_url_present() {
        let out = build_pr_body("## Summary\nFix.", Some("https://x/tasks/t_1"));
        assert!(out.starts_with("## Summary"));
        assert!(out.contains("---\nMoatless task: https://x/tasks/t_1\n"));
    }

    #[test]
    fn pr_body_skips_footer_when_already_present() {
        let body = "body\n\n---\nMoatless task: https://x/tasks/t_1\n";
        let out = build_pr_body(body, Some("https://x/tasks/t_1"));
        assert_eq!(out, body);
    }

    #[test]
    fn pr_body_without_task_url_is_unchanged() {
        assert_eq!(build_pr_body("body", None), "body");
    }

    #[test]
    fn pr_body_truncates_before_appending_footer() {
        let url = "https://x/tasks/t_1";
        let long_body = "a".repeat(MAX_COMMENT_LENGTH + 100);
        let out = build_pr_body(&long_body, Some(url));
        assert!(out.len() <= MAX_COMMENT_LENGTH);
        assert!(out.contains(FOOTER_MARKER));
    }
}
