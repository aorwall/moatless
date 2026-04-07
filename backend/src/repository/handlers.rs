//! Repository HTTP handlers.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;

use tracing::{debug, info, warn};

use crate::extractors::AuthUser;
use crate::scope::Scope;
use crate::state::AppState;
use crate::task::models::Visibility;

use super::config_reader::{config_to_patch, fetch_moatless_config};
use super::dao::RepositoryPatch;
use super::error::RepositoryError;
use super::models::{Repository, RepositoryKind, RepositoryProvider};
use super::schemas::*;

/// Convert a serializable value to JSON Value, mapping errors to `RepositoryError`.
fn to_json<T: serde::Serialize>(val: T) -> Result<serde_json::Value, RepositoryError> {
    serde_json::to_value(val)
        .map_err(|e| RepositoryError::BadRequest(format!("Serialization error: {e}")))
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
    // Check for name uniqueness
    if let Some(_existing) =
        state.repositories.find_by_name_and_user(&req.name, &auth.user_id).await?
    {
        return Err(RepositoryError::Conflict(format!(
            "Repository '{}' already exists",
            req.name
        )));
    }

    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now();
    let has_servers = req.servers.is_some();
    let scope_ctx = auth.scope_context();
    let user_id = auth.user_id;

    // Determine scope: use explicit scope from request, or derive from visibility for backwards compat
    let scope = req.scope.unwrap_or(match req.visibility {
        Some(Visibility::Public) => Scope::Global,
        _ => Scope::User,
    });

    let row = Repository {
        id: id.clone(),
        name: req.name,
        kind: req.kind,
        remote_url: req.remote_url,
        default_branch: req.default_branch,
        docker_image: req.docker_image,
        system_prompt: req.system_prompt,
        setup_commands: req.setup_commands.map(to_json).transpose()?,
        env_vars: req.env_vars.map(to_json).transpose()?,
        servers: req.servers.map(to_json).transpose()?,
        plugins: req.plugins.map(to_json).transpose()?,
        server_start_commands: req.server_start_commands.map(to_json).transpose()?,
        startup_timeout_seconds: req.startup_timeout_seconds,
        build_command: req.build_command,
        build_output_directory: req.build_output_directory,
        enable_inspector: req.enable_inspector,
        auto_reload: req.auto_reload,
        resources: req.resources.map(to_json).transpose()?,
        description: req.description,
        provider: req.provider,
        visibility: Some(req.visibility.unwrap_or(Visibility::Public)),
        scope,
        team_id: req.team_id,
        created_by: Some(user_id.clone()),
        deleted: Some(false),
        created_at: Some(now),
        updated_at: Some(now),
    };

    state.repositories.insert(&row).await?;

    // Auto-read .moatless/config.json if no servers were provided and repo has a remote URL
    if !has_servers {
        if let Some(ref remote_url) = row.remote_url {
            let provider = row.provider.unwrap_or(RepositoryProvider::Github);
            let provider_str = provider.as_str();

            // Try to get a provider token (optional for GitHub public repos)
            let token = match state
                .provider_tokens
                .find_provider_token(&user_id, provider_str)
                .await
            {
                Ok(t) => t,
                Err(e) => {
                    warn!(repository_id = %id, error = %e, "Failed to look up provider token");
                    None
                }
            };

            let token_str = token.as_ref().map(|t| t.access_token.as_str());
            let branch = row.default_branch.as_deref();
            match fetch_moatless_config(remote_url, branch, token_str, provider).await {
                Ok(Some(config)) => {
                    info!(repository_id = %id, "Auto-applying .moatless/config.json");
                    match config_to_patch(&config) {
                        Ok(patch) => {
                            if let Err(e) = state.repositories.update(&id, &patch).await {
                                warn!(repository_id = %id, error = %e, "Failed to apply repo config");
                            }
                        }
                        Err(e) => {
                            warn!(repository_id = %id, error = %e, "Failed to convert repo config to patch");
                        }
                    }
                }
                Ok(None) => {
                    debug!(repository_id = %id, "No .moatless/config.json found");
                }
                Err(e) => {
                    warn!(repository_id = %id, error = %e, "Failed to fetch .moatless/config.json");
                }
            }
        }
    }

    // Re-fetch to include any config that was applied
    let final_row = state
        .repositories
        .find_by_id(&id, &scope_ctx)
        .await?
        .unwrap_or(row);

    let resp: RepositoryResponse = final_row.into();
    Ok((StatusCode::CREATED, Json(resp)))
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
    let row = state.repositories.find_by_id(&repository_id, &auth.scope_context())
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
        (status = 404, description = "Repository not found", body = crate::error::ErrorBody)
    )
)]
pub async fn update_repository(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(repository_id): Path<String>,
    Json(req): Json<UpdateRepositoryRequest>,
) -> Result<Json<RepositoryResponse>, RepositoryError> {
    // Verify ownership
    let _existing = state.repositories.find_by_id(&repository_id, &auth.scope_context())
        .await?
        .ok_or_else(|| RepositoryError::NotFound(repository_id.clone()))?;

    let patch = RepositoryPatch {
        name: req.name,
        remote_url: req.remote_url.map(Some),
        default_branch: req.default_branch.map(Some),
        docker_image: req.docker_image.map(Some),
        system_prompt: req.system_prompt.map(Some),
        description: req.description.map(Some),
        visibility: req.visibility,
        servers: req.servers.map(to_json).transpose()?.map(Some),
        plugins: req.plugins.map(to_json).transpose()?.map(Some),
        env_vars: req.env_vars.map(to_json).transpose()?.map(Some),
        setup_commands: req.setup_commands.map(to_json).transpose()?.map(Some),
        server_start_commands: req.server_start_commands.map(to_json).transpose()?.map(Some),
        startup_timeout_seconds: req.startup_timeout_seconds.map(Some),
        build_command: req.build_command.map(Some),
        build_output_directory: req.build_output_directory.map(Some),
        enable_inspector: req.enable_inspector.map(Some),
        auto_reload: req.auto_reload.map(Some),
        resources: req.resources.map(to_json).transpose()?.map(Some),
        ..Default::default()
    };

    state.repositories.update(&repository_id, &patch).await?;

    // Re-fetch updated row
    let updated = state.repositories.find_by_id(&repository_id, &auth.scope_context())
        .await?
        .ok_or_else(|| RepositoryError::NotFound(repository_id))?;

    Ok(Json(updated.into()))
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
    // Verify ownership
    let _existing = state.repositories.find_by_id(&repository_id, &auth.scope_context())
        .await?
        .ok_or_else(|| RepositoryError::NotFound(repository_id.clone()))?;

    state.repositories.soft_delete(&repository_id).await?;

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
    let existing = state.repositories.find_by_id(&repository_id, &auth.scope_context())
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

    state.repositories.update(&repository_id, &patch).await?;

    let updated = state.repositories.find_by_id(&repository_id, &auth.scope_context())
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
        (status = 404, description = "Repository not found", body = crate::error::ErrorBody)
    )
)]
pub async fn get_branches(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(repository_id): Path<String>,
) -> Result<Json<BranchListResponse>, RepositoryError> {
    let repo = state.repositories.find_by_id(&repository_id, &auth.scope_context())
        .await?
        .ok_or_else(|| RepositoryError::NotFound(repository_id.clone()))?;

    if repo.default_branch.is_none() {
        info!(repository_id = %repository_id, "No default branch set, fetching from provider");
    }

    let remote_url = repo.remote_url.as_deref().ok_or_else(|| {
        RepositoryError::BadRequest("Repository has no remote URL".to_string())
    })?;

    let provider = repo.provider.unwrap_or(RepositoryProvider::Github);
    let provider_str = provider.as_str();

    // Get user's provider token (refresh expired GitHub App tokens)
    let access_token = if provider == RepositoryProvider::Github {
        resolve_github_token(&state, &auth.user_id).await?
    } else {
        state.provider_tokens.find_provider_token(&auth.user_id, provider_str)
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
            let full_token = state.provider_tokens.find_provider_token_full(&auth.user_id, "gitness")
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
            if let Err(e) = state.repositories.update(&repository_id, &patch).await {
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
// Token helpers
// =============================================================================

/// Resolve a fresh GitHub token for the user, refreshing expired GitHub App tokens.
///
/// Falls back to the raw stored token when no GitHub App service is configured.
async fn resolve_github_token(state: &AppState, user_id: &str) -> Result<String, RepositoryError> {
    if let Some(ref app_svc) = state.github_app_service {
        crate::auth::token_refresh::ensure_fresh_token(
            state.provider_tokens.as_ref(),
            state.provider_token_auth.as_ref(),
            app_svc,
            user_id,
        )
        .await
        .map_err(|e| RepositoryError::ProviderError(format!("GitHub token refresh failed: {e}")))
    } else {
        state
            .provider_tokens
            .find_provider_token(user_id, "github")
            .await
            .map_err(RepositoryError::Db)?
            .map(|t| t.access_token)
            .ok_or_else(|| {
                RepositoryError::BadRequest(
                    "No github token configured. Please connect your github account.".into(),
                )
            })
    }
}

// =============================================================================
// Provider branch helpers
// =============================================================================

/// Result from fetching branches from a provider, including the detected default branch.
struct BranchFetchResult {
    branches: Vec<String>,
    default_branch: Option<String>,
}

/// Parse a GitHub remote URL into owner/repo.
pub(crate) fn parse_github_owner_repo(remote_url: &str) -> Option<(String, String)> {
    // Handles: https://github.com/owner/repo.git or https://github.com/owner/repo
    let url = remote_url
        .trim_end_matches('/')
        .trim_end_matches(".git");
    let parts: Vec<&str> = url.split('/').collect();
    if parts.len() >= 2 {
        let repo = parts[parts.len() - 1].to_string();
        let owner = parts[parts.len() - 2].to_string();
        Some((owner, repo))
    } else {
        None
    }
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
    let url = remote_url
        .trim_end_matches('/')
        .trim_end_matches(".git");

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
    let url = remote_url
        .trim_end_matches('/')
        .trim_end_matches(".git");
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

/// Sync `.moatless/config.json` from the remote repository into the DB.
#[utoipa::path(
    post,
    path = "/api/v1/repositories/{repository_id}/sync-config",
    tag = "repositories",
    params(
        ("repository_id" = String, Path, description = "Repository identifier")
    ),
    responses(
        (status = 200, description = "Config synced", body = RepositoryResponse),
        (status = 400, description = "Bad request", body = crate::error::ErrorBody),
        (status = 404, description = "Repository not found", body = crate::error::ErrorBody)
    )
)]
pub async fn sync_config(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(repository_id): Path<String>,
) -> Result<Json<RepositoryResponse>, RepositoryError> {
    let scope_ctx = auth.scope_context();
    let repo = state
        .repositories
        .find_by_id(&repository_id, &scope_ctx)
        .await?
        .ok_or_else(|| RepositoryError::NotFound(repository_id.clone()))?;

    let remote_url = repo.remote_url.as_deref().ok_or_else(|| {
        RepositoryError::BadRequest("Repository has no remote URL".to_string())
    })?;

    let provider = repo.provider.unwrap_or(RepositoryProvider::Github);
    let provider_str = provider.as_str();

    let token = state
        .provider_tokens
        .find_provider_token(&auth.user_id, provider_str)
        .await
        .map_err(RepositoryError::Db)?;

    let token_str = token.as_ref().map(|t| t.access_token.as_str());
    let branch = repo.default_branch.as_deref();
    let config =
        fetch_moatless_config(remote_url, branch, token_str, provider).await?;

    let config = config.ok_or_else(|| {
        RepositoryError::NotFound(
            "No .moatless/config.json found in the repository".to_string(),
        )
    })?;

    let patch = config_to_patch(&config)?;
    state.repositories.update(&repository_id, &patch).await?;

    let updated = state
        .repositories
        .find_by_id(&repository_id, &scope_ctx)
        .await?
        .ok_or_else(|| RepositoryError::NotFound(repository_id))?;

    Ok(Json(updated.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_github_https_with_git_suffix() {
        let (owner, repo) = parse_github_owner_repo("https://github.com/owner/repo.git").unwrap();
        assert_eq!(owner, "owner");
        assert_eq!(repo, "repo");
    }

    #[test]
    fn parse_github_https_without_git_suffix() {
        let (owner, repo) = parse_github_owner_repo("https://github.com/my-org/my-repo").unwrap();
        assert_eq!(owner, "my-org");
        assert_eq!(repo, "my-repo");
    }

    #[test]
    fn parse_github_trailing_slash() {
        let (owner, repo) = parse_github_owner_repo("https://github.com/owner/repo/").unwrap();
        assert_eq!(owner, "owner");
        assert_eq!(repo, "repo");
    }

    #[test]
    fn parse_github_too_few_parts() {
        // A single segment after splitting by '/' won't have 2 parts
        let result = parse_github_owner_repo("x");
        assert!(result.is_none());
    }

    #[test]
    fn parse_gitlab_url_still_extracts() {
        // parse_github_owner_repo doesn't validate host — it extracts last 2 path segments
        let (owner, repo) =
            parse_github_owner_repo("https://gitlab.com/group/project.git").unwrap();
        assert_eq!(owner, "group");
        assert_eq!(repo, "project");
    }
}
