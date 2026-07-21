//! Reads `.moatless/config.json` from remote repositories via provider APIs.

use moatless_api_schemas::repository::{
    DOCKER_STORAGE_MODES, STORAGE_MODES, is_valid_docker_storage_mode, is_valid_storage_mode,
};
use serde::Deserialize;
use tracing::{debug, warn};

use crate::sandbox::models::ServerConfig;

use super::dao::RepositoryPatch;
use super::error::RepositoryError;
use super::handlers::parse_github_owner_repo;
use super::models::{RepositoryProvider, ResourceConfig};

/// Top-level `.moatless/config.json` schema.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoatlessRepoConfig {
    pub docker_image: Option<String>,
    pub code_execution: Option<CodeExecution>,
    pub servers: Option<Vec<ServerConfig>>,
    pub system_prompt: Option<String>,
    pub resources: Option<ResourceConfig>,
    /// Per-repo Kubernetes ServiceAccount for this repo's sandboxes. Overrides the
    /// global default. Security: the named SA must exist and be RBAC-bound in the
    /// sandbox namespace — this field only selects it, it grants nothing on its own.
    pub sandbox_service_account: Option<String>,
    /// Per-repo network profile — pods carry `moatless.io/network-profile=<value>`
    /// for a static NetworkPolicy to select. Selects a policy; grants nothing.
    pub network_profile: Option<String>,
}

/// Code execution settings.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeExecution {
    pub setup_commands: Option<Vec<String>>,
}

/// Top-level `.moatless/workspaces.json` schema — the default (non-legacy) form,
/// a list of workspaces the repo declares, each with its own run-config and
/// explicit repository URL(s).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoatlessWorkspacesFile {
    #[allow(dead_code)] // reserved for forward-compat; not yet branched on
    pub version: Option<u32>,
    pub workspaces: Vec<MoatlessWorkspaceEntry>,
}

/// One workspace entry in `.moatless/workspaces.json`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoatlessWorkspaceEntry {
    /// Stable natural key within the repo. When present, renaming `name` keeps
    /// the workspace identity; when absent, the slugified `name` is the key.
    pub key: Option<String>,
    pub name: String,
    pub description: Option<String>,
    pub repos: Vec<MoatlessWorkspaceRepoRef>,
    // --- Run configuration (mirrors `.moatless/config.json`) ---
    pub docker_image: Option<String>,
    pub docker_access: Option<bool>,
    pub code_execution: Option<CodeExecution>,
    pub servers: Option<Vec<ServerConfig>>,
    pub system_prompt: Option<String>,
    pub resources: Option<ResourceConfig>,
    pub env_vars: Option<std::collections::HashMap<String, String>>,
    pub plugins: Option<Vec<String>>,
    pub sandbox_service_account: Option<String>,
    pub network_profile: Option<String>,
}

/// A repository reference inside a `workspaces.json` entry.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoatlessWorkspaceRepoRef {
    pub url: String,
    pub branch: Option<String>,
    pub mount_name: Option<String>,
    #[serde(default)]
    pub primary: bool,
}

/// Provider reachability of a repository URL. Stored on the `repositories` row
/// as `access_status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessStatus {
    Unchecked,
    Ok,
    NotFound,
    Forbidden,
    Error,
}

impl AccessStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            AccessStatus::Unchecked => "unchecked",
            AccessStatus::Ok => "ok",
            AccessStatus::NotFound => "not_found",
            AccessStatus::Forbidden => "forbidden",
            AccessStatus::Error => "error",
        }
    }
}

/// Fetch `.moatless/config.json` from a remote repository.
///
/// Returns `Ok(None)` when the file does not exist (HTTP 404).
/// Token is optional — GitHub public repos can be fetched without auth.
pub async fn fetch_moatless_config(
    remote_url: &str,
    branch: Option<&str>,
    token: Option<&str>,
    provider: RepositoryProvider,
) -> Result<Option<MoatlessRepoConfig>, RepositoryError> {
    let Some(body) =
        fetch_provider_file(remote_url, ".moatless/config.json", branch, token, provider).await?
    else {
        return Ok(None);
    };
    let config = serde_json::from_str(&body).map_err(|e| {
        RepositoryError::ProviderError(format!("Failed to parse .moatless/config.json: {e}"))
    })?;
    Ok(Some(config))
}

