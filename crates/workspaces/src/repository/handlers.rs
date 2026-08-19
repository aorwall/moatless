//! Repository HTTP handlers.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;

use tracing::{debug, info, warn};

use moatless_api_schemas::workspace::WorkspaceSyncSummary;

use std::sync::Arc;

use moatless_types::repository::RepositoryProvider as GitHostProvider;
use moatless_backend_core::access_control::resolve_scope_for_create;
use moatless_types::access_control::{Scope, ScopeContext};
use moatless_core::db::DbPool;
use moatless_types::access_control::Actor;
use moatless_identity::auth::provider_token_dao::ProviderTokenDao;
use moatless_identity::extractors::AuthUser;
use moatless_identity::git_host_repo::GitHostRepo;
use moatless_identity::git_host_tokens::GitHostTokens;
use moatless_identity::secret::models::looks_like_secret_key;

use super::access::git_host_token_error;
use super::dao::{RepositoryDao, RepositoryPatch};
use super::error::RepositoryError;
use moatless_types::repository::{RepositoryKind, RepositoryProvider};
use super::models::Repository;
use super::schemas::*;
use crate::workspace::git_sync::{GitSyncService, SyncForce};

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
        (status = 409, description = "Repository already exists", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn create_repository(
    State(repositories): State<Arc<dyn RepositoryDao>>,
    State(git_sync): State<Arc<GitSyncService>>,
    auth: AuthUser,
    Json(req): Json<CreateRepositoryRequest>,
) -> Result<(StatusCode, Json<RepositoryResponse>), RepositoryError> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now();
    let user_id = auth.user_id;

    let resolved_scope =
        resolve_scope_for_create(&auth.scope_context(), req.scope)?;
    let scope_ctx = auth.scope_context();

    ensure_unique_remote_url(
        repositories.as_ref(),
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
        deleted: false,
        created_at: Some(now),
        updated_at: Some(now),
    };

    repositories
        .insert(&row)
        .await
        .map_err(|error| RepositoryError::from_write_error(error, row.remote_url.as_deref()))?;

    // A repository is only a git connection; its run-config flows to the
    // workspace via the git sync below, not into repo columns.
    let final_row = repositories
        .find_by_id(&id, &scope_ctx)
        .await?
        .unwrap_or(row);

    if final_row.remote_url.is_some() {
        // Sync git-declared workspaces (`.moatless/workspaces.json` or the legacy
        // `.moatless/config.json`). Loop sync is deliberately never automatic:
        // an administrator must invoke the explicit sync endpoint.
        log_workspace_sync(
            &git_sync,
            &final_row,
            &auth,
            "create",
            &SyncForce::RespectOverrides,
        )
        .await;
    }

    let resp = repository_response(final_row);
    Ok((StatusCode::CREATED, Json(resp)))
}

