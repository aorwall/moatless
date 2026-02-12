//! Repository HTTP handlers.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;

use tracing::{info, warn};

use crate::db::{provider_token_dao, repository_dao};
use crate::db::models::RepositoryRow;
use crate::db::repository_dao::RepositoryPatch;
use crate::extractors::AuthUser;
use crate::state::AppState;

use super::error::RepositoryError;
use super::types::*;

/// Create a new repository.
#[utoipa::path(
    post,
    path = "/api/v1/repositories",
    tag = "repositories",
    request_body = CreateRepositoryRequest,
    responses(
        (status = 201, description = "Repository created", body = RepositoryResponse),
        (status = 409, description = "Repository already exists", body = crate::sandbox::types::ErrorBody)
    )
)]
pub async fn create_repository(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<CreateRepositoryRequest>,
) -> Result<(StatusCode, Json<RepositoryResponse>), RepositoryError> {
    // Check for name uniqueness
    if let Some(_existing) =
        repository_dao::find_by_name_and_user(&state.db, &req.name, &auth.user_id).await?
    {
        return Err(RepositoryError::Conflict(format!(
            "Repository '{}' already exists",
            req.name
        )));
    }

    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now();

    let row = RepositoryRow {
        id: id.clone(),
        name: req.name,
        kind: req.kind,
        remote_url: req.remote_url,
        default_branch: req.default_branch,
        docker_image: req.docker_image,
        system_prompt: req.system_prompt,
        setup_commands: req.setup_commands,
        env_vars: req.env_vars,
        servers: req.servers,
        plugins: req.plugins,
        server_start_commands: req.server_start_commands,
        startup_timeout_seconds: req.startup_timeout_seconds,
        build_command: req.build_command,
        build_output_directory: req.build_output_directory,
        enable_inspector: req.enable_inspector,
        auto_reload: req.auto_reload,
        description: req.description,
        provider: req.provider,
        visibility: Some(req.visibility.unwrap_or_else(|| "public".to_string())),
        created_by: Some(auth.user_id),
        deleted: Some(false),
        created_at: Some(now),
        updated_at: Some(now),
    };

    repository_dao::insert(&state.db, &row).await?;

    let resp: RepositoryResponse = row.into();
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
    let rows = repository_dao::list_all(&state.db, &auth.user_id).await?;
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
        (status = 404, description = "Repository not found", body = crate::sandbox::types::ErrorBody)
    )
)]
pub async fn get_repository(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(repository_id): Path<String>,
) -> Result<Json<RepositoryResponse>, RepositoryError> {
    let row = repository_dao::find_by_id(&state.db, &repository_id, &auth.user_id)
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
        (status = 404, description = "Repository not found", body = crate::sandbox::types::ErrorBody)
    )
)]
pub async fn update_repository(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(repository_id): Path<String>,
    Json(req): Json<UpdateRepositoryRequest>,
) -> Result<Json<RepositoryResponse>, RepositoryError> {
    // Verify ownership
    let _existing = repository_dao::find_by_id(&state.db, &repository_id, &auth.user_id)
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
        servers: req.servers.map(Some),
        plugins: req.plugins.map(Some),
        env_vars: req.env_vars.map(Some),
        setup_commands: req.setup_commands.map(Some),
        server_start_commands: req.server_start_commands.map(Some),
        startup_timeout_seconds: req.startup_timeout_seconds.map(Some),
        build_command: req.build_command.map(Some),
        build_output_directory: req.build_output_directory.map(Some),
        enable_inspector: req.enable_inspector.map(Some),
        auto_reload: req.auto_reload.map(Some),
        ..Default::default()
    };

    repository_dao::update(&state.db, &repository_id, &patch).await?;

    // Re-fetch updated row
    let updated = repository_dao::find_by_id(&state.db, &repository_id, &auth.user_id)
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
        (status = 404, description = "Repository not found", body = crate::sandbox::types::ErrorBody)
    )
)]
pub async fn delete_repository(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(repository_id): Path<String>,
) -> Result<StatusCode, RepositoryError> {
    // Verify ownership
    let _existing = repository_dao::find_by_id(&state.db, &repository_id, &auth.user_id)
        .await?
        .ok_or_else(|| RepositoryError::NotFound(repository_id.clone()))?;

    repository_dao::soft_delete(&state.db, &repository_id).await?;

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
        (status = 400, description = "Cannot convert", body = crate::sandbox::types::ErrorBody),
        (status = 404, description = "Repository not found", body = crate::sandbox::types::ErrorBody)
    )
)]
pub async fn convert_to_template(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(repository_id): Path<String>,
) -> Result<Json<RepositoryResponse>, RepositoryError> {
    let existing = repository_dao::find_by_id(&state.db, &repository_id, &auth.user_id)
        .await?
        .ok_or_else(|| RepositoryError::NotFound(repository_id.clone()))?;

    if existing.kind != "remote" {
        return Err(RepositoryError::BadRequest(format!(
            "Only remote repositories can be converted to templates (current kind: {})",
            existing.kind
        )));
    }

    let patch = RepositoryPatch {
        kind: Some("template".to_string()),
        ..Default::default()
    };

    repository_dao::update(&state.db, &repository_id, &patch).await?;

    let updated = repository_dao::find_by_id(&state.db, &repository_id, &auth.user_id)
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
        (status = 400, description = "Bad request", body = crate::sandbox::types::ErrorBody),
        (status = 404, description = "Repository not found", body = crate::sandbox::types::ErrorBody)
    )
)]
pub async fn get_branches(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(repository_id): Path<String>,
) -> Result<Json<BranchListResponse>, RepositoryError> {
    let repo = repository_dao::find_by_id(&state.db, &repository_id, &auth.user_id)
        .await?
        .ok_or_else(|| RepositoryError::NotFound(repository_id.clone()))?;

    if repo.default_branch.is_none() {
        info!(repository_id = %repository_id, "No default branch set, fetching from provider");
    }

    let remote_url = repo.remote_url.as_deref().ok_or_else(|| {
        RepositoryError::BadRequest("Repository has no remote URL".to_string())
    })?;

    let provider = repo.provider.as_deref().unwrap_or("github");

    // Get user's provider token
    let token = provider_token_dao::find_provider_token(&state.db, &auth.user_id, provider)
        .await
        .map_err(|e| RepositoryError::Db(e))?
        .ok_or_else(|| {
            RepositoryError::BadRequest(format!(
                "No {provider} token configured. Please connect your {provider} account."
            ))
        })?;

    let result = match provider {
        "github" => fetch_github_branches(remote_url, &token.access_token).await?,
        "gitlab" => fetch_gitlab_branches(remote_url, &token.access_token).await?,
        _ => {
            return Err(RepositoryError::BadRequest(format!(
                "Unsupported provider: {provider}"
            )))
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
            let _ = repository_dao::update(&state.db, &repository_id, &patch).await;
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

/// Parse a GitHub remote URL into owner/repo.
fn parse_github_owner_repo(remote_url: &str) -> Option<(String, String)> {
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

    info!("Fetched {} branches for {owner}/{repo}", all_branches.len());

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
