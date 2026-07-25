//! Repository HTTP handlers.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;

use tracing::{debug, info, warn};

use moatless_api_schemas::loops::{LoopSyncSummary, SyncConfigResponse};

use crate::access_control::resolve_scope_for_create_auth;
use crate::extractors::AuthUser;
use crate::loops::sync::sync_repo_loops;
use crate::secret::models::looks_like_secret_key;
use crate::state::AppState;
use moatless_backend_core::scope::Scope;

use super::access::{access_error_for, parse_github_owner_repo, resolve_repository_github_token};
use super::config_reader::{
    validate_repository_access_target, verify_repository_access, AccessStatus,
};
use super::dao::RepositoryPatch;
use super::error::RepositoryError;
use super::models::{Repository, RepositoryKind, RepositoryProvider};
use super::schemas::*;
use crate::workspace::git_sync::sync_repo_workspaces;

pub(crate) fn validate_non_secret_env_vars(
    env_vars: Option<&std::collections::HashMap<String, String>>,
) -> Result<(), RepositoryError> {
    let Some(env_vars) = env_vars else {
        return Ok(());
    };
    for key in env_vars.keys() {
        if looks_like_secret_key(key) {
            return Err(RepositoryError::BadRequest(format!(
                "Repository env_vars are non-secret config only; move {key} to scoped secrets"
            )));
        }
    }
    Ok(())
}

/// Create a new repository.
#[utoipa::path(
    post,
    path = "/api/v1/repositories",
    tag = "repositories",
    request_body = CreateRepositoryRequest,
    responses(
        (status = 201, description = "Repository created", body = RepositoryResponse),
        (status = 409, description = "Repository already exists", body = crate::error::ErrorBody)
    )
)]
pub async fn create_repository(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<CreateRepositoryRequest>,
) -> Result<(StatusCode, Json<RepositoryResponse>), RepositoryError> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now();
    let user_id = auth.user_id;

    let resolved_scope = resolve_scope_for_create_auth(&auth, req.scope)?;
    let scope_ctx = auth.scope_context();

    ensure_unique_remote_url(
        state.repositories.as_ref(),
        None,
        req.remote_url.as_deref(),
        resolved_scope.scope,
        resolved_scope.owner_user_id.clone(),
    )
    .await?;

    let row = Repository {
        id: id.clone(),
        name: req.name,
        kind: req.kind,
        remote_url: req.remote_url,
        default_branch: req.default_branch,
        description: req.description,
        provider: req.provider,
        scope: resolved_scope.scope,
        owner_user_id: resolved_scope.owner_user_id,
        created_by: Some(user_id.to_string()),
        access_status: "unchecked".to_string(),
        access_checked_at: None,
        access_error: None,
        deleted: Some(false),
        created_at: Some(now),
        updated_at: Some(now),
    };

    state
        .repositories
        .insert(&row)
        .await
        .map_err(|error| RepositoryError::from_write_error(error, row.remote_url.as_deref()))?;

    // A repository is only a git connection; its run-config flows to the
    // workspace via the git sync below, not into repo columns.
    let final_row = state
        .repositories
        .find_by_id(&id, &scope_ctx)
        .await?
        .unwrap_or(row);

    // Sync `.moatless/loops/` into the trigger tables. Best-effort and
    // non-fatal, exactly like the config auto-read above.
    if final_row.remote_url.is_some() {
        match sync_repo_loops(&state, &final_row, &auth).await {
            Ok(summary) => {
                if summary.created + summary.updated + summary.removed > 0
                    || !summary.skipped.is_empty()
                {
                    info!(
                        repository_id = %id,
                        created = summary.created,
                        updated = summary.updated,
                        removed = summary.removed,
                        skipped = summary.skipped.len(),
                        "Synced repo loops on create"
                    );
                }
            }
            Err(e) => {
                warn!(repository_id = %id, error = %e, "Failed to sync repo loops on create");
            }
        }

        // Sync git-declared workspaces (`.moatless/workspaces.json` or the legacy
        // `.moatless/config.json`). Best-effort and non-fatal, like loop sync.
        log_workspace_sync(&state, &final_row, &auth, "create").await;
    }

    let resp: RepositoryResponse = final_row.into();
    Ok((StatusCode::CREATED, Json(resp)))
}

/// Run `sync_repo_workspaces` and log the outcome (`Ok=>info!`, `Err=>warn!`),
/// never blocking the caller.
async fn log_workspace_sync(state: &AppState, repo: &Repository, auth: &AuthUser, phase: &str) {
    match sync_repo_workspaces(state, repo, auth).await {
        Ok(summary) => {
            if summary.created + summary.updated + summary.removed > 0
                || !summary.skipped.is_empty()
            {
                info!(
                    repository_id = %repo.id,
                    created = summary.created,
                    updated = summary.updated,
                    removed = summary.removed,
                    skipped = summary.skipped.len(),
                    phase,
                    "Synced git workspaces"
                );
            }
        }
        Err(e) => {
            warn!(repository_id = %repo.id, error = %e, phase, "Failed to sync git workspaces");
        }
    }
}