/// Fetch and parse `.moatless/workspaces.json` from a remote repository.
///
/// Returns `Ok(None)` when the file does not exist. This is the default
/// (non-legacy) workspace declaration form; `.moatless/config.json` remains the
/// legacy single-workspace form.
pub async fn fetch_moatless_workspaces(
    remote_url: &str,
    branch: Option<&str>,
    token: Option<&str>,
    provider: RepositoryProvider,
) -> Result<Option<MoatlessWorkspacesFile>, RepositoryError> {
    let Some(body) =
        fetch_provider_file(remote_url, ".moatless/workspaces.json", branch, token, provider)
            .await?
    else {
        return Ok(None);
    };
    let file = serde_json::from_str(&body).map_err(|e| {
        RepositoryError::ProviderError(format!("Failed to parse .moatless/workspaces.json: {e}"))
    })?;
    Ok(Some(file))
}

/// Fetch a repository file's raw contents via the provider API.
///
/// Returns `Ok(None)` on a 404 (file absent). `path` is a repo-relative path
/// such as `.moatless/config.json`. Gitness is not wired (needs a base_url).
async fn fetch_provider_file(
    remote_url: &str,
    path: &str,
    branch: Option<&str>,
    token: Option<&str>,
    provider: RepositoryProvider,
) -> Result<Option<String>, RepositoryError> {
    match provider {
        RepositoryProvider::Github => fetch_github_file(remote_url, path, branch, token).await,
        RepositoryProvider::Gitlab => {
            let token = token.ok_or_else(|| {
                RepositoryError::BadRequest(
                    "GitLab requires an access token to fetch config".to_string(),
                )
            })?;
            fetch_gitlab_file(remote_url, path, branch, token).await
        }
        RepositoryProvider::Gitness => {
            // Gitness needs a base_url; skip for now – callers can pass it later.
            Ok(None)
        }
    }
}

/// Cheap best-effort check for whether a repo ships a `.moatless/` directory.
///
/// Used to gate the automatic config sync on task start so we don't hit the
/// provider API for repos that don't use moatless config at all. Fails open:
/// on a definitive 404 it returns `false`, but on any other error (transport,
/// rate limit, unsupported provider) it returns `true` so the caller still
/// attempts the sync rather than silently skipping a repo that does have config.
pub async fn moatless_dir_exists(
    remote_url: &str,
    branch: Option<&str>,
    token: Option<&str>,
    provider: RepositoryProvider,
) -> bool {
    match provider {
        RepositoryProvider::Github => github_moatless_dir_exists(remote_url, branch, token).await,
        // Non-GitHub providers have no cheap check wired yet — don't skip them.
        _ => true,
    }
}

/// GitHub existence probe: `GET /repos/{owner}/{repo}/contents/.moatless?ref={branch}`.
/// Only a 404 is treated as "no directory"; everything else fails open to `true`.
async fn github_moatless_dir_exists(
    remote_url: &str,
    branch: Option<&str>,
    token: Option<&str>,
) -> bool {
    let Some((owner, repo)) = parse_github_owner_repo(remote_url) else {
        return true;
    };

    let mut url = format!("https://api.github.com/repos/{owner}/{repo}/contents/.moatless");
    if let Some(r) = branch {
        use std::fmt::Write;
        let _ = write!(url, "?ref={r}");
    }

    let client = reqwest::Client::new();
    let mut req = client
        .get(&url)
        .header("User-Agent", "moatless-vibe-backend")
        .header("Accept", "application/vnd.github+json");
    if let Some(token) = token {
        req = req.header("Authorization", format!("Bearer {token}"));
    }

    match req.send().await {
        Ok(resp) if resp.status() == reqwest::StatusCode::NOT_FOUND => false,
        Ok(_) => true,
        Err(e) => {
            warn!(error = %e, "Failed to probe .moatless directory; assuming present");
            true
        }
    }
}

