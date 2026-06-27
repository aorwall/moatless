//! Reads `.moatless/config.json` from remote repositories via provider APIs.

use serde::Deserialize;
use tracing::debug;

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
}

/// Code execution settings.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeExecution {
    pub setup_commands: Option<Vec<String>>,
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
    match provider {
        RepositoryProvider::Github => fetch_github_config(remote_url, branch, token).await,
        RepositoryProvider::Gitlab => {
            let token = token.ok_or_else(|| {
                RepositoryError::BadRequest(
                    "GitLab requires an access token to fetch config".to_string(),
                )
            })?;
            fetch_gitlab_config(remote_url, branch, token).await
        }
        RepositoryProvider::Gitness => {
            // Gitness needs a base_url; skip for now – callers can pass it later.
            Ok(None)
        }
    }
}

/// Fetch from GitHub: `GET /repos/{owner}/{repo}/contents/.moatless/config.json?ref={branch}`
async fn fetch_github_config(
    remote_url: &str,
    branch: Option<&str>,
    token: Option<&str>,
) -> Result<Option<MoatlessRepoConfig>, RepositoryError> {
    let (owner, repo) = parse_github_owner_repo(remote_url).ok_or_else(|| {
        RepositoryError::BadRequest(format!("Cannot parse GitHub URL: {remote_url}"))
    })?;

    let mut url =
        format!("https://api.github.com/repos/{owner}/{repo}/contents/.moatless/config.json");
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
        debug!("No .moatless/config.json found in {owner}/{repo}");
        return Ok(None);
    }

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(RepositoryError::ProviderError(format!(
            "GitHub API returned {status} fetching config: {body}"
        )));
    }

    let config: MoatlessRepoConfig = resp.json().await.map_err(|e| {
        RepositoryError::ProviderError(format!("Failed to parse .moatless/config.json: {e}"))
    })?;

    Ok(Some(config))
}

/// Fetch from GitLab: `GET /api/v4/projects/{id}/repository/files/.moatless%2Fconfig.json/raw?ref={branch}`
async fn fetch_gitlab_config(
    remote_url: &str,
    branch: Option<&str>,
    token: &str,
) -> Result<Option<MoatlessRepoConfig>, RepositoryError> {
    let url_trimmed = remote_url.trim_end_matches('/').trim_end_matches(".git");

    let project_path = url_trimmed
        .strip_prefix("https://gitlab.com/")
        .or_else(|| url_trimmed.strip_prefix("http://gitlab.com/"))
        .ok_or_else(|| {
            RepositoryError::BadRequest(format!("Cannot parse GitLab URL: {remote_url}"))
        })?;

    let encoded_path = urlencoding::encode(project_path);
    let ref_param = branch.unwrap_or("HEAD");

    let api_url = format!(
        "https://gitlab.com/api/v4/projects/{encoded_path}/repository/files/.moatless%2Fconfig.json/raw?ref={ref_param}"
    );

    let client = reqwest::Client::new();
    let resp = client
        .get(&api_url)
        .header("PRIVATE-TOKEN", token)
        .send()
        .await
        .map_err(|e| RepositoryError::ProviderError(format!("GitLab API request failed: {e}")))?;

    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        debug!("No .moatless/config.json found in {project_path}");
        return Ok(None);
    }

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(RepositoryError::ProviderError(format!(
            "GitLab API returned {status} fetching config: {body}"
        )));
    }

    let config: MoatlessRepoConfig = resp.json().await.map_err(|e| {
        RepositoryError::ProviderError(format!("Failed to parse .moatless/config.json: {e}"))
    })?;

    Ok(Some(config))
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

    let resources = config
        .resources
        .as_ref()
        .map(serde_json::to_value)
        .transpose()
        .map_err(|e| RepositoryError::BadRequest(format!("Serialization error: {e}")))?
        .map(Some);

    let docker_image = config.docker_image.as_ref().map(|s| Some(s.clone()));

    Ok(RepositoryPatch {
        docker_image,
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
        };

        let patch = config_to_patch(&config).unwrap();
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
        assert_eq!(value.get("storageMode"), Some(&serde_json::json!("ephemeral")));
        assert_eq!(value.get("storage"), Some(&serde_json::json!("8Gi")));
        assert!(
            value.get("storage_mode").is_none(),
            "snake_case key must not leak into stored JSONB"
        );
    }

    #[test]
    fn config_to_patch_empty_config() {
        let config = MoatlessRepoConfig {
            docker_image: None,
            code_execution: None,
            servers: None,
            system_prompt: None,
            resources: None,
        };

        let patch = config_to_patch(&config).unwrap();
        assert!(patch.servers.is_none());
        assert!(patch.setup_commands.is_none());
        assert!(patch.system_prompt.is_none());
    }
}