/// List all repositories for the current user.
#[utoipa::path(
    get,
    path = "/api/v1/repositories",
    tag = "repositories",
    responses(
        (status = 200, description = "List of repositories", body = Vec<RepositoryResponse>)
    )
)]
pub async fn list_repositories(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Vec<RepositoryResponse>>, RepositoryError> {
    let rows = state.repositories.list_all(&auth.scope_context()).await?;
    let repos: Vec<RepositoryResponse> = rows.into_iter().map(Into::into).collect();
    Ok(Json(repos))
}

/// Get a repository by ID.
#[utoipa::path(
    get,
    path = "/api/v1/repositories/{repository_id}",
    tag = "repositories",
    params(
        ("repository_id" = String, Path, description = "Repository identifier")
    ),
    responses(
        (status = 200, description = "Repository details", body = RepositoryResponse),
        (status = 404, description = "Repository not found", body = crate::error::ErrorBody)
    )
)]
pub async fn get_repository(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(repository_id): Path<String>,
) -> Result<Json<RepositoryResponse>, RepositoryError> {
    let row = state
        .repositories
        .find_by_id(&repository_id, &auth.scope_context())
        .await?
        .ok_or_else(|| RepositoryError::NotFound(repository_id))?;

    Ok(Json(row.into()))
}

/// Update a repository by ID.
#[utoipa::path(
    put,
    path = "/api/v1/repositories/{repository_id}",
    tag = "repositories",
    params(
        ("repository_id" = String, Path, description = "Repository identifier")
    ),
    request_body = UpdateRepositoryRequest,
    responses(
        (status = 200, description = "Repository updated", body = RepositoryResponse),
        (status = 404, description = "Repository not found", body = crate::error::ErrorBody),
        (status = 409, description = "Repository URL already exists", body = crate::error::ErrorBody)
    )
)]
pub async fn update_repository(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(repository_id): Path<String>,
    Json(req): Json<UpdateRepositoryRequest>,
) -> Result<Json<RepositoryResponse>, RepositoryError> {
    let ctx = auth.scope_context();
    let existing = state
        .repositories
        .find_by_id(&repository_id, &ctx)
        .await?
        .ok_or_else(|| RepositoryError::NotFound(repository_id.clone()))?;

    let access_context_changed = req.scope.is_some()
        || req
            .remote_url
            .as_ref()
            .is_some_and(|remote_url| existing.remote_url.as_ref() != Some(remote_url));

    // User ownership is immutable here: re-submitting `scope: user` preserves
    // the existing owner even when an admin is editing the row.
    // Global moves require admin (enforced by resolve_scope_for_create_auth).
    let (scope, owner_user_id) = match req.scope {
        Some(Scope::User) => {
            let existing_owner_user_id = preserved_user_scope(&existing)?;
            (Some(Scope::User), Some(Some(existing_owner_user_id)))
        }
        Some(new_scope) => {
            let resolved = resolve_scope_for_create_auth(&auth, Some(new_scope))?;
            (Some(resolved.scope), Some(resolved.owner_user_id))
        }
        None => (None, None),
    };

    let target_scope = scope.unwrap_or(existing.scope);
    let target_owner_user_id = owner_user_id
        .clone()
        .unwrap_or_else(|| existing.owner_user_id.clone());
    let target_remote_url = req
        .remote_url
        .clone()
        .or_else(|| existing.remote_url.clone());

    ensure_unique_remote_url(
        state.repositories.as_ref(),
        Some(&repository_id),
        target_remote_url.as_deref(),
        target_scope,
        target_owner_user_id,
    )
    .await?;

    let patch = RepositoryPatch {
        scope,
        owner_user_id,
        name: req.name,
        remote_url: req.remote_url.map(Some),
        default_branch: req.default_branch.map(Some),
        description: req.description.map(Some),
        access_status: access_context_changed.then(|| "unchecked".to_string()),
        access_checked_at: access_context_changed.then_some(None),
        access_error: access_context_changed.then_some(None),
        ..Default::default()
    };

    state
        .repositories
        .update(&repository_id, &patch, &ctx)
        .await
        .map_err(|error| RepositoryError::from_write_error(error, target_remote_url.as_deref()))?;

    // Re-fetch updated row
    let updated = state
        .repositories
        .find_by_id(&repository_id, &ctx)
        .await?
        .ok_or_else(|| RepositoryError::NotFound(repository_id))?;

    Ok(Json(updated.into()))
}

