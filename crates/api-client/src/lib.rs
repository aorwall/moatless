//! Hand-written typed HTTP client for the Moatless API.
//!
//! Uses types from `moatless-api-schemas` for request/response serialization.

pub use moatless_api_schemas as types;

use reqwest::StatusCode;
use serde::de::DeserializeOwned;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ApiError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("API error ({status}): {body}")]
    Api { status: u16, body: String },
    #[error("Not authenticated — run `moat login`")]
    Unauthorized,
    #[error("Invalid JSON response: {0}")]
    InvalidJson(String),
}

/// Typed HTTP client for the Moatless API.
pub struct ApiClient {
    base_url: String,
    http: reqwest::Client,
}

impl ApiClient {
    pub fn new(base_url: &str, http: reqwest::Client) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            http,
        }
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Get a reference to the inner reqwest::Client.
    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }

    // ========================================================================
    // Internal helpers
    // ========================================================================

    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, ApiError> {
        let url = format!("{}{path}", self.base_url);
        let resp = self.http.get(&url).send().await?;
        self.parse_response(resp).await
    }

    async fn post<B: serde::Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, ApiError> {
        let url = format!("{}{path}", self.base_url);
        let resp = self.http.post(&url).json(body).send().await?;
        self.parse_response(resp).await
    }

    async fn post_no_body<T: DeserializeOwned>(&self, path: &str) -> Result<T, ApiError> {
        let url = format!("{}{path}", self.base_url);
        let resp = self.http.post(&url).send().await?;
        self.parse_response(resp).await
    }

    async fn post_empty(&self, path: &str) -> Result<(), ApiError> {
        let url = format!("{}{path}", self.base_url);
        let resp = self.http.post(&url).send().await?;
        self.check_status(resp).await
    }

    async fn put<B: serde::Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, ApiError> {
        let url = format!("{}{path}", self.base_url);
        let resp = self.http.put(&url).json(body).send().await?;
        self.parse_response(resp).await
    }

    async fn delete_request(&self, path: &str) -> Result<(), ApiError> {
        let url = format!("{}{path}", self.base_url);
        let resp = self.http.delete(&url).send().await?;
        self.check_status(resp).await
    }

    async fn put_empty_with_body<B: serde::Serialize>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<(), ApiError> {
        let url = format!("{}{path}", self.base_url);
        let resp = self.http.put(&url).json(body).send().await?;
        self.check_status(resp).await
    }

    async fn parse_response<T: DeserializeOwned>(
        &self,
        resp: reqwest::Response,
    ) -> Result<T, ApiError> {
        let status = resp.status();
        if status == StatusCode::UNAUTHORIZED {
            return Err(ApiError::Unauthorized);
        }
        let body = resp.text().await?;
        if !status.is_success() {
            return Err(ApiError::Api {
                status: status.as_u16(),
                body,
            });
        }
        serde_json::from_str(&body).map_err(|e| ApiError::InvalidJson(format!("{e}: {body}")))
    }

    async fn check_status(&self, resp: reqwest::Response) -> Result<(), ApiError> {
        let status = resp.status();
        if status == StatusCode::UNAUTHORIZED {
            return Err(ApiError::Unauthorized);
        }
        if !status.is_success() {
            let body = resp.text().await?;
            return Err(ApiError::Api {
                status: status.as_u16(),
                body,
            });
        }
        Ok(())
    }

    // ========================================================================
    // Auth
    // ========================================================================

    /// GET /api/v1/auth/me
    pub async fn me(
        &self,
    ) -> Result<moatless_api_schemas::auth::SessionResponse, ApiError> {
        self.get("/api/v1/auth/me").await
    }

    /// POST /api/v1/auth/token/session (sandbox token exchange)
    pub async fn exchange_token(
        &self,
    ) -> Result<moatless_api_schemas::auth::TokenSessionResponse, ApiError> {
        self.post_no_body("/api/v1/auth/token/session").await
    }

    // ========================================================================
    // Tasks
    // ========================================================================

    /// GET /api/v1/tasks
    pub async fn list_tasks(
        &self,
        query: &moatless_api_schemas::task::ListTasksQuery,
    ) -> Result<moatless_api_schemas::task::ListTasksResponse, ApiError> {
        let mut params = Vec::new();
        if let Some(limit) = query.limit {
            params.push(format!("limit={limit}"));
        }
        if let Some(offset) = query.offset {
            params.push(format!("offset={offset}"));
        }
        if let Some(ref status) = query.status {
            params.push(format!("status={}", status.as_str()));
        }
        if let Some(ref repo_id) = query.repository_id {
            params.push(format!("repositoryId={}", urlenc(repo_id)));
        }
        if let Some(ref created_by) = query.created_by {
            params.push(format!("createdBy={}", urlenc(created_by)));
        }
        if let Some(ref tag) = query.tag {
            params.push(format!("tag={}", urlenc(tag)));
        }
        if let Some(ref parent) = query.parent_task_id {
            params.push(format!("parentTaskId={}", urlenc(parent)));
        }
        let qs = if params.is_empty() {
            String::new()
        } else {
            format!("?{}", params.join("&"))
        };
        self.get(&format!("/api/v1/tasks{qs}")).await
    }

    /// GET /api/v1/tasks/{task_id}
    pub async fn get_task(
        &self,
        task_id: &str,
    ) -> Result<moatless_api_schemas::task::TaskResponse, ApiError> {
        self.get(&format!("/api/v1/tasks/{task_id}")).await
    }

    /// POST /api/v1/tasks
    pub async fn create_task(
        &self,
        req: &moatless_api_schemas::task::CreateTaskRequest,
    ) -> Result<moatless_api_schemas::task::CreateTaskResponse, ApiError> {
        self.post("/api/v1/tasks", req).await
    }

    /// POST /api/v1/tasks/{task_id}/start
    pub async fn start_task(&self, task_id: &str) -> Result<(), ApiError> {
        self.post_empty(&format!("/api/v1/tasks/{task_id}/start"))
            .await
    }

    /// POST /api/v1/tasks/{task_id}/stop
    pub async fn stop_task(&self, task_id: &str) -> Result<(), ApiError> {
        self.post_empty(&format!("/api/v1/tasks/{task_id}/stop"))
            .await
    }

    /// POST /api/v1/tasks/{task_id}/close
    pub async fn close_task(
        &self,
        task_id: &str,
    ) -> Result<moatless_api_schemas::task::TaskResponse, ApiError> {
        self.post_no_body(&format!("/api/v1/tasks/{task_id}/close"))
            .await
    }

    /// DELETE /api/v1/tasks/{task_id}
    pub async fn delete_task(&self, task_id: &str) -> Result<(), ApiError> {
        self.delete_request(&format!("/api/v1/tasks/{task_id}"))
            .await
    }

    /// PUT /api/v1/tasks/{task_id}/metadata
    pub async fn update_task_metadata(
        &self,
        task_id: &str,
        req: &moatless_api_schemas::task::UpdateTaskMetadataRequest,
    ) -> Result<moatless_api_schemas::task::TaskResponse, ApiError> {
        self.put(&format!("/api/v1/tasks/{task_id}/metadata"), req)
            .await
    }

    /// PUT /api/v1/tasks/{task_id}/tags
    pub async fn set_task_tags(
        &self,
        task_id: &str,
        req: &moatless_api_schemas::task::SetTagsRequest,
    ) -> Result<(), ApiError> {
        self.put_empty_with_body(&format!("/api/v1/tasks/{task_id}/tags"), req)
            .await
    }

    /// PUT /api/v1/tasks/{task_id}/visibility
    pub async fn update_visibility(
        &self,
        task_id: &str,
        req: &moatless_api_schemas::task::UpdateVisibilityRequest,
    ) -> Result<moatless_api_schemas::task::TaskResponse, ApiError> {
        self.put(&format!("/api/v1/tasks/{task_id}/visibility"), req)
            .await
    }

    // ========================================================================
    // Sandbox
    // ========================================================================

    /// GET /api/sandbox/v1/tasks/{task_id}/status
    pub async fn sandbox_status(
        &self,
        task_id: &str,
    ) -> Result<moatless_api_schemas::sandbox::SandboxStatusFullResponse, ApiError> {
        self.get(&format!("/api/sandbox/v1/tasks/{task_id}/status"))
            .await
    }

    /// POST /api/sandbox/v1/tasks/{task_id}/restart
    pub async fn restart_sandbox(&self, task_id: &str) -> Result<(), ApiError> {
        self.post_empty(&format!("/api/sandbox/v1/tasks/{task_id}/restart"))
            .await
    }

    /// POST /api/sandbox/v1/tasks/{task_id}/stop
    pub async fn stop_sandbox(&self, task_id: &str) -> Result<(), ApiError> {
        self.post_empty(&format!("/api/sandbox/v1/tasks/{task_id}/stop"))
            .await
    }

    /// POST /api/sandbox/v1/tasks/{task_id}/provision
    pub async fn provision_sandbox(
        &self,
        task_id: &str,
    ) -> Result<moatless_api_schemas::sandbox::ProvisionResponse, ApiError> {
        self.post_no_body(&format!(
            "/api/sandbox/v1/tasks/{task_id}/provision"
        ))
        .await
    }

    /// POST /api/sandbox/v1/tasks/{task_id}/redeploy
    pub async fn redeploy_sandbox(
        &self,
        task_id: &str,
    ) -> Result<moatless_api_schemas::sandbox::ProvisionResponse, ApiError> {
        self.post_no_body(&format!(
            "/api/sandbox/v1/tasks/{task_id}/redeploy"
        ))
        .await
    }

    /// POST /api/sandbox/v1/tasks/{task_id}/cleanup
    pub async fn cleanup_sandbox(&self, task_id: &str) -> Result<(), ApiError> {
        self.post_empty(&format!("/api/sandbox/v1/tasks/{task_id}/cleanup"))
            .await
    }

    /// GET /api/sandbox/v1/tasks/{task_id}/files
    pub async fn list_files(
        &self,
        task_id: &str,
    ) -> Result<moatless_api_schemas::sandbox::ListFilesResponse, ApiError> {
        self.get(&format!("/api/sandbox/v1/tasks/{task_id}/files"))
            .await
    }

    /// GET /api/sandbox/v1/tasks/{task_id}/files/content?path={path}
    pub async fn read_file(
        &self,
        task_id: &str,
        path: &str,
    ) -> Result<moatless_api_schemas::sandbox::FileContentResponse, ApiError> {
        self.get(&format!(
            "/api/sandbox/v1/tasks/{task_id}/files/content?path={}",
            urlenc(path)
        ))
        .await
    }

    /// POST /api/sandbox/v1/tasks/{task_id}/files/write
    pub async fn write_file(
        &self,
        task_id: &str,
        req: &moatless_api_schemas::sandbox::WriteFileRequestBody,
    ) -> Result<(), ApiError> {
        let url = format!(
            "{}/api/sandbox/v1/tasks/{task_id}/files/write",
            self.base_url
        );
        let resp = self.http.post(&url).json(req).send().await?;
        self.check_status(resp).await
    }

    /// GET /api/sandbox/v1/tasks/{task_id}/files/tree
    pub async fn file_tree(
        &self,
        task_id: &str,
    ) -> Result<moatless_api_schemas::sandbox::FileTreeResponse, ApiError> {
        self.get(&format!(
            "/api/sandbox/v1/tasks/{task_id}/files/tree"
        ))
        .await
    }

    /// POST /api/sandbox/v1/tasks/{task_id}/files/search-by-name
    pub async fn search_by_name(
        &self,
        task_id: &str,
        req: &moatless_api_schemas::sandbox::SearchByNameRequest,
    ) -> Result<moatless_api_schemas::sandbox::SearchByNameResponse, ApiError> {
        self.post(
            &format!("/api/sandbox/v1/tasks/{task_id}/files/search-by-name"),
            req,
        )
        .await
    }

    /// POST /api/sandbox/v1/tasks/{task_id}/search-files
    pub async fn search_files(
        &self,
        task_id: &str,
        req: &moatless_api_schemas::sandbox::SearchFilesRequest,
    ) -> Result<Vec<moatless_api_schemas::sandbox::FileSearchResult>, ApiError> {
        self.post(
            &format!("/api/sandbox/v1/tasks/{task_id}/search-files"),
            req,
        )
        .await
    }

    // ========================================================================
    // Repositories
    // ========================================================================

    /// GET /api/v1/repositories
    pub async fn list_repositories(
        &self,
    ) -> Result<Vec<moatless_api_schemas::repository::RepositoryResponse>, ApiError> {
        self.get("/api/v1/repositories").await
    }

    /// GET /api/v1/repositories/{repo_id}
    pub async fn get_repository(
        &self,
        repo_id: &str,
    ) -> Result<moatless_api_schemas::repository::RepositoryResponse, ApiError> {
        self.get(&format!("/api/v1/repositories/{repo_id}")).await
    }

    /// POST /api/v1/repositories
    pub async fn create_repository(
        &self,
        req: &moatless_api_schemas::repository::CreateRepositoryRequest,
    ) -> Result<moatless_api_schemas::repository::RepositoryResponse, ApiError> {
        self.post("/api/v1/repositories", req).await
    }

    /// PUT /api/v1/repositories/{repo_id}
    pub async fn update_repository(
        &self,
        repo_id: &str,
        req: &moatless_api_schemas::repository::UpdateRepositoryRequest,
    ) -> Result<moatless_api_schemas::repository::RepositoryResponse, ApiError> {
        self.put(&format!("/api/v1/repositories/{repo_id}"), req)
            .await
    }

    /// DELETE /api/v1/repositories/{repo_id}
    pub async fn delete_repository(&self, repo_id: &str) -> Result<(), ApiError> {
        self.delete_request(&format!("/api/v1/repositories/{repo_id}"))
            .await
    }

    /// POST /api/v1/repositories/{repo_id}/sync-config
    pub async fn sync_config(
        &self,
        repo_id: &str,
    ) -> Result<moatless_api_schemas::repository::RepositoryResponse, ApiError> {
        self.post_no_body(&format!(
            "/api/v1/repositories/{repo_id}/sync-config"
        ))
        .await
    }

    // ========================================================================
    // Messages
    // ========================================================================

    /// GET /api/v1/tasks/{task_id}/messages
    pub async fn get_messages(
        &self,
        task_id: &str,
    ) -> Result<moatless_api_schemas::message::GetMessagesResponse, ApiError> {
        self.get(&format!("/api/v1/tasks/{task_id}/messages")).await
    }

    /// POST /api/v1/tasks/{task_id}/messages
    pub async fn send_message(
        &self,
        task_id: &str,
        req: &moatless_api_schemas::message::SendMessageRequest,
    ) -> Result<moatless_api_schemas::message::SendMessageResponse, ApiError> {
        self.post(&format!("/api/v1/tasks/{task_id}/messages"), req)
            .await
    }

    // ========================================================================
    // Channels
    // ========================================================================

    /// GET /api/v1/channels/configs
    pub async fn list_channel_configs(
        &self,
        provider: Option<&str>,
        team_id: Option<&str>,
    ) -> Result<Vec<moatless_api_schemas::channel::ChannelConfigResponse>, ApiError> {
        let mut params = Vec::new();
        if let Some(p) = provider {
            params.push(format!("provider={}", urlenc(p)));
        }
        if let Some(t) = team_id {
            params.push(format!("teamId={}", urlenc(t)));
        }
        let qs = if params.is_empty() {
            String::new()
        } else {
            format!("?{}", params.join("&"))
        };
        self.get(&format!("/api/v1/channels/configs{qs}")).await
    }

    /// POST /api/v1/channels/configs
    pub async fn create_channel_config(
        &self,
        req: &moatless_api_schemas::channel::CreateChannelConfigRequest,
    ) -> Result<moatless_api_schemas::channel::ChannelConfigResponse, ApiError> {
        self.post("/api/v1/channels/configs", req).await
    }

    /// GET /api/v1/channels/configs/{config_id}
    pub async fn get_channel_config(
        &self,
        config_id: &str,
    ) -> Result<moatless_api_schemas::channel::ChannelConfigResponse, ApiError> {
        self.get(&format!("/api/v1/channels/configs/{config_id}"))
            .await
    }

    /// PUT /api/v1/channels/configs/{config_id}
    pub async fn update_channel_config(
        &self,
        config_id: &str,
        req: &moatless_api_schemas::channel::UpdateChannelConfigRequest,
    ) -> Result<moatless_api_schemas::channel::ChannelConfigResponse, ApiError> {
        self.put(&format!("/api/v1/channels/configs/{config_id}"), req)
            .await
    }

    /// DELETE /api/v1/channels/configs/{config_id}
    pub async fn delete_channel_config(&self, config_id: &str) -> Result<(), ApiError> {
        self.delete_request(&format!("/api/v1/channels/configs/{config_id}"))
            .await
    }

    /// GET /api/v1/tasks/{task_id}/channels
    pub async fn list_task_channels(
        &self,
        task_id: &str,
    ) -> Result<Vec<moatless_api_schemas::channel::TaskChannelResponse>, ApiError> {
        self.get(&format!("/api/v1/tasks/{task_id}/channels")).await
    }

    /// POST /api/v1/tasks/{task_id}/channels/send
    pub async fn send_to_task_channels(
        &self,
        task_id: &str,
        req: &moatless_api_schemas::channel::SendChannelMessageRequest,
    ) -> Result<moatless_api_schemas::channel::SendChannelMessageResponse, ApiError> {
        self.post(&format!("/api/v1/tasks/{task_id}/channels/send"), req)
            .await
    }

    /// POST /api/v1/tasks/{task_id}/channels/{config_id}/send
    pub async fn send_to_task_channel(
        &self,
        task_id: &str,
        config_id: &str,
        req: &moatless_api_schemas::channel::SendChannelMessageRequest,
    ) -> Result<moatless_api_schemas::channel::SendChannelMessageResponse, ApiError> {
        self.post(
            &format!("/api/v1/tasks/{task_id}/channels/{config_id}/send"),
            req,
        )
        .await
    }

    // ========================================================================
    // Teams
    // ========================================================================

    /// GET /api/v1/teams
    pub async fn list_teams(
        &self,
    ) -> Result<Vec<moatless_api_schemas::team::TeamResponse>, ApiError> {
        self.get("/api/v1/teams").await
    }

    /// GET /api/v1/teams/{team_id}
    pub async fn get_team(
        &self,
        team_id: &str,
    ) -> Result<moatless_api_schemas::team::TeamResponse, ApiError> {
        self.get(&format!("/api/v1/teams/{team_id}")).await
    }

    /// POST /api/v1/teams
    pub async fn create_team(
        &self,
        req: &moatless_api_schemas::team::CreateTeamRequest,
    ) -> Result<moatless_api_schemas::team::TeamResponse, ApiError> {
        self.post("/api/v1/teams", req).await
    }

    /// PUT /api/v1/teams/{team_id}
    pub async fn update_team(
        &self,
        team_id: &str,
        req: &moatless_api_schemas::team::UpdateTeamRequest,
    ) -> Result<moatless_api_schemas::team::TeamResponse, ApiError> {
        self.put(&format!("/api/v1/teams/{team_id}"), req).await
    }

    /// DELETE /api/v1/teams/{team_id}
    pub async fn delete_team(&self, team_id: &str) -> Result<(), ApiError> {
        self.delete_request(&format!("/api/v1/teams/{team_id}"))
            .await
    }

    /// POST /api/v1/teams/{team_id}/members
    pub async fn add_member(
        &self,
        team_id: &str,
        req: &moatless_api_schemas::team::AddTeamMemberRequest,
    ) -> Result<(), ApiError> {
        let url = format!("{}/api/v1/teams/{team_id}/members", self.base_url);
        let resp = self.http.post(&url).json(req).send().await?;
        self.check_status(resp).await
    }

    /// GET /api/v1/teams/{team_id}/members
    pub async fn list_members(
        &self,
        team_id: &str,
    ) -> Result<Vec<moatless_api_schemas::team::TeamMemberResponse>, ApiError> {
        self.get(&format!("/api/v1/teams/{team_id}/members")).await
    }

    /// DELETE /api/v1/teams/{team_id}/members/{user_id}
    pub async fn remove_member(
        &self,
        team_id: &str,
        user_id: &str,
    ) -> Result<(), ApiError> {
        self.delete_request(&format!(
            "/api/v1/teams/{team_id}/members/{user_id}"
        ))
        .await
    }

    /// PUT /api/v1/teams/{team_id}/credentials/{provider}
    pub async fn upsert_credential(
        &self,
        team_id: &str,
        provider: &str,
        req: &moatless_api_schemas::team::UpsertTeamCredentialRequest,
    ) -> Result<(), ApiError> {
        let url = format!(
            "{}/api/v1/teams/{team_id}/credentials/{provider}",
            self.base_url
        );
        let resp = self.http.put(&url).json(req).send().await?;
        self.check_status(resp).await
    }

    /// GET /api/v1/teams/{team_id}/credentials
    pub async fn list_credentials(
        &self,
        team_id: &str,
    ) -> Result<Vec<moatless_api_schemas::team::TeamCredentialResponse>, ApiError> {
        self.get(&format!("/api/v1/teams/{team_id}/credentials"))
            .await
    }

    /// DELETE /api/v1/teams/{team_id}/credentials/{provider}
    pub async fn delete_credential(
        &self,
        team_id: &str,
        provider: &str,
    ) -> Result<(), ApiError> {
        self.delete_request(&format!(
            "/api/v1/teams/{team_id}/credentials/{provider}"
        ))
        .await
    }

    // ========================================================================
    // Commands & Tags
    // ========================================================================

    /// GET /api/v1/commands
    pub async fn list_commands(
        &self,
    ) -> Result<Vec<moatless_api_schemas::command::CommandResponse>, ApiError> {
        self.get("/api/v1/commands").await
    }

    /// GET /api/v1/tags
    pub async fn list_tags(
        &self,
    ) -> Result<Vec<moatless_api_schemas::tag::TagResponse>, ApiError> {
        self.get("/api/v1/tags").await
    }

    // ========================================================================
    // Plugin Repos
    // ========================================================================

    /// GET /api/v1/plugin-repos
    pub async fn list_plugin_repos(
        &self,
    ) -> Result<Vec<moatless_api_schemas::plugin_repo::PluginRepoResponse>, ApiError> {
        self.get("/api/v1/plugin-repos").await
    }

    /// POST /api/v1/plugin-repos
    pub async fn create_plugin_repo(
        &self,
        req: &moatless_api_schemas::plugin_repo::CreatePluginRepoRequest,
    ) -> Result<moatless_api_schemas::plugin_repo::PluginRepoResponse, ApiError> {
        self.post("/api/v1/plugin-repos", req).await
    }

    /// DELETE /api/v1/plugin-repos/{id}
    pub async fn delete_plugin_repo(&self, id: &str) -> Result<(), ApiError> {
        self.delete_request(&format!("/api/v1/plugin-repos/{id}"))
            .await
    }

    /// GET /api/v1/repositories/{repo_id}/plugins
    pub async fn list_repo_plugins(
        &self,
        repo_id: &str,
    ) -> Result<Vec<moatless_api_schemas::plugin_repo::PluginRepoResponse>, ApiError> {
        self.get(&format!("/api/v1/repositories/{repo_id}/plugins"))
            .await
    }

    /// POST /api/v1/repositories/{repo_id}/plugins/{plugin_id}
    pub async fn link_plugin_to_repo(
        &self,
        repo_id: &str,
        plugin_id: &str,
    ) -> Result<(), ApiError> {
        self.post_empty(&format!(
            "/api/v1/repositories/{repo_id}/plugins/{plugin_id}"
        ))
        .await
    }

    /// DELETE /api/v1/repositories/{repo_id}/plugins/{plugin_id}
    pub async fn unlink_plugin_from_repo(
        &self,
        repo_id: &str,
        plugin_id: &str,
    ) -> Result<(), ApiError> {
        self.delete_request(&format!(
            "/api/v1/repositories/{repo_id}/plugins/{plugin_id}"
        ))
        .await
    }
}

/// Percent-encode a value for use in a query string.
fn urlenc(s: &str) -> String {
    // Simple percent-encoding for query parameters
    s.replace('%', "%25")
        .replace(' ', "%20")
        .replace('&', "%26")
        .replace('=', "%3D")
        .replace('+', "%2B")
        .replace('#', "%23")
}
