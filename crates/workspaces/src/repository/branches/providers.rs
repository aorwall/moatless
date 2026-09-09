use super::super::error::RepositoryError;
use moatless_types::repository::{GitHostRepo, RepositoryProvider as GitHostProvider};
use tracing::{debug, warn};

pub(super) struct BranchFetchResult {
    pub branches: Vec<String>,
    pub default_branch: Option<String>,
}

pub(super) async fn fetch_github_branches(
    client: &reqwest::Client,
    remote_url: &str,
    token: &str,
) -> Result<BranchFetchResult, RepositoryError> {
    let parsed = GitHostRepo::parse(GitHostProvider::Github, remote_url).ok_or_else(|| {
        RepositoryError::BadRequest(format!("Cannot parse GitHub URL: {remote_url}"))
    })?;
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

/// Forgejo's REST API v1, addressed at the host `remote_url` names.
///
/// HTTPS always, whatever scheme `remote_url` carries: a Workspace controls
/// that URL, and honouring a scheme from it would let one downgrade the
/// connection `token` travels over.
pub(super) async fn fetch_forgejo_branches(
    client: &reqwest::Client,
    remote_url: &str,
    token: &str,
) -> Result<BranchFetchResult, RepositoryError> {
    let parsed = GitHostRepo::parse(GitHostProvider::Forgejo, remote_url).ok_or_else(|| {
        RepositoryError::BadRequest(format!("Cannot parse Forgejo URL: {remote_url}"))
    })?;
    let host = parsed.host().ok_or_else(|| {
        RepositoryError::BadRequest(format!("Forgejo URL names no host: {remote_url}"))
    })?;
    let (owner, repo) = (parsed.owner(), parsed.name());
    let repo_api = format!("https://{host}/api/v1/repos/{owner}/{repo}");

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

pub(super) async fn fetch_gitness_branches(
    client: &reqwest::Client,
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