async fn ensure_unique_remote_url(
    repositories: &dyn super::dao::RepoRepository,
    repository_id: Option<&str>,
    remote_url: Option<&str>,
    scope: Scope,
    owner_user_id: Option<String>,
) -> Result<(), RepositoryError> {
    let Some(remote_url) = remote_url else {
        return Ok(());
    };
    let url_key = normalize_remote_url(remote_url);
    let existing = repositories
        .find_by_normalized_remote_url(&url_key, scope, owner_user_id)
        .await?;
    if existing
        .as_ref()
        .is_some_and(|existing| Some(existing.id.as_str()) != repository_id)
    {
        return Err(RepositoryError::Conflict(remote_url.to_string()));
    }
    Ok(())
}

/// Preserve the immutable owner when a repository is submitted as user-scoped.
/// Ownerless global rows cannot be promoted to user scope because this API
/// intentionally has no owner-picker contract.
fn preserved_user_scope(existing: &Repository) -> Result<String, RepositoryError> {
    existing.owner_user_id.clone().ok_or_else(|| {
        RepositoryError::BadRequest("Cannot move an ownerless repository to user scope".to_string())
    })
}

/// Verify the persisted repository URL with the credentials appropriate to its
/// persisted scope, then stamp and return the updated repository.
#[utoipa::path(
    post,
    path = "/api/v1/repositories/{repository_id}/verify-access",
    tag = "repositories",
    params(
        ("repository_id" = String, Path, description = "Repository identifier")
    ),
    responses(
        (status = 200, description = "Repository access checked", body = RepositoryResponse),
        (status = 400, description = "Repository cannot be verified", body = crate::error::ErrorBody),
        (status = 403, description = "Write access required", body = crate::error::ErrorBody),
        (status = 404, description = "Repository not found", body = crate::error::ErrorBody)
    )
)]
pub async fn verify_access(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(repository_id): Path<String>,
) -> Result<Json<RepositoryResponse>, RepositoryError> {
    let repository = state
        .repositories
        .find_by_id(&repository_id, &auth.scope_context())
        .await?
        .ok_or_else(|| RepositoryError::NotFound(repository_id.clone()))?;

    let remote_url = repository
        .remote_url
        .as_deref()
        .ok_or_else(|| RepositoryError::BadRequest("Repository has no remote URL".to_string()))?;
    let provider = repository.provider.unwrap_or(RepositoryProvider::Github);
    validate_repository_access_target(remote_url, provider)?;

    let token = match provider {
        RepositoryProvider::Github => {
            resolve_repository_github_token(&state, &repository, remote_url).await?
        }
        RepositoryProvider::Gitlab => state
            .provider_tokens
            .find_provider_token(&auth.user_id.to_string(), provider.as_str())
            .await
            .map_err(RepositoryError::Db)?
            .map(|token| token.access_token),
        RepositoryProvider::Gitness => {
            return Err(RepositoryError::BadRequest(
                "Gitness access verification is not supported".to_string(),
            ));
        }
    };

    let status = verify_repository_access(remote_url, token.as_deref(), provider).await;
    let ctx = auth.scope_context();
    let updated = persist_access_verification(
        state.repositories.as_ref(),
        &repository,
        status,
        chrono::Utc::now(),
        &ctx,
    )
    .await?;

    Ok(Json(updated.into()))
}

async fn persist_access_verification(
    repositories: &dyn super::dao::RepoRepository,
    repository: &Repository,
    status: AccessStatus,
    checked_at: chrono::DateTime<chrono::Utc>,
    ctx: &moatless_backend_core::scope::ScopeContext,
) -> Result<Repository, RepositoryError> {
    let access_error = access_error_for(status, repository.remote_url.as_deref().unwrap_or(""));
    let patch = RepositoryPatch {
        access_status: Some(status.as_str().to_string()),
        access_checked_at: Some(Some(checked_at)),
        access_error: Some(access_error.clone()),
        ..Default::default()
    };
    if !repositories.update(&repository.id, &patch, ctx).await? {
        return Err(RepositoryError::NotFound(repository.id.clone()));
    }

    let mut updated = repository.clone();
    updated.access_status = status.as_str().to_string();
    updated.access_checked_at = Some(checked_at);
    updated.access_error = access_error;
    Ok(updated)
}

