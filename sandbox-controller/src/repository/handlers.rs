//! Repository HTTP handlers.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;

use crate::db::{provider_token_dao, repository_dao};
use crate::db::models::RepositoryRow;
use crate::db::repository_dao::RepositoryPatch;
use crate::extractors::AuthUser;
use crate::state::AppState;

use super::error::RepositoryError;
use super::types::*;

/// POST /api/v1/repositories
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

/// GET /api/v1/repositories
pub async fn list_repositories(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Vec<RepositoryResponse>>, RepositoryError> {
    let rows = repository_dao::list_all(&state.db, &auth.user_id).await?;
    let repos: Vec<RepositoryResponse> = rows.into_iter().map(Into::into).collect();
    Ok(Json(repos))
}

/// GET /api/v1/repositories/{repository_id}
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

/// PUT /api/v1/repositories/{repository_id}
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

/// DELETE /api/v1/repositories/{repository_id}
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

/// POST /api/v1/repositories/{repository_id}/convert-to-template
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

/// GET /api/v1/repositories/{repository_id}/branches
pub async fn get_branches(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(repository_id): Path<String>,
) -> Result<Json<BranchListResponse>, RepositoryError> {
    let repo = repository_dao::find_by_id(&state.db, &repository_id, &auth.user_id)
        .await?
        .ok_or_else(|| RepositoryError::NotFound(repository_id.clone()))?;

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

    let branches = match provider {
        "github" => fetch_github_branches(remote_url, &token.access_token).await?,
        "gitlab" => fetch_gitlab_branches(remote_url, &token.access_token).await?,
        _ => {
            return Err(RepositoryError::BadRequest(format!(
                "Unsupported provider: {provider}"
            )))
        }
    };

    let default_branch = repo.default_branch.clone();

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

/// Fetch branches from GitHub API.
async fn fetch_github_branches(
    remote_url: &str,
    token: &str,
) -> Result<Vec<String>, RepositoryError> {
    let (owner, repo) = parse_github_owner_repo(remote_url).ok_or_else(|| {
        RepositoryError::BadRequest(format!("Cannot parse GitHub URL: {remote_url}"))
    })?;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!(
            "https://api.github.com/repos/{owner}/{repo}/branches?per_page=100"
        ))
        .header("Authorization", format!("Bearer {token}"))
        .header("User-Agent", "moatless-sandbox-controller")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| RepositoryError::ProviderError(format!("GitHub API request failed: {e}")))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(RepositoryError::ProviderError(format!(
            "GitHub API returned {status}: {body}"
        )));
    }

    #[derive(serde::Deserialize)]
    struct GhBranch {
        name: String,
    }

    let branches: Vec<GhBranch> = resp
        .json()
        .await
        .map_err(|e| RepositoryError::ProviderError(format!("Failed to parse GitHub response: {e}")))?;

    Ok(branches.into_iter().map(|b| b.name).collect())
}

/// Fetch branches from GitLab API.
async fn fetch_gitlab_branches(
    remote_url: &str,
    token: &str,
) -> Result<Vec<String>, RepositoryError> {
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
    let resp = client
        .get(format!(
            "https://gitlab.com/api/v4/projects/{encoded_path}/repository/branches?per_page=100"
        ))
        .header("PRIVATE-TOKEN", token)
        .send()
        .await
        .map_err(|e| RepositoryError::ProviderError(format!("GitLab API request failed: {e}")))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(RepositoryError::ProviderError(format!(
            "GitLab API returned {status}: {body}"
        )));
    }

    #[derive(serde::Deserialize)]
    struct GlBranch {
        name: String,
    }

    let branches: Vec<GlBranch> = resp
        .json()
        .await
        .map_err(|e| RepositoryError::ProviderError(format!("Failed to parse GitLab response: {e}")))?;

    Ok(branches.into_iter().map(|b| b.name).collect())
}