/// Fetch a file from GitHub: `GET /repos/{owner}/{repo}/contents/{path}?ref={branch}`.
/// Returns the raw file body, or `Ok(None)` on 404.
async fn fetch_github_file(
    remote_url: &str,
    path: &str,
    branch: Option<&str>,
    token: Option<&str>,
) -> Result<Option<String>, RepositoryError> {
    let (owner, repo) = parse_github_owner_repo(remote_url).ok_or_else(|| {
        RepositoryError::BadRequest(format!("Cannot parse GitHub URL: {remote_url}"))
    })?;

    let mut url = format!("https://api.github.com/repos/{owner}/{repo}/contents/{path}");
    if let Some(r) = branch {
        use std::fmt::Write;
        let _ = write!(url, "?ref={r}");
    }

    let client = reqwest::Client::new();
    let mut req = client
        .get(&url)
        .header("User-Agent", "moatless-vibe-backend")
        .header("Accept", "application/vnd.github.raw+json");
    if let Some(token) = token {
        req = req.header("Authorization", format!("Bearer {token}"));
    }
    let resp = req
        .send()
        .await
        .map_err(|e| RepositoryError::ProviderError(format!("GitHub API request failed: {e}")))?;

    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        debug!("No {path} found in {owner}/{repo}");
        return Ok(None);
    }

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(RepositoryError::ProviderError(format!(
            "GitHub API returned {status} fetching {path}: {body}"
        )));
    }

    let body = resp.text().await.map_err(|e| {
        RepositoryError::ProviderError(format!("Failed to read {path} body: {e}"))
    })?;
    Ok(Some(body))
}

/// Fetch a file from GitLab:
/// `GET /api/v4/projects/{id}/repository/files/{encoded_path}/raw?ref={branch}`.
/// Returns the raw file body, or `Ok(None)` on 404.
async fn fetch_gitlab_file(
    remote_url: &str,
    path: &str,
    branch: Option<&str>,
    token: &str,
) -> Result<Option<String>, RepositoryError> {
    let project_path = gitlab_project_path(remote_url)?;

    let encoded_path = urlencoding::encode(&project_path);
    let encoded_file = urlencoding::encode(path);
    let ref_param = branch.unwrap_or("HEAD");

    let api_url = format!(
        "https://gitlab.com/api/v4/projects/{encoded_path}/repository/files/{encoded_file}/raw?ref={ref_param}"
    );

    let client = reqwest::Client::new();
    let resp = client
        .get(&api_url)
        .header("PRIVATE-TOKEN", token)
        .send()
        .await
        .map_err(|e| RepositoryError::ProviderError(format!("GitLab API request failed: {e}")))?;

    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        debug!("No {path} found in {project_path}");
        return Ok(None);
    }

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(RepositoryError::ProviderError(format!(
            "GitLab API returned {status} fetching {path}: {body}"
        )));
    }

    let body = resp.text().await.map_err(|e| {
        RepositoryError::ProviderError(format!("Failed to read {path} body: {e}"))
    })?;
    Ok(Some(body))
}

/// Parse the `owner/group/project` path out of a gitlab.com URL.
fn gitlab_project_path(remote_url: &str) -> Result<String, RepositoryError> {
    let url_trimmed = remote_url.trim_end_matches('/').trim_end_matches(".git");
    url_trimmed
        .strip_prefix("https://gitlab.com/")
        .or_else(|| url_trimmed.strip_prefix("http://gitlab.com/"))
        .map(str::to_string)
        .ok_or_else(|| RepositoryError::BadRequest(format!("Cannot parse GitLab URL: {remote_url}")))
}

/// Verify a repository URL is reachable via the provider API, mapping the
/// response to an [`AccessStatus`]: 200→`ok`, 404→`not_found`,
/// 401/403→`forbidden`, anything else (incl. transport errors)→`error`.
///
/// Providers without a wired check (Gitness) and unparseable URLs return
/// `Unchecked`, leaving the repo's default `unchecked` state untouched.
pub async fn verify_repository_access(
    remote_url: &str,
    token: Option<&str>,
    provider: RepositoryProvider,
) -> AccessStatus {
    let url = match provider {
        RepositoryProvider::Github => {
            let Some((owner, repo)) = parse_github_owner_repo(remote_url) else {
                return AccessStatus::Unchecked;
            };
            format!("https://api.github.com/repos/{owner}/{repo}")
        }
        RepositoryProvider::Gitlab => match gitlab_project_path(remote_url) {
            Ok(path) => format!(
                "https://gitlab.com/api/v4/projects/{}",
                urlencoding::encode(&path)
            ),
            Err(_) => return AccessStatus::Unchecked,
        },
        RepositoryProvider::Gitness => return AccessStatus::Unchecked,
    };

    let client = reqwest::Client::new();
    let mut req = client.get(&url).header("User-Agent", "moatless-vibe-backend");
    req = match provider {
        RepositoryProvider::Github => {
            let mut r = req.header("Accept", "application/vnd.github+json");
            if let Some(token) = token {
                r = r.header("Authorization", format!("Bearer {token}"));
            }
            r
        }
        RepositoryProvider::Gitlab => match token {
            Some(token) => req.header("PRIVATE-TOKEN", token),
            None => req,
        },
        RepositoryProvider::Gitness => req,
    };

    match req.send().await {
        Ok(resp) => map_status_to_access(resp.status()),
        Err(e) => {
            warn!(error = %e, %url, "verify_repository_access request failed");
            AccessStatus::Error
        }
    }
}