/// Delete a repository by ID (soft delete).
#[utoipa::path(
    delete,
    path = "/api/v1/repositories/{repository_id}",
    tag = "repositories",
    params(
        ("repository_id" = String, Path, description = "Repository identifier")
    ),
    responses(
        (status = 204, description = "Repository deleted"),
        (status = 404, description = "Repository not found", body = crate::error::ErrorBody)
    )
)]
pub async fn delete_repository(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(repository_id): Path<String>,
) -> Result<StatusCode, RepositoryError> {
    let ctx = auth.scope_context();
    state
        .repositories
        .find_by_id(&repository_id, &ctx)
        .await?
        .ok_or_else(|| RepositoryError::NotFound(repository_id.clone()))?;

    if !state.repositories.soft_delete(&repository_id, &ctx).await? {
        return Err(RepositoryError::NotFound(repository_id.clone()));
    }

    // Soft-delete workspaces this repo declared in git — once the source repo is
    // gone no future sync will prune them. Best-effort; the repo is already deleted.
    if let Err(e) = sqlx::query(
        "UPDATE workspaces SET deleted = true, updated_at = now() \
         WHERE source_repository_id = $1 AND source = 'git' \
           AND (deleted IS NULL OR deleted = false)",
    )
    .bind(&repository_id)
    .execute(&state.db)
    .await
    {
        warn!(repository_id = %repository_id, error = %e, "Failed to soft-delete git workspaces for deleted repo");
    }

    Ok(StatusCode::NO_CONTENT)
}

/// Convert a remote repository to a template.
#[utoipa::path(
    post,
    path = "/api/v1/repositories/{repository_id}/convert-to-template",
    tag = "repositories",
    params(
        ("repository_id" = String, Path, description = "Repository identifier")
    ),
    responses(
        (status = 200, description = "Repository converted to template", body = RepositoryResponse),
        (status = 400, description = "Cannot convert", body = crate::error::ErrorBody),
        (status = 404, description = "Repository not found", body = crate::error::ErrorBody)
    )
)]
pub async fn convert_to_template(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(repository_id): Path<String>,
) -> Result<Json<RepositoryResponse>, RepositoryError> {
    let ctx = auth.scope_context();
    let existing = state
        .repositories
        .find_by_id(&repository_id, &ctx)
        .await?
        .ok_or_else(|| RepositoryError::NotFound(repository_id.clone()))?;

    if existing.kind != RepositoryKind::Remote {
        return Err(RepositoryError::BadRequest(format!(
            "Only remote repositories can be converted to templates (current kind: {})",
            existing.kind
        )));
    }

    let patch = RepositoryPatch {
        kind: Some(RepositoryKind::Template),
        ..Default::default()
    };

    state.repositories.update(&repository_id, &patch, &ctx).await?;

    let updated = state
        .repositories
        .find_by_id(&repository_id, &ctx)
        .await?
        .ok_or_else(|| RepositoryError::NotFound(repository_id))?;

    Ok(Json(updated.into()))
}

/// Get branches for a repository.
#[utoipa::path(
    get,
    path = "/api/v1/repositories/{repository_id}/branches",
    tag = "repositories",
    params(
        ("repository_id" = String, Path, description = "Repository identifier")
    ),
    responses(
        (status = 200, description = "Branch list", body = BranchListResponse),
        (status = 400, description = "Bad request", body = crate::error::ErrorBody),
        (status = 403, description = "Access denied - token lacks permissions", body = crate::error::ErrorBody),
        (status = 404, description = "Repository not found", body = crate::error::ErrorBody)
    )
)]
pub async fn get_branches(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(repository_id): Path<String>,
) -> Result<Json<BranchListResponse>, RepositoryError> {
    let repo = state
        .repositories
        .find_by_id(&repository_id, &auth.scope_context())
        .await?
        .ok_or_else(|| RepositoryError::NotFound(repository_id.clone()))?;

    if repo.default_branch.is_none() {
        info!(repository_id = %repository_id, "No default branch set, fetching from provider");
    }

    let remote_url = repo
        .remote_url
        .as_deref()
        .ok_or_else(|| RepositoryError::BadRequest("Repository has no remote URL".to_string()))?;

    let provider = repo.provider.unwrap_or(RepositoryProvider::Github);
    let provider_str = provider.as_str();

    // Resolve a token for the repository's persisted scope. User-scoped
    // repositories always use their owner, never a viewing admin.
    let access_token = if provider == RepositoryProvider::Github {
        resolve_repository_github_token(&state, &repo, remote_url)
            .await?
            .ok_or_else(|| {
                RepositoryError::BadRequest(
                    "No github token configured. Please connect your github account.".into(),
                )
            })?
    } else {
        state.provider_tokens.find_provider_token(&auth.user_id.to_string(), provider_str)
            .await
            .map_err(RepositoryError::Db)?
            .ok_or_else(|| {
                RepositoryError::BadRequest(format!(
                    "No {provider_str} token configured. Please connect your {provider_str} account."
                ))
            })?
            .access_token
    };

    // For Gitness, we also need the base URL from provider_tokens metadata
    let result = match provider {
        RepositoryProvider::Github => fetch_github_branches(remote_url, &access_token).await?,
        RepositoryProvider::Gitlab => fetch_gitlab_branches(remote_url, &access_token).await?,
        RepositoryProvider::Gitness => {
            let full_token = state
                .provider_tokens
                .find_provider_token_full(&auth.user_id.to_string(), "gitness")
                .await
                .map_err(RepositoryError::Db)?;
            let base_url = full_token
                .as_ref()
                .and_then(|t| t.metadata.as_ref())
                .and_then(|m| m.get("base_url"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    RepositoryError::BadRequest(
                        "Gitness base URL not configured. Please reconnect your Gitness account in Settings.".to_string()
                    )
                })?
                .to_string();
            fetch_gitness_branches(remote_url, &access_token, &base_url).await?
        }
    };

    let default_branch = repo
        .default_branch
        .clone()
        .or(result.default_branch.clone());

    // Persist provider-reported default_branch back to DB if missing
    if repo.default_branch.is_none() {
        if let Some(ref branch) = default_branch {
            let patch = RepositoryPatch {
                default_branch: Some(Some(branch.clone())),
                ..Default::default()
            };
            if let Err(e) = state
                .repositories
                .update(&repository_id, &patch, &auth.scope_context())
                .await
            {
                warn!(repository_id = %repository_id, error = %e, "Failed to persist default branch");
            }
        }
    }

    let branches = result.branches;

    // Sort: default branch first, then alphabetical
    let mut entries: Vec<BranchEntry> = branches
        .into_iter()
        .map(|name| {
            let is_default = default_branch.as_deref() == Some(&name);
            BranchEntry { name, is_default }
        })
        .collect();

    entries.sort_by(|a, b| {
        // Default branch always first
        match (a.is_default, b.is_default) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.cmp(&b.name),
        }
    });

    Ok(Json(BranchListResponse {
        branches: entries,
        default_branch,
    }))
}