/// Run `sync_repo_workspaces` and log the outcome (`Ok=>info!`, `Err=>warn!`),
/// never blocking the caller. A failure yields an empty summary.
pub async fn log_workspace_sync(
    git_sync: &Arc<GitSyncService>,
    repo: &Repository,
    auth: &AuthUser,
    phase: &str,
    force: &SyncForce,
) -> WorkspaceSyncSummary {
    match git_sync.sync_repo_workspaces(repo, auth, force).await {
        Ok(summary) => {
            if summary.created + summary.updated + summary.reclaimed + summary.removed > 0
                || !summary.skipped.is_empty()
            {
                info!(
                    repository_id = %repo.id,
                    created = summary.created,
                    updated = summary.updated,
                    reclaimed = summary.reclaimed,
                    removed = summary.removed,
                    skipped = summary.skipped.len(),
                    phase,
                    "Synced git workspaces"
                );
            }
            summary
        }
        Err(e) => {
            warn!(repository_id = %repo.id, error = %e, phase, "Failed to sync git workspaces");
            WorkspaceSyncSummary::default()
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
    State(repositories): State<Arc<dyn RepositoryDao>>,
    auth: AuthUser,
) -> Result<Json<Vec<RepositoryResponse>>, RepositoryError> {
    let rows = repositories.list_all(&auth.scope_context()).await?;
    let repos: Vec<RepositoryResponse> = rows.into_iter().map(repository_response).collect();
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
        (status = 404, description = "Repository not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn get_repository(
    State(repositories): State<Arc<dyn RepositoryDao>>,
    auth: AuthUser,
    Path(repository_id): Path<String>,
) -> Result<Json<RepositoryResponse>, RepositoryError> {
    let row = repositories
        .find_by_id(&repository_id, &auth.scope_context())
        .await?
        .ok_or_else(|| RepositoryError::NotFound(repository_id))?;

    Ok(Json(repository_response(row)))
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
        (status = 404, description = "Repository not found", body = moatless_api_schemas::error::ErrorBody),
        (status = 409, description = "Repository URL already exists", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn update_repository(
    State(repositories): State<Arc<dyn RepositoryDao>>,
    auth: AuthUser,
    Path(repository_id): Path<String>,
    Json(req): Json<UpdateRepositoryRequest>,
) -> Result<Json<RepositoryResponse>, RepositoryError> {
    let ctx = auth.scope_context();
    let existing = repositories
        .find_by_id(&repository_id, &ctx)
        .await?
        .ok_or_else(|| RepositoryError::NotFound(repository_id.clone()))?;

    // User ownership is immutable here: re-submitting `scope: user` preserves
    // the existing owner even when an admin is editing the row.
    // Global moves require admin (enforced by resolve_scope_for_create).
    let (scope, owner_user_id) = match req.scope {
        Some(Scope::User) => {
            let existing_owner_user_id = preserved_user_scope(&existing)?;
            (Some(Scope::User), Some(Some(existing_owner_user_id)))
        }
        Some(new_scope) => {
            let resolved = resolve_scope_for_create(&auth.scope_context(), Some(new_scope))?;
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
        repositories.as_ref(),
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
        ..Default::default()
    };

    let applied = repositories
        .update(&repository_id, &patch, &ctx)
        .await
        .map_err(|error| RepositoryError::from_write_error(error, target_remote_url.as_deref()))?;
    if !applied {
        // control_filter matched no row: the caller may read this repository
        // but not control it (e.g. a global repo owned by someone else). Surface
        // NotFound so a denied write confirms no more than the read already did.
        return Err(RepositoryError::NotFound(repository_id));
    }

    // Re-fetch updated row
    let updated = repositories
        .find_by_id(&repository_id, &ctx)
        .await?
        .ok_or_else(|| RepositoryError::NotFound(repository_id))?;

    Ok(Json(repository_response(updated)))
}

async fn ensure_unique_remote_url(
    repositories: &dyn super::dao::RepositoryDao,
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
        (status = 404, description = "Repository not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn delete_repository(
    State(repositories): State<Arc<dyn RepositoryDao>>,
    State(db): State<DbPool>,
    auth: AuthUser,
    Path(repository_id): Path<String>,
) -> Result<StatusCode, RepositoryError> {
    let ctx = auth.scope_context();
    repositories
        .find_by_id(&repository_id, &ctx)
        .await?
        .ok_or_else(|| RepositoryError::NotFound(repository_id.clone()))?;

    if !repositories.soft_delete(&repository_id, &ctx).await? {
        return Err(RepositoryError::NotFound(repository_id.clone()));
    }

    // Soft-delete workspaces this repo declared in git — once the source repo is
    // gone no future sync will prune them. Best-effort; the repo is already deleted.
    if let Err(e) = delete_declared_workspaces(&db, &repository_id).await {
        warn!(repository_id = %repository_id, error = %e, "Failed to soft-delete git workspaces for deleted repo");
    }

    Ok(StatusCode::NO_CONTENT)
}

/// Soft-delete every workspace a deleted repository declared in git.
async fn delete_declared_workspaces(
    db: &moatless_core::db::DbPool,
    repository_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE workspaces SET deleted = true, updated_at = now() \
         WHERE source_repository_id = $1 AND source = 'git' \
           AND (deleted IS NULL OR deleted = false)",
    )
    .bind(repository_id)
    .execute(db)
    .await?;
    Ok(())
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
        (status = 400, description = "Cannot convert", body = moatless_api_schemas::error::ErrorBody),
        (status = 404, description = "Repository not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn convert_to_template(
    State(repositories): State<Arc<dyn RepositoryDao>>,
    auth: AuthUser,
    Path(repository_id): Path<String>,
) -> Result<Json<RepositoryResponse>, RepositoryError> {
    let ctx = auth.scope_context();
    let existing = repositories
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

    repositories.update(&repository_id, &patch, &ctx).await?;

    let updated = repositories
        .find_by_id(&repository_id, &ctx)
        .await?
        .ok_or_else(|| RepositoryError::NotFound(repository_id))?;

    Ok(Json(repository_response(updated)))
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
        (status = 400, description = "Bad request", body = moatless_api_schemas::error::ErrorBody),
        (status = 403, description = "Access denied - token lacks permissions", body = moatless_api_schemas::error::ErrorBody),
        (status = 404, description = "Repository not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn get_branches(
    State(repositories): State<Arc<dyn RepositoryDao>>,
    State(git_host_tokens): State<Arc<GitHostTokens>>,
    State(provider_tokens): State<Arc<dyn ProviderTokenDao>>,
    auth: AuthUser,
    Path(repository_id): Path<String>,
) -> Result<Json<BranchListResponse>, RepositoryError> {
    Ok(Json(
        list_branches(
            &repositories,
            &git_host_tokens,
            &provider_tokens,
            &auth.scope_context(),
            &repository_id,
        )
        .await?,
    ))
}

/// A repository's branches, read from its git host.
///
/// The HTTP `GET /branches` handler and the T3 branch picker both list a
/// repository's branches before any Sandbox exists, so the token resolution and
/// provider dispatch live here rather than in the handler. Scope is enforced by
/// the repository lookup: a `repository_id` the viewer cannot read answers
/// `NotFound`, the same as one that does not exist.
pub async fn list_branches(
    repositories: &Arc<dyn RepositoryDao>,
    git_host_tokens: &Arc<GitHostTokens>,
    provider_tokens: &Arc<dyn ProviderTokenDao>,
    ctx: &ScopeContext,
    repository_id: &str,
) -> Result<BranchListResponse, RepositoryError> {
    let repo = repositories
        .find_by_id(repository_id, ctx)
        .await?
        .ok_or_else(|| RepositoryError::NotFound(repository_id.to_string()))?;

    if repo.default_branch.is_none() {
        info!(repository_id = %repository_id, "No default branch set, fetching from provider");
    }

    let remote_url = repo
        .remote_url
        .as_deref()
        .ok_or_else(|| RepositoryError::BadRequest("Repository has no remote URL".to_string()))?;

    let provider = repo.provider.unwrap_or(RepositoryProvider::Github);

    // The viewer's own credential, on every host. A repository someone else
    // shared does not lend its owner's access to whoever opens it.
    let access_token = git_host_tokens
        .token(&Actor::caller(ctx), &git_host_repo(provider, remote_url)?)
        .await
        .map_err(git_host_token_error)?;

    // For Gitness, we also need the base URL from provider_tokens metadata
    let result = match provider {
        RepositoryProvider::Github => fetch_github_branches(remote_url, &access_token).await?,
        RepositoryProvider::Gitlab => fetch_gitlab_branches(remote_url, &access_token).await?,
        RepositoryProvider::Gitness => {
            let full_token = provider_tokens
                .find_provider_token_full(&ctx.user_id, "gitness")
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
            if let Err(e) = repositories.update(repository_id, &patch, ctx).await {
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

    Ok(BranchListResponse {
        branches: entries,
        default_branch,
    })
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

/// Name a repository on its git host, so the token seam can scope a credential
/// to it.
fn git_host_repo(
    provider: RepositoryProvider,
    remote_url: &str,
) -> Result<GitHostRepo, RepositoryError> {
    GitHostRepo::parse(provider, remote_url).ok_or_else(|| {
        RepositoryError::BadRequest(format!(
            "Cannot parse {} URL: {remote_url}",
            provider.as_str()
        ))
    })
}

/// Fetch branches from GitHub API, including the default branch from the repo endpoint.
async fn fetch_github_branches(
    remote_url: &str,
    token: &str,
) -> Result<BranchFetchResult, RepositoryError> {
    let parsed = GitHostRepo::parse(GitHostProvider::Github, remote_url).ok_or_else(|| {
        RepositoryError::BadRequest(format!("Cannot parse GitHub URL: {remote_url}"))
    })?;
    let (owner, repo) = (parsed.owner(), parsed.name());

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

#[cfg(test)]
mod tests {
    use super::*;

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
            deleted: false,
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
}