/// Map an HTTP status to an [`AccessStatus`].
fn map_status_to_access(status: reqwest::StatusCode) -> AccessStatus {
    use reqwest::StatusCode;
    if status.is_success() {
        AccessStatus::Ok
    } else if status == StatusCode::NOT_FOUND {
        AccessStatus::NotFound
    } else if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        AccessStatus::Forbidden
    } else {
        AccessStatus::Error
    }
}

/// Convert a [`MoatlessRepoConfig`] into a [`RepositoryPatch`].
///
/// Only sets fields that are present in the config.
pub fn config_to_patch(config: &MoatlessRepoConfig) -> Result<RepositoryPatch, RepositoryError> {
    let servers = config
        .servers
        .as_ref()
        .map(serde_json::to_value)
        .transpose()
        .map_err(|e| RepositoryError::BadRequest(format!("Serialization error: {e}")))?
        .map(Some);

    let setup_commands = config
        .code_execution
        .as_ref()
        .and_then(|ce| ce.setup_commands.as_ref())
        .map(serde_json::to_value)
        .transpose()
        .map_err(|e| RepositoryError::BadRequest(format!("Serialization error: {e}")))?
        .map(Some);

    let system_prompt = config.system_prompt.as_ref().map(|s| Some(s.clone()));

    // A committed `.moatless/config.json` can set the storage tier. An
    // unrecognized value isn't fatal — provisioning falls back to the EBS tier —
    // but warn so a typo (e.g. "s33") doesn't silently strand a repo on EBS.
    if let Some(mode) = config
        .resources
        .as_ref()
        .and_then(|r| r.storage_mode.as_deref())
    {
        if !is_valid_storage_mode(mode) {
            warn!(
                storage_mode = %mode,
                valid = %STORAGE_MODES.join(", "),
                "Unrecognized resources.storageMode in .moatless/config.json; provisioning will fall back to the ebs tier"
            );
        }
    }
    if let Some(mode) = config
        .resources
        .as_ref()
        .and_then(|r| r.docker_storage_mode.as_deref())
    {
        if !is_valid_docker_storage_mode(mode) {
            warn!(
                docker_storage_mode = %mode,
                valid = %DOCKER_STORAGE_MODES.join(", "),
                "Unrecognized resources.dockerStorageMode in .moatless/config.json; dind storage will fall back to node"
            );
        }
    }

    let resources = config
        .resources
        .as_ref()
        .map(serde_json::to_value)
        .transpose()
        .map_err(|e| RepositoryError::BadRequest(format!("Serialization error: {e}")))?
        .map(Some);

    let docker_image = config.docker_image.as_ref().map(|s| Some(s.clone()));

    let sandbox_service_account = config
        .sandbox_service_account
        .as_ref()
        .map(|s| Some(s.clone()));

    let network_profile = config.network_profile.as_ref().map(|s| Some(s.clone()));

    Ok(RepositoryPatch {
        docker_image,
        sandbox_service_account,
        network_profile,
        system_prompt,
        servers,
        setup_commands,
        resources,
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn moatless_dir_exists_fails_open_for_non_github() {
        // Non-GitHub providers have no cheap probe wired — the gate must not
        // skip them, so the check reports the directory as present.
        assert!(
            moatless_dir_exists(
                "https://gitness.example.com/x/y.git",
                None,
                None,
                RepositoryProvider::Gitness
            )
            .await
        );
    }

    #[test]
    fn parse_config_json() {
        let json = r#"{
            "codeExecution": {
                "setupCommands": ["bun install"]
            },
            "servers": [
                {
                    "name": "dev",
                    "label": "Dev Server",
                    "port": 3000,
                    "startCommands": ["bun run dev"],
                    "startupTimeoutSeconds": 30,
                    "autoReload": true,
                    "enableInspector": true,
                    "default": true
                }
            ]
        }"#;

        let config: MoatlessRepoConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.servers.as_ref().unwrap().len(), 1);
        assert_eq!(config.servers.as_ref().unwrap()[0].name, "dev");
        assert_eq!(
            config
                .code_execution
                .as_ref()
                .unwrap()
                .setup_commands
                .as_ref()
                .unwrap(),
            &vec!["bun install".to_string()]
        );
    }

    #[test]
    fn config_to_patch_maps_fields() {
        let config = MoatlessRepoConfig {
            docker_image: Some("myregistry/custom-sandbox:latest".to_string()),
            code_execution: Some(CodeExecution {
                setup_commands: Some(vec!["npm install".to_string()]),
            }),
            servers: Some(vec![ServerConfig {
                name: "dev".to_string(),
                label: Some("Dev".to_string()),
                port: 3000,
                start_commands: Some(vec!["npm run dev".to_string()]),
                startup_timeout_seconds: Some(30),
                auto_reload: true,
                enable_inspector: false,
                default: true,
                ..Default::default()
            }]),
            system_prompt: Some("Be helpful".to_string()),
            resources: None,
            sandbox_service_account: Some("moatless-dev".to_string()),
            network_profile: None,
        };

        let patch = config_to_patch(&config).unwrap();
        assert_eq!(
            patch.sandbox_service_account,
            Some(Some("moatless-dev".to_string()))
        );
        assert_eq!(
            patch.docker_image,
            Some(Some("myregistry/custom-sandbox:latest".to_string()))
        );
        assert!(patch.servers.is_some());
        assert!(patch.setup_commands.is_some());
        assert_eq!(patch.system_prompt, Some(Some("Be helpful".to_string())));
        // Fields not set should remain None
        assert!(patch.name.is_none());
        assert!(patch.remote_url.is_none());
    }

    #[test]
    fn config_carries_storage_mode_from_git_repo_to_patch() {
        // A repo selecting the ephemeral storage tier via its committed
        // `.moatless/config.json`. `MoatlessRepoConfig` is camelCase, and the
        // nested `ResourceConfig` must read the camelCase keys too — otherwise
        // `storageMode` silently drops and the repo stays on the EBS tier.
        let json = r#"{
            "resources": { "storageMode": "ephemeral", "storage": "8Gi" }
        }"#;
        let config: MoatlessRepoConfig = serde_json::from_str(json).unwrap();

        let resources = config.resources.as_ref().expect("resources present");
        assert_eq!(resources.storage_mode.as_deref(), Some("ephemeral"));
        assert_eq!(resources.storage.as_deref(), Some("8Gi"));

        // config_to_patch serializes resources into the JSONB blob stored on the
        // repository row. It must round-trip back to camelCase so the
        // provision-time deserialize (`build_repo_config`) reads it correctly.
        let patch = config_to_patch(&config).unwrap();
        let value = patch
            .resources
            .expect("resources field set")
            .expect("resources value present");
        assert_eq!(
            value.get("storageMode"),
            Some(&serde_json::json!("ephemeral"))
        );
        assert_eq!(value.get("storage"), Some(&serde_json::json!("8Gi")));
        assert!(
            value.get("storage_mode").is_none(),
            "snake_case key must not leak into stored JSONB"
        );
    }

    #[test]
    fn config_carries_s3_storage_mode_from_git_repo_to_patch() {
        // The `s3` tier must survive repo config sync end-to-end the same way
        // `ephemeral` does: read from camelCase, round-trip back to camelCase in
        // the stored JSONB so provision-time deserialize resolves it.
        let json = r#"{ "resources": { "storageMode": "s3" } }"#;
        let config: MoatlessRepoConfig = serde_json::from_str(json).unwrap();
        assert_eq!(
            config
                .resources
                .as_ref()
                .and_then(|r| r.storage_mode.as_deref()),
            Some("s3")
        );

        let patch = config_to_patch(&config).unwrap();
        let value = patch
            .resources
            .expect("resources field set")
            .expect("resources value present");
        assert_eq!(value.get("storageMode"), Some(&serde_json::json!("s3")));
    }

    #[test]
    fn config_carries_node_storage_mode_from_git_repo_to_patch() {
        // The `node` tier is a valid non-PVC backend and must survive repo config
        // sync the same way as the other storage modes.
        let json = r#"{
            "resources": {
                "storageMode": "node",
                "storage": "30Gi",
                "dockerStorageMode": "node"
            }
        }"#;
        let config: MoatlessRepoConfig = serde_json::from_str(json).unwrap();
        assert_eq!(
            config
                .resources
                .as_ref()
                .and_then(|r| r.storage_mode.as_deref()),
            Some("node")
        );

        let patch = config_to_patch(&config).unwrap();
        let value = patch
            .resources
            .expect("resources field set")
            .expect("resources value present");
        assert_eq!(value.get("storageMode"), Some(&serde_json::json!("node")));
        assert_eq!(value.get("storage"), Some(&serde_json::json!("30Gi")));
        assert_eq!(
            value.get("dockerStorageMode"),
            Some(&serde_json::json!("node"))
        );
    }

    #[test]
    fn parse_workspaces_json() {
        let json = r#"{
            "version": 1,
            "workspaces": [
                {
                    "key": "api",
                    "name": "API",
                    "description": "backend",
                    "repos": [
                        { "url": "https://github.com/acme/api", "primary": true },
                        { "url": "https://github.com/acme/shared", "mountName": "shared" }
                    ],
                    "dockerImage": "acme/sandbox:latest",
                    "codeExecution": { "setupCommands": ["bun install"] },
                    "servers": [{ "name": "dev", "port": 3000, "default": true }],
                    "envVars": { "LOG_LEVEL": "debug" },
                    "networkProfile": "sealed"
                },
                { "name": "Web Only", "repos": [{ "url": "https://github.com/acme/web" }] }
            ]
        }"#;

        let file: MoatlessWorkspacesFile = serde_json::from_str(json).unwrap();
        assert_eq!(file.version, Some(1));
        assert_eq!(file.workspaces.len(), 2);

        let api = &file.workspaces[0];
        assert_eq!(api.key.as_deref(), Some("api"));
        assert_eq!(api.name, "API");
        assert_eq!(api.repos.len(), 2);
        assert!(api.repos[0].primary);
        assert_eq!(api.repos[0].url, "https://github.com/acme/api");
        assert_eq!(api.repos[1].mount_name.as_deref(), Some("shared"));
        assert!(!api.repos[1].primary);
        assert_eq!(api.docker_image.as_deref(), Some("acme/sandbox:latest"));
        assert_eq!(
            api.code_execution.as_ref().unwrap().setup_commands.as_ref().unwrap(),
            &vec!["bun install".to_string()]
        );
        assert_eq!(api.network_profile.as_deref(), Some("sealed"));

        // Missing key → resolved later from the name; primary defaults to false.
        let web = &file.workspaces[1];
        assert!(web.key.is_none());
        assert_eq!(web.name, "Web Only");
        assert!(!web.repos[0].primary);
    }

    #[test]
    fn verify_status_mapping() {
        use reqwest::StatusCode;
        assert_eq!(map_status_to_access(StatusCode::OK), AccessStatus::Ok);
        assert_eq!(
            map_status_to_access(StatusCode::NOT_FOUND),
            AccessStatus::NotFound
        );
        assert_eq!(
            map_status_to_access(StatusCode::UNAUTHORIZED),
            AccessStatus::Forbidden
        );
        assert_eq!(
            map_status_to_access(StatusCode::FORBIDDEN),
            AccessStatus::Forbidden
        );
        assert_eq!(
            map_status_to_access(StatusCode::INTERNAL_SERVER_ERROR),
            AccessStatus::Error
        );
        assert_eq!(
            map_status_to_access(StatusCode::TOO_MANY_REQUESTS),
            AccessStatus::Error
        );
    }

    #[test]
    fn access_status_as_str_values() {
        assert_eq!(AccessStatus::Unchecked.as_str(), "unchecked");
        assert_eq!(AccessStatus::Ok.as_str(), "ok");
        assert_eq!(AccessStatus::NotFound.as_str(), "not_found");
        assert_eq!(AccessStatus::Forbidden.as_str(), "forbidden");
        assert_eq!(AccessStatus::Error.as_str(), "error");
    }

    #[test]
    fn config_to_patch_empty_config() {
        let config = MoatlessRepoConfig {
            docker_image: None,
            code_execution: None,
            servers: None,
            system_prompt: None,
            resources: None,
            sandbox_service_account: None,
            network_profile: None,
        };

        let patch = config_to_patch(&config).unwrap();
        assert!(patch.servers.is_none());
        assert!(patch.setup_commands.is_none());
        assert!(patch.system_prompt.is_none());
    }
}
