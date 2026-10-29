use super::super::error::RepositoryError;
use crate::repository::remote::{forgejo_repo_api, require_host_repo};
use moatless_workspace_types::repository::RepositoryRemote;
use tracing::{debug, warn};

pub(super) struct BranchFetchResult {
    pub branches: Vec<String>,
    pub default_branch: Option<String>,
}

pub(super) async fn fetch_github_branches(
    client: &reqwest::Client,
    remote: &RepositoryRemote,
    token: &str,
) -> Result<BranchFetchResult, RepositoryError> {
    let parsed = require_host_repo(remote)?;
    let (owner, repo) = (parsed.owner(), parsed.name());

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

pub(super) async fn fetch_gitlab_branches(
    client: &reqwest::Client,
    remote: &RepositoryRemote,
    token: &str,
) -> Result<BranchFetchResult, RepositoryError> {
    let remote_url = remote.url.as_str();
    let parsed = require_host_repo(remote)?;
    if parsed.host() != "gitlab.com" {
        return Err(RepositoryError::BadRequest(format!(
            "Cannot fetch GitLab branches from {remote_url}"
        )));
    }
    let project_path = parsed.to_string();

    let encoded_path = urlencoding::encode(&project_path);

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

pub(super) async fn fetch_forgejo_branches(
    client: &reqwest::Client,
    remote: &RepositoryRemote,
    token: &str,
) -> Result<BranchFetchResult, RepositoryError> {
    let parsed = require_host_repo(remote)?;
    let host = parsed.host();
    let (owner, repo) = (parsed.owner(), parsed.name());
    let repo_api = forgejo_repo_api(parsed);

    #[derive(serde::Deserialize)]
    struct FjBranch {
        name: String,
    }

    let mut all_branches: Vec<FjBranch> = Vec::new();
    let mut page = 1u32;
    loop {
        let resp = client
            .get(format!("{repo_api}/branches?limit=100&page={page}"))
            .header("Authorization", format!("token {token}"))
            .send()
            .await
            .map_err(|e| {
                RepositoryError::ProviderError(format!("Forgejo API request failed: {e}"))
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();

            if status == reqwest::StatusCode::UNAUTHORIZED
                || status == reqwest::StatusCode::FORBIDDEN
            {
                return Err(RepositoryError::Forbidden(format!(
                    "Cannot access repository. Your {host} token may lack the required \
                     permissions — a personal access token needs `read:repository`. Try \
                     reconnecting the instance in Settings."
                )));
            }

            if status == reqwest::StatusCode::NOT_FOUND {
                return Err(RepositoryError::Forbidden(
                    "Repository not found or not accessible. If this is a private repository, \
                     ensure your Forgejo token has access."
                        .to_string(),
                ));
            }

            return Err(RepositoryError::ProviderError(format!(
                "Forgejo API returned {status}: {body}"
            )));
        }

        let branches: Vec<FjBranch> = resp.json().await.map_err(|e| {
            RepositoryError::ProviderError(format!("Failed to parse Forgejo response: {e}"))
        })?;

        // Stop on what the host served, not on what was asked for: Forgejo
        // clamps `limit` to its own `MAX_RESPONSE_ITEMS`, which defaults to 50,
        // so a full page under that ceiling is still short of the requested 100.
        if branches.is_empty() {
            break;
        }
        all_branches.extend(branches);

        page += 1;
        if page > 20 {
            warn!(
                "Forgejo branches pagination stopped at 20 pages ({} branches) for {owner}/{repo}",
                all_branches.len()
            );
            break;
        }
    }

    #[derive(serde::Deserialize)]
    struct FjRepo {
        default_branch: Option<String>,
    }

    let default_branch = match client
        .get(&repo_api)
        .header("Authorization", format!("token {token}"))
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => match resp.json::<FjRepo>().await {
            Ok(r) => r.default_branch,
            Err(e) => {
                warn!("Failed to parse Forgejo repo metadata for {owner}/{repo}: {e}");
                None
            }
        },
        Ok(resp) => {
            let status = resp.status();
            warn!("Forgejo repo metadata returned {status} for {owner}/{repo}");
            None
        }
        Err(e) => {
            warn!("Forgejo repo metadata request failed for {owner}/{repo}: {e}");
            None
        }
    };

    Ok(BranchFetchResult {
        branches: all_branches.into_iter().map(|b| b.name).collect(),
        default_branch,
    })
}
