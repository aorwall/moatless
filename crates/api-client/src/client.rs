use crate::ApiError;
use reqwest::StatusCode;
use serde::de::DeserializeOwned;

/// Typed HTTP client for the Moatless API.
pub struct ApiClient {
    base_url: String,
    http: reqwest::Client,
}

/// The inline fields of [`ApiClient::publish_artifact`].
#[derive(Debug, Clone, Default)]
pub struct InlineArtifact {
    pub height: Option<i32>,
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

    /// Get a reference to the inner `reqwest::Client`.
    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }

    // ========================================================================
    // Agents
    // ========================================================================

    /// GET /api/v1/agents
    pub async fn list_agents(
        &self,
    ) -> Result<moatless_api_schemas::agent::ListAgentsResponse, ApiError> {
        self.get("/api/v1/agents").await
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

    async fn patch<B: serde::Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, ApiError> {
        let url = format!("{}{path}", self.base_url);
        let resp = self.http.patch(&url).json(body).send().await?;
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
        serde_json::from_str(&body).map_err(|e| {
            // Truncate the response body in the error to avoid dumping megabytes of JSON
            let preview: &str = if body.len() > 200 {
                &body[..200]
            } else {
                &body
            };
            ApiError::InvalidJson(format!("{e} (response preview: {preview}…)"))
        })
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
    pub async fn me(&self) -> Result<moatless_api_schemas::auth::SessionResponse, ApiError> {
        self.get("/api/v1/auth/me").await
    }

    /// POST /api/v1/users
    pub async fn create_user(
        &self,
        req: &moatless_api_schemas::auth::CreateUserRequest,
    ) -> Result<moatless_api_schemas::auth::CreateUserResponse, ApiError> {
        self.post("/api/v1/users", req).await
    }

    /// POST /api/v1/auth/api-keys
    pub async fn create_api_key(
        &self,
        req: &moatless_api_schemas::auth::CreateApiKeyRequest,
    ) -> Result<moatless_api_schemas::auth::ApiKeyCreateResponse, ApiError> {
        self.post("/api/v1/auth/api-keys", req).await
    }

    /// GET /api/v1/auth/mode
    pub async fn auth_mode(
        &self,
    ) -> Result<moatless_api_schemas::auth::AuthModeResponse, ApiError> {
        self.get("/api/v1/auth/mode").await
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

    /// POST `/api/v1/tasks/{task_id}/cancel`
    pub async fn cancel_task(
        &self,
        task_id: &str,
    ) -> Result<moatless_api_schemas::task::CancelTaskResponse, ApiError> {
        self.post(
            &format!("/api/v1/tasks/{task_id}/cancel"),
            &serde_json::json!({}),
        )
        .await
    }

    /// GET `/api/v1/tasks/{task_id}`
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

    /// POST /`api/v1/tasks/{task_id}/fork`
    pub async fn fork_task(
        &self,
        task_id: &str,
        req: &moatless_api_schemas::task::ForkTaskRequest,
    ) -> Result<moatless_api_schemas::task::CreateTaskResponse, ApiError> {
        self.post(&format!("/api/v1/tasks/{task_id}/fork"), req)
            .await
    }

    /// POST /`api/v1/tasks/{task_id}/start`
    pub async fn start_task(&self, task_id: &str) -> Result<(), ApiError> {
        self.post_empty(&format!("/api/v1/tasks/{task_id}/start"))
            .await
    }

    /// POST `/api/v1/tasks/{task_id}/stop`
    pub async fn stop_task(&self, task_id: &str) -> Result<(), ApiError> {
        self.post_empty(&format!("/api/v1/tasks/{task_id}/stop"))
            .await
    }

    /// POST `/api/v1/tasks/{task_id}/close`
    pub async fn close_task(
        &self,
        task_id: &str,
        req: &moatless_api_schemas::task::CloseTaskRequest,
    ) -> Result<moatless_api_schemas::task::TaskResponse, ApiError> {
        self.post(&format!("/api/v1/tasks/{task_id}/close"), req)
            .await
    }

    /// POST `/api/v1/tasks/{task_id}/reopen`
    pub async fn reopen_task(&self, task_id: &str) -> Result<(), ApiError> {
        self.post_empty(&format!("/api/v1/tasks/{task_id}/reopen"))
            .await
    }

    /// DELETE `/api/v1/tasks/{task_id}`
    pub async fn delete_task(&self, task_id: &str) -> Result<(), ApiError> {
        self.delete_request(&format!("/api/v1/tasks/{task_id}"))
            .await
    }

    /// PATCH `/api/v1/tasks/{task_id}`
    pub async fn update_task_metadata(
        &self,
        task_id: &str,
        req: &moatless_api_schemas::task::UpdateTaskMetadataRequest,
    ) -> Result<moatless_api_schemas::task::TaskResponse, ApiError> {
        self.patch(&format!("/api/v1/tasks/{task_id}"), req).await
    }

    /// PUT `/api/v1/tasks/{task_id}/tags`
    pub async fn set_task_tags(
        &self,
        task_id: &str,
        req: &moatless_api_schemas::task::SetTagsRequest,
    ) -> Result<(), ApiError> {
        self.put_empty_with_body(&format!("/api/v1/tasks/{task_id}/tags"), req)
            .await
    }

    /// POST `/api/v1/tasks/{task_id}/repositories`
    pub async fn add_task_repository(
        &self,
        task_id: &str,
        req: &moatless_api_schemas::task::AddTaskRepositoryRequest,
    ) -> Result<Vec<moatless_api_schemas::task::TaskRepositoryResponse>, ApiError> {
        self.post(&format!("/api/v1/tasks/{task_id}/repositories"), req)
            .await
    }

    /// PUT `/api/v1/tasks/{task_id}/ttl`
    pub async fn update_ttl(
        &self,
        task_id: &str,
        req: &moatless_api_schemas::task::UpdateTaskTtlRequest,
    ) -> Result<moatless_api_schemas::task::TaskResponse, ApiError> {
        self.put(&format!("/api/v1/tasks/{task_id}/ttl"), req).await
    }

    /// PUT `/api/v1/tasks/{task_id}/visibility`
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

    /// GET `/api/sandbox/v1/tasks/{task_id}/status`
    pub async fn sandbox_status(
        &self,
        task_id: &str,
    ) -> Result<moatless_api_schemas::sandbox::SandboxStatusFullResponse, ApiError> {
        self.get(&format!("/api/sandbox/v1/tasks/{task_id}/status"))
            .await
    }

    /// GET /api/sandbox/v1/tasks/{task_id}/live-status
    pub async fn live_status(
        &self,
        task_id: &str,
    ) -> Result<moatless_api_schemas::sandbox::LiveStatus, ApiError> {
        self.get(&format!("/api/sandbox/v1/tasks/{task_id}/live-status"))
            .await
    }

    /// POST `/api/sandbox/v1/tasks/{task_id}/restart`
    pub async fn restart_sandbox(&self, task_id: &str) -> Result<(), ApiError> {
        self.post_empty(&format!("/api/sandbox/v1/tasks/{task_id}/restart"))
            .await
    }

    /// POST `/api/sandbox/v1/tasks/{task_id}/redeploy`
    pub async fn redeploy_sandbox(
        &self,
        task_id: &str,
    ) -> Result<moatless_api_schemas::sandbox::ProvisionResponse, ApiError> {
        self.post_no_body(&format!("/api/sandbox/v1/tasks/{task_id}/redeploy"))
            .await
    }

    /// POST `/api/sandbox/v1/tasks/{task_id}/workspace-repository/access`
    ///
    /// Platform admins only. Every field of the answer is a credential.
    pub async fn workspace_repository_access(
        &self,
        task_id: &str,
    ) -> Result<moatless_api_schemas::sandbox::WorkspaceRepositoryAccessResponse, ApiError> {
        self.post_no_body(&format!(
            "/api/sandbox/v1/tasks/{task_id}/workspace-repository/access"
        ))
        .await
    }

    /// GET `/api/sandbox/v1/tasks/{task_id}/files/list`
    pub async fn list_files(
        &self,
        task_id: &str,
    ) -> Result<moatless_api_schemas::sandbox::ListFilesResponse, ApiError> {
        self.get(&format!("/api/sandbox/v1/tasks/{task_id}/files/list"))
            .await
    }

    /// GET `/api/sandbox/v1/tasks/{task_id}/files/read?path={path}`
    pub async fn read_file(
        &self,
        task_id: &str,
        path: &str,
    ) -> Result<moatless_api_schemas::sandbox::FileContentResponse, ApiError> {
        self.get(&format!(
            "/api/sandbox/v1/tasks/{task_id}/files/read?path={}",
            urlenc(path)
        ))
        .await
    }

    /// POST `/api/sandbox/v1/tasks/{task_id}/files/write`
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

    /// GET `/api/sandbox/v1/tasks/{task_id}/files/tree`
    pub async fn file_tree(
        &self,
        task_id: &str,
    ) -> Result<moatless_api_schemas::sandbox::FileTreeResponse, ApiError> {
        self.get(&format!("/api/sandbox/v1/tasks/{task_id}/files/tree"))
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

    // ========================================================================
    // Repositories
    // ========================================================================

    /// GET /api/v1/repositories
    pub async fn list_repositories(
        &self,
    ) -> Result<Vec<moatless_workspace_types::schemas::repository::RepositoryResponse>, ApiError>
    {
        self.get("/api/v1/repositories").await
    }

    /// GET `/api/v1/repositories/{repo_id}`
    pub async fn get_repository(
        &self,
        repo_id: &str,
    ) -> Result<moatless_workspace_types::schemas::repository::RepositoryResponse, ApiError> {
        self.get(&format!("/api/v1/repositories/{repo_id}")).await
    }

    /// POST /api/v1/repositories
    pub async fn create_repository(
        &self,
        req: &moatless_workspace_types::schemas::repository::CreateRepositoryRequest,
    ) -> Result<moatless_workspace_types::schemas::repository::RepositoryResponse, ApiError> {
        self.post("/api/v1/repositories", req).await
    }

    /// PUT `/api/v1/repositories/{repo_id}`
    pub async fn update_repository(
        &self,
        repo_id: &str,
        req: &moatless_workspace_types::schemas::repository::UpdateRepositoryRequest,
    ) -> Result<moatless_workspace_types::schemas::repository::RepositoryResponse, ApiError> {
        self.put(&format!("/api/v1/repositories/{repo_id}"), req)
            .await
    }

    /// DELETE `/api/v1/repositories/{repo_id}`
    pub async fn delete_repository(&self, repo_id: &str) -> Result<(), ApiError> {
        self.delete_request(&format!("/api/v1/repositories/{repo_id}"))
            .await
    }

    /// POST /api/v1/repositories/{repo_id}/sync-config
    pub async fn sync_config(
        &self,
        repo_id: &str,
    ) -> Result<moatless_api_schemas::loops::SyncConfigResponse, ApiError> {
        self.post_no_body(&format!("/api/v1/repositories/{repo_id}/sync-config"))
            .await
    }

    // ========================================================================
    // Workspaces
    // ========================================================================

    /// GET /api/v1/workspaces
    pub async fn list_workspaces(
        &self,
    ) -> Result<Vec<moatless_workspace_types::schemas::workspace::WorkspaceResponse>, ApiError>
    {
        self.get("/api/v1/workspaces").await
    }

    /// GET `/api/v1/workspaces/{workspace_id}`
    pub async fn get_workspace(
        &self,
        workspace_id: &str,
    ) -> Result<moatless_workspace_types::schemas::workspace::WorkspaceResponse, ApiError> {
        self.get(&format!("/api/v1/workspaces/{workspace_id}"))
            .await
    }

    /// POST /api/v1/workspaces
    pub async fn create_workspace(
        &self,
        req: &moatless_workspace_types::schemas::workspace::CreateWorkspaceRequest,
    ) -> Result<moatless_workspace_types::schemas::workspace::WorkspaceResponse, ApiError> {
        self.post("/api/v1/workspaces", req).await
    }

    /// PATCH `/api/v1/workspaces/{workspace_id}`
    pub async fn update_workspace(
        &self,
        workspace_id: &str,
        req: &moatless_workspace_types::schemas::workspace::UpdateWorkspaceRequest,
    ) -> Result<moatless_workspace_types::schemas::workspace::WorkspaceResponse, ApiError> {
        self.patch(&format!("/api/v1/workspaces/{workspace_id}"), req)
            .await
    }

    /// DELETE `/api/v1/workspaces/{workspace_id}`
    pub async fn delete_workspace(&self, workspace_id: &str) -> Result<(), ApiError> {
        self.delete_request(&format!("/api/v1/workspaces/{workspace_id}"))
            .await
    }

    /// PATCH `/api/v1/workspaces/{workspace_id}/override`
    ///
    /// Sets `source = "manual"` so git sync skips this workspace;
    /// `reset_git_workspace` re-enables sync.
    pub async fn override_workspace(
        &self,
        workspace_id: &str,
        req: &moatless_workspace_types::schemas::workspace::UpdateWorkspaceRequest,
    ) -> Result<moatless_workspace_types::schemas::workspace::WorkspaceResponse, ApiError> {
        self.patch(&format!("/api/v1/workspaces/{workspace_id}/override"), req)
            .await
    }

    /// POST /api/v1/workspaces/{workspace_id}/reset-git
    ///
    /// Discards the local override and re-syncs run-config from the repository;
    /// `resynced` reports whether it took effect.
    pub async fn reset_git_workspace(
        &self,
        workspace_id: &str,
    ) -> Result<moatless_workspace_types::schemas::workspace::ResetGitWorkspaceResponse, ApiError>
    {
        self.post_no_body(&format!("/api/v1/workspaces/{workspace_id}/reset-git"))
            .await
    }

    /// GET `/api/v1/workspaces/{workspace_id}/credentials`
    ///
    /// Metadata only — a credential value is never returned.
    pub async fn list_workspace_credentials(
        &self,
        workspace_id: &str,
    ) -> Result<
        Vec<moatless_workspace_types::schemas::workspace::WorkspaceCredentialResponse>,
        ApiError,
    > {
        self.get(&format!("/api/v1/workspaces/{workspace_id}/credentials"))
            .await
    }

    /// PUT `/api/v1/workspaces/{workspace_id}/credentials/{provider}`
    pub async fn set_workspace_credential(
        &self,
        workspace_id: &str,
        provider: &str,
        value: &str,
    ) -> Result<moatless_workspace_types::schemas::workspace::WorkspaceCredentialResponse, ApiError>
    {
        let req = moatless_workspace_types::schemas::workspace::SetWorkspaceCredentialRequest {
            value: value.to_string(),
        };
        self.put(
            &format!("/api/v1/workspaces/{workspace_id}/credentials/{provider}"),
            &req,
        )
        .await
    }

    /// DELETE `/api/v1/workspaces/{workspace_id}/credentials/{provider}`
    pub async fn delete_workspace_credential(
        &self,
        workspace_id: &str,
        provider: &str,
    ) -> Result<(), ApiError> {
        self.delete_request(&format!(
            "/api/v1/workspaces/{workspace_id}/credentials/{provider}"
        ))
        .await
    }

    // ========================================================================
    // Messages
    // ========================================================================

    /// GET `/api/v1/tasks/{task_id}/messages`
    pub async fn get_messages(
        &self,
        task_id: &str,
    ) -> Result<moatless_api_schemas::message::GetMessagesResponse, ApiError> {
        self.get_messages_page(task_id, false, None, None).await
    }

    /// GET `/api/v1/tasks/{task_id}/messages` with explicit pagination.
    ///
    /// `limit` defaults to 50 (clamped to 1..=200); `before_id` is the cursor
    /// from a previous page's `next_before_id`, for fetching the next older
    /// page.
    pub async fn get_messages_page(
        &self,
        task_id: &str,
        summary: bool,
        limit: Option<i32>,
        before_id: Option<i32>,
    ) -> Result<moatless_api_schemas::message::GetMessagesResponse, ApiError> {
        use std::fmt::Write as _;
        let mut url = format!("/api/v1/tasks/{task_id}/messages?summary={summary}");
        if let Some(limit) = limit {
            // write! to a String is infallible
            let _ = write!(url, "&limit={limit}");
        }
        if let Some(before_id) = before_id {
            let _ = write!(url, "&before_id={before_id}");
        }
        self.get(&url).await
    }

    /// POST `/api/v1/tasks/{task_id}/messages`
    pub async fn send_message(
        &self,
        task_id: &str,
        req: &moatless_api_schemas::message::SendMessageRequest,
    ) -> Result<moatless_api_schemas::message::SendMessageResponse, ApiError> {
        self.post(&format!("/api/v1/tasks/{task_id}/messages"), req)
            .await
    }

    /// GET /api/v1/tasks/{task_id}/tool-calls/{tool_use_id}
    pub async fn get_tool_call_detail(
        &self,
        task_id: &str,
        tool_use_id: &str,
    ) -> Result<moatless_api_schemas::message::ToolCallDetailResponse, ApiError> {
        self.get(&format!("/api/v1/tasks/{task_id}/tool-calls/{tool_use_id}"))
            .await
    }

    // ========================================================================
    // Adapters
    // ========================================================================

    /// GET /api/v1/adapters
    pub async fn list_adapters(&self) -> Result<Vec<String>, ApiError> {
        self.get("/api/v1/adapters").await
    }

    /// GET /api/v1/adapters/connections
    pub async fn list_adapter_connections(
        &self,
        adapter_kind: Option<&str>,
    ) -> Result<Vec<moatless_api_schemas::adapters::AdapterConnectionResponse>, ApiError> {
        let qs = adapter_kind
            .map(|p| format!("?adapterKind={}", urlenc(p)))
            .unwrap_or_default();
        self.get(&format!("/api/v1/adapters/connections{qs}")).await
    }

    /// POST /api/v1/adapters/connections
    pub async fn create_adapter_connection(
        &self,
        req: &moatless_api_schemas::adapters::CreateAdapterConnectionRequest,
    ) -> Result<moatless_api_schemas::adapters::AdapterConnectionResponse, ApiError> {
        self.post("/api/v1/adapters/connections", req).await
    }

    /// POST /api/v1/adapters/send
    pub async fn adapter_send(
        &self,
        req: &moatless_api_schemas::adapters::AdapterSendRequest,
    ) -> Result<moatless_api_schemas::adapters::AdapterSendResponse, ApiError> {
        self.post("/api/v1/adapters/send", req).await
    }

    /// POST /api/v1/users/{login}/dm
    pub async fn send_user_dm(
        &self,
        login: &str,
        req: &moatless_api_schemas::user::SendDmRequest,
    ) -> Result<moatless_api_schemas::user::SendDmResponse, ApiError> {
        self.post(&format!("/api/v1/users/{login}/dm"), req).await
    }

    /// GET /api/v1/loops
    pub async fn list_loops(
        &self,
        kind: Option<&str>,
        adapter_kind: Option<&str>,
    ) -> Result<Vec<moatless_api_schemas::loops::Loop>, ApiError> {
        let mut params = Vec::new();
        if let Some(kind) = kind {
            params.push(format!("kind={}", urlenc(kind)));
        }
        if let Some(adapter_kind) = adapter_kind {
            params.push(format!("adapterKind={}", urlenc(adapter_kind)));
        }
        let qs = if params.is_empty() {
            String::new()
        } else {
            format!("?{}", params.join("&"))
        };
        self.get(&format!("/api/v1/loops{qs}")).await
    }

    /// POST /api/v1/loops
    pub async fn create_loop(
        &self,
        req: &moatless_api_schemas::loops::CreateLoopRequest,
    ) -> Result<moatless_api_schemas::loops::Loop, ApiError> {
        self.post("/api/v1/loops", req).await
    }

    /// GET /api/v1/loops/{id}
    pub async fn get_loop(&self, id: &str) -> Result<moatless_api_schemas::loops::Loop, ApiError> {
        self.get(&format!("/api/v1/loops/{id}")).await
    }

    /// PUT /api/v1/loops/{id}
    pub async fn update_loop(
        &self,
        id: &str,
        req: &moatless_api_schemas::loops::UpdateLoopRequest,
    ) -> Result<moatless_api_schemas::loops::Loop, ApiError> {
        self.put(&format!("/api/v1/loops/{id}"), req).await
    }

    /// DELETE /api/v1/loops/{id}
    pub async fn delete_loop(&self, id: &str) -> Result<(), ApiError> {
        self.delete_request(&format!("/api/v1/loops/{id}")).await
    }

    /// GET /api/v1/loops/{id}/tasks
    pub async fn list_loop_recent_tasks(
        &self,
        id: &str,
        limit: Option<u32>,
    ) -> Result<Vec<moatless_api_schemas::loops::LoopRecentTaskItem>, ApiError> {
        let qs = limit
            .map(|limit| format!("?limit={limit}"))
            .unwrap_or_default();
        self.get(&format!("/api/v1/loops/{id}/tasks{qs}")).await
    }

    /// POST /api/v1/loops/{id}/activate
    pub async fn activate_loop(
        &self,
        id: &str,
        req: &moatless_api_schemas::loops::ActivateLoopRequest,
    ) -> Result<moatless_api_schemas::loops::Loop, ApiError> {
        self.post(&format!("/api/v1/loops/{id}/activate"), req)
            .await
    }

    /// POST /api/v1/loops/{id}/pause
    pub async fn pause_loop(
        &self,
        id: &str,
    ) -> Result<moatless_api_schemas::loops::Loop, ApiError> {
        self.post(&format!("/api/v1/loops/{id}/pause"), &serde_json::json!({}))
            .await
    }

    /// POST /api/v1/loops/{id}/resume
    pub async fn resume_loop(
        &self,
        id: &str,
    ) -> Result<moatless_api_schemas::loops::Loop, ApiError> {
        self.post(
            &format!("/api/v1/loops/{id}/resume"),
            &serde_json::json!({}),
        )
        .await
    }

    /// POST /api/v1/loops/{id}/override
    pub async fn override_loop(
        &self,
        id: &str,
    ) -> Result<moatless_api_schemas::loops::Loop, ApiError> {
        self.post(
            &format!("/api/v1/loops/{id}/override"),
            &serde_json::json!({}),
        )
        .await
    }

    /// POST /api/v1/loops/{id}/reset-git
    pub async fn reset_git_loop(
        &self,
        id: &str,
    ) -> Result<moatless_api_schemas::loops::ResetGitLoopResponse, ApiError> {
        self.post(
            &format!("/api/v1/loops/{id}/reset-git"),
            &serde_json::json!({}),
        )
        .await
    }

    /// GET `/api/v1/tasks/{task_id}/bindings`
    pub async fn list_task_bindings(
        &self,
        task_id: &str,
    ) -> Result<Vec<moatless_api_schemas::bindings::TaskBindingResponse>, ApiError> {
        self.get(&format!("/api/v1/tasks/{task_id}/bindings")).await
    }

    /// POST `/api/v1/tasks/{task_id}/bindings`
    pub async fn create_task_binding(
        &self,
        task_id: &str,
        req: &moatless_api_schemas::bindings::CreateTaskBindingRequest,
    ) -> Result<moatless_api_schemas::bindings::TaskBindingResponse, ApiError> {
        self.post(&format!("/api/v1/tasks/{task_id}/bindings"), req)
            .await
    }

    /// POST `/api/v1/messages/{message_id}/reactions`
    pub async fn add_message_reaction(
        &self,
        message_id: &str,
        req: &moatless_api_schemas::message::AddMessageReactionRequest,
    ) -> Result<moatless_api_schemas::message::MessageReactionResponse, ApiError> {
        self.post(&format!("/api/v1/messages/{message_id}/reactions"), req)
            .await
    }

    /// DELETE `/api/v1/messages/{message_id}/reactions/{emoji}`
    pub async fn remove_message_reaction(
        &self,
        message_id: &str,
        emoji: &str,
    ) -> Result<(), ApiError> {
        self.delete_request(&format!(
            "/api/v1/messages/{message_id}/reactions/{}",
            urlenc(emoji)
        ))
        .await
    }

    /// POST `/api/v1/messages/{message_id}/reply`
    pub async fn reply_to_message(
        &self,
        message_id: &str,
        req: &moatless_api_schemas::message::ReplyToMessageRequest,
    ) -> Result<moatless_api_schemas::message::ReplyToMessageResponse, ApiError> {
        self.post(&format!("/api/v1/messages/{message_id}/reply"), req)
            .await
    }

    /// POST `/api/v1/tasks/{task_id}/reply`
    pub async fn send_task_reply(
        &self,
        task_id: &str,
        req: &moatless_api_schemas::bindings::SendTaskReplyRequest,
    ) -> Result<moatless_api_schemas::bindings::SendTaskReplyResponse, ApiError> {
        self.post(&format!("/api/v1/tasks/{task_id}/reply"), req)
            .await
    }

    // ========================================================================
    // Decisions
    // ========================================================================

    /// POST /api/v1/decisions/pick
    pub async fn pick_decision(
        &self,
        req: &moatless_api_schemas::decisions::PickDecisionRequest,
    ) -> Result<moatless_api_schemas::decisions::PickDecisionResponse, ApiError> {
        self.post("/api/v1/decisions/pick", req).await
    }

    // ========================================================================
    // Feedback
    // ========================================================================

    /// POST /api/v1/feedback
    pub async fn submit_feedback(
        &self,
        req: &moatless_api_schemas::feedback::CreateFeedbackRequest,
    ) -> Result<moatless_api_schemas::feedback::FeedbackResponse, ApiError> {
        self.post("/api/v1/feedback", req).await
    }

    /// PATCH `/api/v1/feedback/{feedback_id}/status`
    pub async fn set_feedback_status(
        &self,
        feedback_id: &str,
        req: &moatless_api_schemas::feedback::UpdateFeedbackStatusRequest,
    ) -> Result<moatless_api_schemas::feedback::FeedbackResponse, ApiError> {
        self.patch(&format!("/api/v1/feedback/{feedback_id}/status"), req)
            .await
    }

    /// GET `/api/v1/tasks/{task_id}/feedback`
    pub async fn list_task_feedback(
        &self,
        task_id: &str,
    ) -> Result<moatless_api_schemas::feedback::ListFeedbackResponse, ApiError> {
        self.get(&format!("/api/v1/tasks/{task_id}/feedback")).await
    }

    /// GET /api/v1/feedback
    pub async fn list_all_feedback(
        &self,
        query: &moatless_api_schemas::feedback::ListAllFeedbackQuery,
    ) -> Result<moatless_api_schemas::feedback::ListAllFeedbackResponse, ApiError> {
        let mut params = Vec::new();
        if let Some(ref workspace_id) = query.workspace_id {
            params.push(format!("workspaceId={}", urlenc(workspace_id)));
        }
        if let Some(ref repository_id) = query.repository_id {
            params.push(format!("repositoryId={}", urlenc(repository_id)));
        }
        if let Some(ref task_id) = query.task_id {
            params.push(format!("taskId={}", urlenc(task_id)));
        }
        if let Some(source) = query.source {
            params.push(format!("source={}", source.as_str()));
        }
        if let Some(surface) = query.surface {
            params.push(format!("surface={}", surface.as_str()));
        }
        if let Some(category) = query.category {
            params.push(format!("category={}", category.as_str()));
        }
        if let Some(status) = query.status {
            params.push(format!("status={}", status.as_str()));
        }
        if let Some(ref created_by) = query.created_by {
            params.push(format!("createdBy={}", urlenc(created_by)));
        }
        if let Some(ref since) = query.since {
            params.push(format!("since={}", urlenc(since)));
        }
        if let Some(limit) = query.limit {
            params.push(format!("limit={limit}"));
        }
        if let Some(offset) = query.offset {
            params.push(format!("offset={offset}"));
        }
        let qs = if params.is_empty() {
            String::new()
        } else {
            format!("?{}", params.join("&"))
        };
        self.get(&format!("/api/v1/feedback{qs}")).await
    }

    // ========================================================================
    // File uploads
    // ========================================================================

    /// POST /api/v1/uploads (multipart)
    pub async fn upload_file(
        &self,
        filename: &str,
        mime_type: &str,
        data: Vec<u8>,
    ) -> Result<moatless_api_schemas::file_storage::UploadResponse, ApiError> {
        let url = format!("{}/api/v1/uploads", self.base_url);
        let part = reqwest::multipart::Part::bytes(data)
            .file_name(filename.to_string())
            .mime_str(mime_type)
            .map_err(ApiError::Http)?;
        let form = reqwest::multipart::Form::new().part("file", part);
        let resp = self.http.post(&url).multipart(form).send().await?;
        self.parse_response(resp).await
    }

    // ========================================================================
    // Artifacts
    // ========================================================================

    /// POST `/api/v1/tasks/{task_id}/artifacts` — publish one HTML page,
    /// named `file_name`, as the next version of an Artifact. `inline` shows
    /// it in the Task's thread.
    pub async fn publish_artifact(
        &self,
        task_id: &str,
        title: &str,
        slug: Option<&str>,
        inline: Option<&InlineArtifact>,
        file_name: &str,
        html: Vec<u8>,
    ) -> Result<moatless_api_schemas::artifact::ArtifactResponse, ApiError> {
        let url = format!(
            "{}/api/v1/tasks/{}/artifacts",
            self.base_url,
            urlenc(task_id)
        );
        let mut form = reqwest::multipart::Form::new().text("title", title.to_string());
        if let Some(slug) = slug {
            form = form.text("slug", slug.to_string());
        }
        if let Some(inline) = inline {
            form = form.text("inline", "true");
            if let Some(height) = inline.height {
                form = form.text("height", height.to_string());
            }
        }
        form = form.part(
            "file",
            reqwest::multipart::Part::bytes(html).file_name(file_name.to_string()),
        );
        let resp = self.http.post(&url).multipart(form).send().await?;
        self.parse_response(resp).await
    }

    /// GET `/api/v1/tasks/{task_id}/artifacts`
    pub async fn list_task_artifacts(
        &self,
        task_id: &str,
    ) -> Result<moatless_api_schemas::artifact::ListArtifactsResponse, ApiError> {
        self.get(&format!("/api/v1/tasks/{}/artifacts", urlenc(task_id)))
            .await
    }

    /// GET `/api/v1/tasks/{task_id}/review-guide`
    pub async fn review_guide_run(
        &self,
        task_id: &str,
    ) -> Result<moatless_api_schemas::review_guide::ReviewGuideRunResponse, ApiError> {
        self.get(&format!("/api/v1/tasks/{}/review-guide", urlenc(task_id)))
            .await
    }

    /// POST `/api/v1/tasks/{task_id}/review-guide`
    pub async fn submit_review_guide(
        &self,
        task_id: &str,
        req: &moatless_api_schemas::review_guide::SubmitReviewGuideRequest,
    ) -> Result<moatless_api_schemas::review_guide::SubmitReviewGuideResponse, ApiError> {
        self.post(
            &format!("/api/v1/tasks/{}/review-guide", urlenc(task_id)),
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

    /// GET `/api/v1/teams/{team_id}`
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

    /// PUT `/api/v1/teams/{team_id}`
    pub async fn update_team(
        &self,
        team_id: &str,
        req: &moatless_api_schemas::team::UpdateTeamRequest,
    ) -> Result<moatless_api_schemas::team::TeamResponse, ApiError> {
        self.put(&format!("/api/v1/teams/{team_id}"), req).await
    }

    /// DELETE `/api/v1/teams/{team_id}`
    pub async fn delete_team(&self, team_id: &str) -> Result<(), ApiError> {
        self.delete_request(&format!("/api/v1/teams/{team_id}"))
            .await
    }

    /// POST `/api/v1/teams/{team_id}/members`
    pub async fn add_member(
        &self,
        team_id: &str,
        req: &moatless_api_schemas::team::AddTeamMemberRequest,
    ) -> Result<(), ApiError> {
        let url = format!("{}/api/v1/teams/{team_id}/members", self.base_url);
        let resp = self.http.post(&url).json(req).send().await?;
        self.check_status(resp).await
    }

    /// GET `/api/v1/teams/{team_id}/members`
    pub async fn list_members(
        &self,
        team_id: &str,
    ) -> Result<Vec<moatless_api_schemas::team::TeamMemberResponse>, ApiError> {
        self.get(&format!("/api/v1/teams/{team_id}/members")).await
    }

    /// DELETE `/api/v1/teams/{team_id}/members/{user_id}`
    pub async fn remove_member(&self, team_id: &str, user_id: &str) -> Result<(), ApiError> {
        self.delete_request(&format!("/api/v1/teams/{team_id}/members/{user_id}"))
            .await
    }

    // ========================================================================
    // Tags
    // ========================================================================

    /// GET /api/v1/tags
    pub async fn list_tags(&self) -> Result<Vec<moatless_api_schemas::tag::TagResponse>, ApiError> {
        self.get("/api/v1/tags").await
    }

    // ========================================================================
    // Plugins
    // ========================================================================

    /// GET /api/v1/plugins
    pub async fn list_plugins(
        &self,
    ) -> Result<Vec<moatless_workspace_types::schemas::plugin::PluginResponse>, ApiError> {
        self.get("/api/v1/plugins").await
    }

    /// POST /api/v1/plugins
    pub async fn create_plugin(
        &self,
        req: &moatless_workspace_types::schemas::plugin::CreatePluginRequest,
    ) -> Result<moatless_workspace_types::schemas::plugin::PluginResponse, ApiError> {
        self.post("/api/v1/plugins", req).await
    }

    /// DELETE /api/v1/plugins/{id}
    pub async fn delete_plugin(&self, id: &str) -> Result<(), ApiError> {
        self.delete_request(&format!("/api/v1/plugins/{id}")).await
    }

    /// GET /api/v1/plugins/{id}/skills
    pub async fn list_plugin_skills(
        &self,
        plugin_id: &str,
    ) -> Result<Vec<moatless_workspace_types::schemas::plugin::PluginSkillResponse>, ApiError> {
        self.get(&format!("/api/v1/plugins/{plugin_id}/skills"))
            .await
    }

    /// GET /api/v1/plugins/effective
    ///
    /// `user_id` asks what somebody else gets (server allows an administrator);
    /// omitted, the answer is the caller's own.
    pub async fn list_effective_plugins(
        &self,
        workspace_id: Option<&str>,
        user_id: Option<&str>,
    ) -> Result<Vec<moatless_workspace_types::schemas::plugin::EffectivePluginResponse>, ApiError>
    {
        let mut params: Vec<String> = Vec::new();
        if let Some(id) = workspace_id {
            params.push(format!("workspaceId={id}"));
        }
        if let Some(id) = user_id {
            params.push(format!("userId={id}"));
        }
        let qs = if params.is_empty() {
            String::new()
        } else {
            format!("?{}", params.join("&"))
        };
        self.get(&format!("/api/v1/plugins/effective{qs}")).await
    }

    /// PUT /api/v1/plugins/{id}/activation
    pub async fn set_plugin_activation(
        &self,
        plugin_id: &str,
        req: &moatless_workspace_types::schemas::plugin::SetActivationRequest,
    ) -> Result<moatless_workspace_types::schemas::plugin::ActivationResponse, ApiError> {
        self.put(&format!("/api/v1/plugins/{plugin_id}/activation"), req)
            .await
    }

    /// DELETE /api/v1/plugins/{id}/activation
    ///
    /// `user_id` clears somebody else's personal record (server allows an
    /// administrator); omitted, it clears the caller's own.
    pub async fn clear_plugin_activation(
        &self,
        plugin_id: &str,
        reach: moatless_workspace_types::schemas::plugin::ActivationReach,
        skill_name: Option<&str>,
        workspace_id: Option<&str>,
        user_id: Option<&str>,
    ) -> Result<(), ApiError> {
        let reach = match reach {
            moatless_workspace_types::schemas::plugin::ActivationReach::Everyone => "everyone",
            moatless_workspace_types::schemas::plugin::ActivationReach::Personal => "personal",
            moatless_workspace_types::schemas::plugin::ActivationReach::Workspace => "workspace",
        };
        let skill_qs = skill_name
            .map(|skill| format!("&skillName={skill}"))
            .unwrap_or_default();
        let workspace_qs = workspace_id
            .map(|id| format!("&workspaceId={id}"))
            .unwrap_or_default();
        let user_qs = user_id
            .map(|id| format!("&userId={id}"))
            .unwrap_or_default();
        let path = format!(
            "/api/v1/plugins/{plugin_id}/activation?reach={reach}{skill_qs}{workspace_qs}{user_qs}"
        );
        self.delete_request(&path).await
    }

    // ========================================================================
    // Linear
    // ========================================================================

    /// GET /api/v1/linear/issues/{id}
    pub async fn linear_get_issue(
        &self,
        id: &str,
        connection_id: Option<&str>,
    ) -> Result<moatless_api_schemas::linear::LinearIssueResponse, ApiError> {
        let qs = linear_connection_qs(connection_id);
        self.get(&format!("/api/v1/linear/issues/{id}{qs}")).await
    }

    /// POST /api/v1/linear/issues
    pub async fn linear_create_issue(
        &self,
        req: &moatless_api_schemas::linear::LinearIssueCreateRequest,
        connection_id: Option<&str>,
    ) -> Result<moatless_api_schemas::linear::LinearIssueResponse, ApiError> {
        let qs = linear_connection_qs(connection_id);
        self.post(&format!("/api/v1/linear/issues{qs}"), req).await
    }

    /// PATCH /api/v1/linear/issues/{id}
    pub async fn linear_update_issue(
        &self,
        id: &str,
        req: &moatless_api_schemas::linear::LinearIssueUpdateRequest,
        connection_id: Option<&str>,
    ) -> Result<moatless_api_schemas::linear::LinearIssueResponse, ApiError> {
        let qs = linear_connection_qs(connection_id);
        self.patch(&format!("/api/v1/linear/issues/{id}{qs}"), req)
            .await
    }

    /// POST /api/v1/linear/issues/{id}/comments
    pub async fn linear_create_comment(
        &self,
        id: &str,
        req: &moatless_api_schemas::linear::LinearCommentCreateRequest,
        connection_id: Option<&str>,
    ) -> Result<moatless_api_schemas::linear::LinearIssueResponse, ApiError> {
        let qs = linear_connection_qs(connection_id);
        self.post(&format!("/api/v1/linear/issues/{id}/comments{qs}"), req)
            .await
    }

    /// POST /api/v1/linear/issues/{id}/labels
    pub async fn linear_mutate_label(
        &self,
        id: &str,
        req: &moatless_api_schemas::linear::LinearLabelMutationRequest,
        connection_id: Option<&str>,
    ) -> Result<moatless_api_schemas::linear::LinearOkResponse, ApiError> {
        let qs = linear_connection_qs(connection_id);
        self.post(&format!("/api/v1/linear/issues/{id}/labels{qs}"), req)
            .await
    }

    /// POST /api/v1/linear/issues/{id}/relations
    pub async fn linear_relate_issue(
        &self,
        id: &str,
        req: &moatless_api_schemas::linear::LinearIssueRelateRequest,
        connection_id: Option<&str>,
    ) -> Result<moatless_api_schemas::linear::LinearIssueResponse, ApiError> {
        let qs = linear_connection_qs(connection_id);
        self.post(&format!("/api/v1/linear/issues/{id}/relations{qs}"), req)
            .await
    }

    /// GET /api/v1/linear/issues
    pub async fn linear_list_issues(
        &self,
        params: &moatless_api_schemas::linear::LinearIssueListParams,
    ) -> Result<moatless_api_schemas::linear::LinearIssueListResponse, ApiError> {
        let qs = linear_list_issues_qs(params);
        self.get(&format!("/api/v1/linear/issues{qs}")).await
    }

    /// POST /api/v1/linear/graphql
    pub async fn linear_graphql(
        &self,
        req: &moatless_api_schemas::linear::LinearGraphqlRequest,
        connection_id: Option<&str>,
    ) -> Result<serde_json::Value, ApiError> {
        let qs = linear_connection_qs(connection_id);
        self.post(&format!("/api/v1/linear/graphql{qs}"), req).await
    }

    /// GET /api/v1/linear/lookup
    pub async fn linear_lookup(
        &self,
        kind: &str,
        name: &str,
        team: Option<&str>,
        connection_id: Option<&str>,
    ) -> Result<moatless_api_schemas::linear::LinearLookupResponse, ApiError> {
        let mut qs = format!("?kind={}&name={}", urlenc(kind), urlenc(name));
        if let Some(team) = team {
            qs.push_str("&team=");
            qs.push_str(&urlenc(team));
        }
        if let Some(id) = connection_id {
            qs.push_str("&connectionId=");
            qs.push_str(&urlenc(id));
        }
        self.get(&format!("/api/v1/linear/lookup{qs}")).await
    }

    // ========================================================================
    // GitHub (typed proxy for sandbox agents — `moat gh`)
    // ========================================================================

    /// POST /api/v1/github/repos/{owner}/{repo}/pulls
    pub async fn github_create_pr(
        &self,
        owner: &str,
        repo: &str,
        req: &moatless_api_schemas::github::GithubCreatePrRequest,
    ) -> Result<moatless_api_schemas::github::GithubCreatePrResponse, ApiError> {
        self.post(&format!("/api/v1/github/repos/{owner}/{repo}/pulls"), req)
            .await
    }

    /// POST /api/v1/github/repos/{owner}/{repo}/issues/{number}/comments
    ///
    /// Serves both PR and issue comments (GitHub's issues API covers both).
    pub async fn github_post_issue_comment(
        &self,
        owner: &str,
        repo: &str,
        number: u64,
        req: &moatless_api_schemas::github::GithubCommentRequest,
    ) -> Result<moatless_api_schemas::github::GithubCommentResponse, ApiError> {
        self.post(
            &format!("/api/v1/github/repos/{owner}/{repo}/issues/{number}/comments"),
            req,
        )
        .await
    }

    /// POST
    /// `/api/v1/github/repos/{owner}/{repo}/pulls/{number}/comments/{comment_id}/replies`
    ///
    /// Threaded reply to a PR review (inline) comment.
    pub async fn github_reply_review_comment(
        &self,
        owner: &str,
        repo: &str,
        number: u64,
        comment_id: u64,
        req: &moatless_api_schemas::github::GithubReplyRequest,
    ) -> Result<moatless_api_schemas::github::GithubCommentResponse, ApiError> {
        self.post(
            &format!(
                "/api/v1/github/repos/{owner}/{repo}/pulls/{number}/comments/{comment_id}/replies"
            ),
            req,
        )
        .await
    }

    /// POST /api/v1/github/repos/{owner}/{repo}/pulls/{number}/bind
    ///
    /// Bind an existing pull request to a task without mutating GitHub.
    pub async fn github_bind_pr(
        &self,
        owner: &str,
        repo: &str,
        number: u64,
        req: &moatless_api_schemas::github::GithubBindPrRequest,
    ) -> Result<moatless_api_schemas::github::GithubBindPrResponse, ApiError> {
        self.post(
            &format!("/api/v1/github/repos/{owner}/{repo}/pulls/{number}/bind"),
            req,
        )
        .await
    }

    /// POST /api/v1/github/external-events
    ///
    /// Records a GitHub event the caller produced outside the typed routes
    /// above — a raw `gh api` POST, or a `git push`.
    pub async fn github_record_external_event(
        &self,
        req: &moatless_api_schemas::github::GithubExternalEventRequest,
    ) -> Result<moatless_api_schemas::github::GithubExternalEventResponse, ApiError> {
        self.post("/api/v1/github/external-events", req).await
    }
}

/// Build the query string for `GET /api/v1/linear/issues` (or empty).
///
/// Every filter field of `LinearIssueListParams` must be pushed here: the
/// handler deserializes with `#[serde(default)]`, so a missing field is
/// silently dropped and the call returns unfiltered results instead of failing.
fn linear_list_issues_qs(params: &moatless_api_schemas::linear::LinearIssueListParams) -> String {
    let mut qs: Vec<String> = Vec::new();
    let mut push = |key: &str, value: &str| qs.push(format!("{key}={}", urlenc(value)));
    if let Some(v) = params.query.as_deref() {
        push("query", v);
    }
    if let Some(v) = params.team.as_deref() {
        push("team", v);
    }
    if let Some(v) = params.project.as_deref() {
        push("project", v);
    }
    if let Some(v) = params.state.as_deref() {
        push("state", v);
    }
    if let Some(v) = params.label.as_deref() {
        push("label", v);
    }
    if let Some(v) = params.assignee.as_deref() {
        push("assignee", v);
    }
    if let Some(v) = params.delegate.as_deref() {
        push("delegate", v);
    }
    if params.open_only.unwrap_or(false) {
        push("openOnly", "true");
    }
    if let Some(v) = params.limit {
        push("limit", &v.to_string());
    }
    if let Some(v) = params.connection_id.as_deref() {
        push("connectionId", v);
    }
    if qs.is_empty() {
        String::new()
    } else {
        format!("?{}", qs.join("&"))
    }
}

/// Build the `?connectionId=...` query string fragment (or empty).
fn linear_connection_qs(connection_id: Option<&str>) -> String {
    connection_id
        .map(|id| format!("?connectionId={}", urlenc(id)))
        .unwrap_or_default()
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

#[cfg(test)]
mod tests {
    use super::*;
    use moatless_api_schemas::linear::LinearIssueListParams;

    /// Every filter a caller can set must reach the wire.
    ///
    /// Expected keys come from serializing the params, not a hand-written list,
    /// so a field added to `LinearIssueListParams` fails here until
    /// `linear_list_issues_qs` pushes it.
    #[test]
    fn list_issues_qs_carries_every_filter() {
        let params = LinearIssueListParams {
            query: Some("q".into()),
            team: Some("SP".into()),
            project: Some("proj".into()),
            state: Some("In Progress".into()),
            label: Some("moatless".into()),
            assignee: Some("me".into()),
            delegate: Some("moatless".into()),
            open_only: Some(true),
            limit: Some(7),
            connection_id: Some("conn".into()),
        };
        let qs = linear_list_issues_qs(&params);

        let fields: Vec<String> = serde_json::to_value(&params)
            .ok()
            .and_then(|v| v.as_object().map(|m| m.keys().cloned().collect()))
            .unwrap_or_default();
        assert!(!fields.is_empty(), "params must serialize to an object");
        let sent: Vec<&str> = qs
            .trim_start_matches('?')
            .split('&')
            .filter_map(|pair| pair.split_once('=').map(|(k, _)| k))
            .collect();
        for key in &fields {
            assert!(sent.contains(&key.as_str()), "{key} missing from {qs}");
        }

        // The keys above do not pin encoding, so check the one value that needs it.
        assert!(qs.contains("state=In%20Progress"), "{qs}");
    }

    #[test]
    fn list_issues_qs_is_empty_without_filters() {
        assert_eq!(linear_list_issues_qs(&LinearIssueListParams::default()), "");
    }
}