// =============================================================================
// Provider branch helpers
// =============================================================================

/// Result from fetching branches from a provider, including the detected default branch.
struct BranchFetchResult {
    branches: Vec<String>,
    default_branch: Option<String>,
}

/// Normalize a remote URL into a stable matching key: strip trailing `/` and a
/// trailing `.git`, then lowercase. For **matching** only — never persisted, so
/// the stored `remote_url` keeps its original casing.
pub(crate) fn normalize_remote_url(remote_url: &str) -> String {
    remote_url
        .trim()
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .trim_end_matches('/')
        .to_ascii_lowercase()
}

/// Fetch branches from GitHub API, including the default branch from the repo endpoint.
async fn fetch_github_branches(
    remote_url: &str,
    token: &str,
) -> Result<BranchFetchResult, RepositoryError> {
    let (owner, repo) = parse_github_owner_repo(remote_url).ok_or_else(|| {
        RepositoryError::BadRequest(format!("Cannot parse GitHub URL: {remote_url}"))
    })?;

    let client = reqwest::Client::new();

    // Fetch all branches (paginated)
    #[derive(serde::Deserialize)]
    struct GhBranch {
        name: String,
    }

    let mut all_branches: Vec<GhBranch> = Vec::new();
    let mut page = 1u32;
    loop {
        let resp = client
            .get(format!(
                "https://api.github.com/repos/{owner}/{repo}/branches?per_page=100&page={page}"
            ))
            .header("Authorization", format!("Bearer {token}"))
            .header("User-Agent", "moatless-vibe-backend")
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .map_err(|e| {
                RepositoryError::ProviderError(format!("GitHub API request failed: {e}"))
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();

            // 401/403 = access denied (token lacks permissions or repo access)
            if status == reqwest::StatusCode::UNAUTHORIZED
                || status == reqwest::StatusCode::FORBIDDEN
            {
                return Err(RepositoryError::Forbidden(
                    "Cannot access repository. Your GitHub token may lack the required permissions. \
                     Try reconnecting your GitHub account in Settings.".to_string(),
                ));
            }

            // 404 = repo not found (may also be private repo without access)
            if status == reqwest::StatusCode::NOT_FOUND {
                return Err(RepositoryError::Forbidden(
                    "Repository not found or not accessible. If this is a private repository, \
                     ensure your GitHub token has access."
                        .to_string(),
                ));
            }

            return Err(RepositoryError::ProviderError(format!(
                "GitHub API returned {status}: {body}"
            )));
        }

        let branches: Vec<GhBranch> = resp.json().await.map_err(|e| {
            RepositoryError::ProviderError(format!("Failed to parse GitHub response: {e}"))
        })?;

        let count = branches.len();
        all_branches.extend(branches);

        if count < 100 {
            break;
        }
        page += 1;
        // Safety cap to avoid infinite loops
        if page > 20 {
            warn!("GitHub branches pagination capped at 2000 for {owner}/{repo}");
            break;
        }
    }

    debug!("Fetched {} branches for {owner}/{repo}", all_branches.len());

    // Fetch repo metadata to get default_branch (best-effort)
    #[derive(serde::Deserialize)]
    struct GhRepo {
        default_branch: Option<String>,
    }

    let default_branch = match client
        .get(format!("https://api.github.com/repos/{owner}/{repo}"))
        .header("Authorization", format!("Bearer {token}"))
        .header("User-Agent", "moatless-vibe-backend")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
    {
        Ok(repo_resp) if repo_resp.status().is_success() => {
            match repo_resp.json::<GhRepo>().await {
                Ok(r) => r.default_branch,
                Err(e) => {
                    warn!("Failed to parse GitHub repo metadata for {owner}/{repo}: {e}");
                    None
                }
            }
        }
        Ok(repo_resp) => {
            let status = repo_resp.status();
            warn!("GitHub repo metadata returned {status} for {owner}/{repo}");
            None
        }
        Err(e) => {
            warn!("GitHub repo metadata request failed for {owner}/{repo}: {e}");
            None
        }
    };

    Ok(BranchFetchResult {
        branches: all_branches.into_iter().map(|b| b.name).collect(),
        default_branch,
    })
}

/// Fetch branches from GitLab API, detecting the default branch from the response.
async fn fetch_gitlab_branches(
    remote_url: &str,
    token: &str,
) -> Result<BranchFetchResult, RepositoryError> {
    // For GitLab we need the project ID (URL-encoded path)
    // https://gitlab.com/owner/repo → owner%2Frepo
    let url = remote_url.trim_end_matches('/').trim_end_matches(".git");

    let project_path = url
        .strip_prefix("https://gitlab.com/")
        .or_else(|| url.strip_prefix("http://gitlab.com/"))
        .ok_or_else(|| {
            RepositoryError::BadRequest(format!("Cannot parse GitLab URL: {remote_url}"))
        })?;

    let encoded_path = urlencoding::encode(project_path);

    let client = reqwest::Client::new();

    #[derive(serde::Deserialize)]
    struct GlBranch {
        name: String,
        default: Option<bool>,
    }

    let mut all_branches: Vec<GlBranch> = Vec::new();
    let mut page = 1u32;
    loop {
        let resp = client
            .get(format!(
                "https://gitlab.com/api/v4/projects/{encoded_path}/repository/branches?per_page=100&page={page}"
            ))
            .header("PRIVATE-TOKEN", token)
            .send()
            .await
            .map_err(|e| {
                RepositoryError::ProviderError(format!("GitLab API request failed: {e}"))
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();

            // 401/403 = access denied (token lacks permissions or repo access)
            if status == reqwest::StatusCode::UNAUTHORIZED
                || status == reqwest::StatusCode::FORBIDDEN
            {
                return Err(RepositoryError::Forbidden(
                    "Cannot access repository. Your GitLab token may lack the required permissions. \
                     Try reconnecting your GitLab account in Settings.".to_string(),
                ));
            }

            // 404 = repo not found (may also be private repo without access)
            if status == reqwest::StatusCode::NOT_FOUND {
                return Err(RepositoryError::Forbidden(
                    "Repository not found or not accessible. If this is a private repository, \
                     ensure your GitLab token has access."
                        .to_string(),
                ));
            }

            return Err(RepositoryError::ProviderError(format!(
                "GitLab API returned {status}: {body}"
            )));
        }

        let branches: Vec<GlBranch> = resp.json().await.map_err(|e| {
            RepositoryError::ProviderError(format!("Failed to parse GitLab response: {e}"))
        })?;

        let count = branches.len();
        all_branches.extend(branches);

        if count < 100 {
            break;
        }
        page += 1;
        if page > 20 {
            warn!("GitLab branches pagination capped at 2000 for {encoded_path}");
            break;
        }
    }

    let default_branch = all_branches
        .iter()
        .find(|b| b.default == Some(true))
        .map(|b| b.name.clone());

    Ok(BranchFetchResult {
        branches: all_branches.into_iter().map(|b| b.name).collect(),
        default_branch,
    })
}

/// Fetch branches from Gitness API.
///
/// Gitness API: `GET {base_url}/api/v1/repos/{repo_ref}/branches`
/// The `repo_ref` is the URL-encoded repository path (e.g. `org/repo`).
async fn fetch_gitness_branches(
    remote_url: &str,
    token: &str,
    base_url: &str,
) -> Result<BranchFetchResult, RepositoryError> {
    // Extract the repo path by stripping the base URL prefix
    let url = remote_url.trim_end_matches('/').trim_end_matches(".git");
    let base = base_url.trim_end_matches('/');

    // Try stripping the base URL to get the repo path (e.g. "org/repo")
    let repo_path = url
        .strip_prefix(&format!("{base}/"))
        .or_else(|| {
            // Also try stripping https://<host>/ patterns
            let base_without_scheme = base
                .strip_prefix("https://")
                .or_else(|| base.strip_prefix("http://"))
                .unwrap_or(base);
            let url_without_scheme = url
                .strip_prefix("https://")
                .or_else(|| url.strip_prefix("http://"))
                .unwrap_or(url);
            url_without_scheme.strip_prefix(&format!("{base_without_scheme}/"))
        })
        .ok_or_else(|| {
            RepositoryError::BadRequest(format!(
                "Cannot extract repo path from Gitness URL: {remote_url} (base: {base_url})"
            ))
        })?;

    let encoded_ref = urlencoding::encode(repo_path);

    let client = reqwest::Client::new();

    #[derive(serde::Deserialize)]
    struct GnBranch {
        name: String,
    }

    let mut all_branches: Vec<GnBranch> = Vec::new();
    let mut page = 1u32;
    loop {
        let resp = client
            .get(format!(
                "{base}/api/v1/repos/{encoded_ref}/branches?limit=100&page={page}"
            ))
            .header("Authorization", format!("Bearer {token}"))
            .send()
            .await
            .map_err(|e| {
                RepositoryError::ProviderError(format!("Gitness API request failed: {e}"))
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();

            // 401/403 = access denied (token lacks permissions or repo access)
            if status == reqwest::StatusCode::UNAUTHORIZED
                || status == reqwest::StatusCode::FORBIDDEN
            {
                return Err(RepositoryError::Forbidden(
                    "Cannot access repository. Your Gitness token may lack the required permissions. \
                     Try reconnecting your Gitness account in Settings.".to_string(),
                ));
            }

            // 404 = repo not found (may also be private repo without access)
            if status == reqwest::StatusCode::NOT_FOUND {
                return Err(RepositoryError::Forbidden(
                    "Repository not found or not accessible. If this is a private repository, \
                     ensure your Gitness token has access."
                        .to_string(),
                ));
            }

            return Err(RepositoryError::ProviderError(format!(
                "Gitness API returned {status}: {body}"
            )));
        }

        let branches: Vec<GnBranch> = resp.json().await.map_err(|e| {
            RepositoryError::ProviderError(format!("Failed to parse Gitness response: {e}"))
        })?;

        let count = branches.len();
        all_branches.extend(branches);

        if count < 100 {
            break;
        }
        page += 1;
        if page > 20 {
            warn!("Gitness branches pagination capped at 2000 for {repo_path}");
            break;
        }
    }

    // Fetch repo metadata to get default branch (best-effort)
    #[derive(serde::Deserialize)]
    struct GnRepo {
        default_branch: Option<String>,
    }

    let default_branch = match client
        .get(format!("{base}/api/v1/repos/{encoded_ref}"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
    {
        Ok(repo_resp) if repo_resp.status().is_success() => {
            match repo_resp.json::<GnRepo>().await {
                Ok(r) => r.default_branch,
                Err(e) => {
                    warn!("Failed to parse Gitness repo metadata for {repo_path}: {e}");
                    None
                }
            }
        }
        Ok(repo_resp) => {
            let status = repo_resp.status();
            warn!("Gitness repo metadata returned {status} for {repo_path}");
            None
        }
        Err(e) => {
            warn!("Gitness repo metadata request failed for {repo_path}: {e}");
            None
        }
    };

    Ok(BranchFetchResult {
        branches: all_branches.into_iter().map(|b| b.name).collect(),
        default_branch,
    })
}

/// Sync `.moatless/config.json` and `.moatless/loops/` from the remote
/// repository into the DB.
#[utoipa::path(
    post,
    path = "/api/v1/repositories/{repository_id}/sync-config",
    tag = "repositories",
    params(
        ("repository_id" = String, Path, description = "Repository identifier")
    ),
    responses(
        (status = 200, description = "Config + loops synced", body = SyncConfigResponse),
        (status = 400, description = "Bad request", body = crate::error::ErrorBody),
        (status = 404, description = "Repository not found", body = crate::error::ErrorBody)
    )
)]
pub async fn sync_config(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(repository_id): Path<String>,
) -> Result<Json<SyncConfigResponse>, RepositoryError> {
    let scope_ctx = auth.scope_context();
    let repo = state
        .repositories
        .find_by_id(&repository_id, &scope_ctx)
        .await?
        .ok_or_else(|| RepositoryError::NotFound(repository_id.clone()))?;

    let (updated, loops) = sync_repo_config(&state, &repo, &auth).await?;

    Ok(Json(SyncConfigResponse {
        repository: updated.into(),
        loops,
    }))
}

/// Reconcile a repository's committed `.moatless/loops/` + git-declared
/// workspaces into the trigger tables and `workspaces` rows.
///
/// Shared by the `sync-config` endpoint and the automatic sync on task start.
/// Run-config from `.moatless/config.json` / `.moatless/workspaces.json` flows
/// into the workspace (via `sync_repo_workspaces`), not into repo columns — a
/// repository is only a git connection. Loop sync is best-effort: a fetch
/// failure yields an empty summary rather than failing the whole reconcile.
pub(crate) async fn sync_repo_config(
    state: &AppState,
    repo: &Repository,
    auth: &AuthUser,
) -> Result<(Repository, LoopSyncSummary), RepositoryError> {
    if repo.remote_url.is_none() {
        return Err(RepositoryError::BadRequest(
            "Repository has no remote URL".to_string(),
        ));
    }

    // Reconcile `.moatless/loops/` into the trigger tables. Best-effort: a fetch
    // failure surfaces an empty summary rather than failing the whole sync.
    let loops = match sync_repo_loops(state, repo, auth).await {
        Ok(summary) => summary,
        Err(e) => {
            warn!(repository_id = %repo.id, error = %e, "Loop sync failed during config sync");
            LoopSyncSummary::default()
        }
    };

    // Reconcile git-declared workspaces (`.moatless/workspaces.json` or the
    // legacy `.moatless/config.json` → workspace run-config). This also covers
    // task start, since `maybe_sync_repo_config_on_start` calls `sync_repo_config`
    // behind the `.moatless/` existence gate. Best-effort and non-fatal.
    log_workspace_sync(state, repo, auth, "sync-config").await;

    Ok((repo.clone(), loops))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repository::dao::MockRepoRepository;

    fn repository(scope: Scope, owner_user_id: Option<&str>) -> Repository {
        Repository {
            id: "repo-1".to_string(),
            name: "repo".to_string(),
            kind: RepositoryKind::Remote,
            remote_url: Some("https://github.com/acme/repo.git".to_string()),
            default_branch: None,
            description: None,
            provider: Some(RepositoryProvider::Github),
            scope,
            owner_user_id: owner_user_id.map(str::to_string),
            created_by: Some("creator".to_string()),
            access_status: "unchecked".to_string(),
            access_checked_at: None,
            access_error: None,
            deleted: Some(false),
            created_at: None,
            updated_at: None,
        }
    }

    #[test]
    fn normalize_remote_url_strips_and_lowercases() {
        assert_eq!(
            normalize_remote_url("https://github.com/Owner/Repo.git"),
            "https://github.com/owner/repo"
        );
        assert_eq!(
            normalize_remote_url("https://github.com/Owner/Repo/"),
            "https://github.com/owner/repo"
        );
        assert_eq!(
            normalize_remote_url("  https://GitHub.com/o/r.git/  "),
            "https://github.com/o/r"
        );
        // Two URLs differing only by trailing `.git`/slash/case normalize equal.
        assert_eq!(
            normalize_remote_url("https://github.com/a/b"),
            normalize_remote_url("https://github.com/A/B.git/")
        );
    }

    #[test]
    fn resubmitting_user_scope_preserves_owner() {
        let existing = repository(Scope::User, Some("original-owner"));
        let owner_user_id = preserved_user_scope(&existing).unwrap();
        assert_eq!(owner_user_id, "original-owner");
    }

    #[test]
    fn ownerless_repository_cannot_move_to_user_scope() {
        let existing = repository(Scope::Global, None);
        let err = preserved_user_scope(&existing)
            .expect_err("ownerless repository needs an explicit future owner-picker");
        assert!(matches!(err, RepositoryError::BadRequest(_)));
    }

    #[tokio::test]
    async fn access_verification_persists_status_time_and_error() {
        use moatless_backend_core::scope::ScopeContext;
        let mut mock = MockRepoRepository::new();
        let checked_at = chrono::Utc::now();
        mock.expect_update()
            .withf(move |id, patch, _ctx| {
                id == "repo-1"
                    && patch.access_status.as_deref() == Some("forbidden")
                    && patch.access_checked_at == Some(Some(checked_at))
                    && patch
                        .access_error
                        .as_ref()
                        .and_then(|error| error.as_deref())
                        == Some("No access to repository: https://github.com/acme/repo.git")
            })
            .returning(|_, _, _| Ok(true));

        let ctx = ScopeContext {
            user_id: "owner".to_string(),
            is_admin: false,
        };
        let updated = persist_access_verification(
            &mock,
            &repository(Scope::User, Some("owner")),
            AccessStatus::Forbidden,
            checked_at,
            &ctx,
        )
        .await
        .unwrap();

        assert_eq!(updated.access_status, "forbidden");
        assert_eq!(updated.access_checked_at, Some(checked_at));
        assert_eq!(
            updated.access_error.as_deref(),
            Some("No access to repository: https://github.com/acme/repo.git")
        );
    }
}
