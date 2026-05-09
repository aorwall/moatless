//! gRPC server implementing `BackendSandboxService`.
//!
//! Receives events from sandbox pods via gRPC (replaces direct Redis XADD
//! from the sandbox) and serves pending messages (replaces HTTP polling).

use std::sync::Arc;

use serde_json::Value;
use tonic::{Request, Response, Status};
use tracing::{debug, info, warn};

use moatless_events::AgentSubType;
use sandbox_proto::backend_sandbox_service_server::BackendSandboxService;
use sandbox_proto::{
    AcknowledgeMessagesRequest, AcknowledgeMessagesResponse, DownloadUploadRequest,
    DownloadUploadResponse, GetCredentialsRequest, GetCredentialsResponse,
    GetPendingMessagesRequest, GetPendingMessagesResponse, GetSystemPromptRequest,
    GetSystemPromptResponse, InterruptedTurn as ProtoInterruptedTurn, PublishEventRequest,
    PublishEventResponse, RecoverInterruptedRunRequest, RecoverInterruptedRunResponse,
    UpdateCredentialRequest, UpdateCredentialResponse,
};

use crate::agent::prompt;
use crate::auth::github_app_service::GitHubAppService;
use crate::auth::provider_token_dao::ProviderTokenRepository;
use crate::auth::provider_tokens::ProviderTokenAuthRepository;
use crate::command::dao::CommandRepository;
use crate::config::Config;
use crate::credentials::{CredentialService, WriteSource};
use crate::events::EventPublisher;
use crate::file_storage::dao::FileRepository;
use crate::file_storage::s3 as file_s3;
use crate::message::dao::MessageRepository;
use crate::message::send::enrich_pending_with_commands;
use crate::repository::dao::RepoRepository;
use crate::sandbox::jwt;
use crate::sandbox::secrets::{
    resolve_server_env, resolve_task_credentials, resolve_task_team_id, TaskCredentialInputs,
};
use crate::secret::service::SecretService;
use crate::shared_models::AgentType;
use crate::task::dao::TaskRepository;

/// Known credential env var names.
///
/// Used as map keys in `GetCredentialsResponse` for type-safe credential handling.
pub struct CredentialKey;

impl CredentialKey {
    pub const GITHUB_TOKEN: &str = "GITHUB_TOKEN";
    pub const GITLAB_TOKEN: &str = "GITLAB_TOKEN";
    pub const GITNESS_TOKEN: &str = "GITNESS_TOKEN";
    pub const CODEX_AUTH_JSON: &str = "CODEX_AUTH_JSON";
    pub const OPENAI_API_KEY: &str = "OPENAI_API_KEY";
    pub const ANTHROPIC_API_KEY: &str = "ANTHROPIC_API_KEY";
    pub const CLAUDE_CODE_OAUTH_TOKEN: &str = "CLAUDE_CODE_OAUTH_TOKEN";
}

/// gRPC server state — holds event publisher, repository trait objects, and JWT secret.
pub struct BackendSandboxServiceImpl {
    pub events: Option<Arc<dyn EventPublisher>>,
    pub messages: Option<Arc<dyn MessageRepository>>,
    pub provider_tokens: Option<Arc<dyn ProviderTokenRepository>>,
    pub tasks: Option<Arc<dyn TaskRepository>>,
    pub commands: Option<Arc<dyn CommandRepository>>,
    /// JWT signing key for verifying sandbox auth tokens.
    pub task_token_secret: Option<String>,
    /// GitHub App service for refreshing expired user access tokens.
    pub github_app_service: Option<Arc<GitHubAppService>>,
    /// Provider token auth persistence for persisting refreshed tokens.
    pub provider_token_auth: Option<Arc<dyn ProviderTokenAuthRepository>>,
    /// Scoped secret resolver for global/team/user secrets.
    pub secrets: Option<Arc<SecretService>>,
    /// Generic credential service — used to route sandbox-pushed rotations
    /// through `store(.., WriteSource::SandboxRotation)` so the broadcast
    /// fan-out picks them up automatically.
    pub credentials_svc: Option<Arc<CredentialService>>,
    /// Repository DAO — used to load `Repository.system_prompt` and `servers`
    /// when assembling the agent's append system prompt.
    pub repositories: Option<Arc<dyn RepoRepository>>,
    /// Backend config — supplies `frontend_url`, `proxy.domain`, `proxy.protocol`
    /// for the agent's append system prompt.
    pub backend_config: Option<Arc<Config>>,
    /// S3 client used by `download_upload` to fetch attachment bytes.
    pub s3: Option<Arc<aws_sdk_s3::Client>>,
    /// File metadata repository — supplies filename and mime type when
    /// fulfilling `download_upload` requests.
    pub files: Option<Arc<dyn FileRepository>>,
    /// S3 bucket name (`config.s3.bucket_name`).
    pub s3_bucket: Option<String>,
    /// S3 key prefix for uploaded files (`config.s3.key_prefix`).
    pub s3_key_prefix: String,
}

impl BackendSandboxServiceImpl {
    /// Verify the authorization header from gRPC metadata.
    ///
    /// Extracts `Bearer <token>` from the `authorization` metadata key,
    /// verifies the JWT, and returns the claims.
    #[allow(clippy::result_large_err)]
    fn verify_auth<T>(&self, request: &Request<T>) -> Result<jwt::UserTokenClaims, Status> {
        let secret = self
            .task_token_secret
            .as_ref()
            .ok_or_else(|| Status::internal("TASK_TOKEN_SECRET not configured"))?;

        let auth_header = request
            .metadata()
            .get("authorization")
            .ok_or_else(|| Status::unauthenticated("Missing authorization header"))?
            .to_str()
            .map_err(|_| Status::unauthenticated("Invalid authorization header"))?;

        let token = auth_header
            .strip_prefix("Bearer ")
            .ok_or_else(|| Status::unauthenticated("Invalid authorization format"))?;

        jwt::verify_user_auth_token(token, secret)
            .map_err(|e| Status::unauthenticated(format!("Invalid token: {e}")))
    }
}

#[tonic::async_trait]
impl BackendSandboxService for BackendSandboxServiceImpl {
    /// Receive an event from the sandbox and publish it to Redis Streams.
    ///
    /// The sandbox sends events here instead of directly to Redis.
    /// The backend writes them to Redis in the same format so existing
    /// SSE consumers and event handlers continue to work unchanged.
    ///
    /// No auth verification — fire-and-forget, can't risk event loss.
    async fn publish_event(
        &self,
        request: Request<PublishEventRequest>,
    ) -> Result<Response<PublishEventResponse>, Status> {
        let event = request.into_inner();

        let Some(ref publisher) = self.events else {
            // Redis not configured — accept but discard
            debug!(task_id = %event.task_id, "Event received but Redis not configured, discarding");
            return Ok(Response::new(PublishEventResponse {
                event_id: String::new(),
            }));
        };

        // Parse the JSON payload
        let payload: serde_json::Value =
            serde_json::from_str(&event.payload_json).unwrap_or_default();

        // Convert proto EventType enum to moatless_events type
        let proto_event_type = sandbox_proto::EventType::try_from(event.event_type);

        let event_type_str = match proto_event_type {
            Ok(sandbox_proto::EventType::Task) => "task",
            Ok(sandbox_proto::EventType::TaskTurn) => "taskTurn",
            Ok(sandbox_proto::EventType::Sandbox) => "sandbox",
            Ok(sandbox_proto::EventType::Agent) => "agent",
            Ok(sandbox_proto::EventType::Server) => "server",
            Ok(sandbox_proto::EventType::Resources) => "resources",
            Err(_) => {
                warn!(
                    event_type = event.event_type,
                    task_id = %event.task_id,
                    "Unknown proto EventType value, rejecting"
                );
                return Err(Status::invalid_argument(format!(
                    "Unknown event_type: {}",
                    event.event_type
                )));
            }
        };

        // Use raw publish — the sandbox already provides typed sub_type strings
        // and the backend doesn't need to re-parse them for the Redis path.
        publisher
            .publish_raw_event(
                &event.task_id,
                event_type_str,
                &event.sub_type,
                None,
                None,
                payload,
            )
            .await;

        // Generate event ID matching the format publishers use
        let event_id = uuid::Uuid::new_v4().to_string();

        Ok(Response::new(PublishEventResponse { event_id }))
    }

    /// Return pending messages for a task.
    ///
    /// The sandbox polls this instead of the HTTP endpoint. The response
    /// format matches what the sandbox consumer expects.
    async fn get_pending_messages(
        &self,
        request: Request<GetPendingMessagesRequest>,
    ) -> Result<Response<GetPendingMessagesResponse>, Status> {
        // Verify auth if secret is configured
        if self.task_token_secret.is_some() {
            let _claims = self.verify_auth(&request)?;
        }

        let req = request.into_inner();

        let Some(ref messages) = self.messages else {
            return Ok(Response::new(GetPendingMessagesResponse {
                messages: vec![],
            }));
        };

        let task = if let Some(ref tasks) = self.tasks {
            tasks.find_by_id_internal(&req.task_id).await.ok().flatten()
        } else {
            None
        };
        let task_agent_type = task.as_ref().map_or_else(
            || AgentType::CLAUDE_CODE.to_string(),
            |t| t.agent_type.0.clone(),
        );

        // Fetch pending user messages from DB
        let mut pending = messages.get_pending_user(&req.task_id).await.map_err(|e| {
            warn!(error = %e, task_id = %req.task_id, "Failed to fetch pending messages");
            Status::internal("Failed to fetch pending messages")
        })?;

        // Enrich messages that carry a command_name with the command's prompt
        if let Some(ref commands) = self.commands {
            pending = enrich_pending_with_commands(pending, commands.as_ref()).await;
        }

        // Look up latest agent_session_id for --resume <id>
        let task_resume_session_id = messages
            .get_latest_agent_session_id(&req.task_id)
            .await
            .unwrap_or(None);

        // Check will_not_resume on the last result — if explicitly set (e.g.
        // by a command like "compact"), don't pass a resume session ID. Also
        // auto-skip resume when the last result row was an error: the prior
        // turn blew up (e.g. codex `--resume` against a missing rollout) and
        // resuming again would just loop the same failure.
        let task_resume_session_id = if task_resume_session_id.is_some() {
            match messages.get_last_result(&req.task_id).await {
                Ok(Some(last_result)) if last_result.will_not_resume == Some(true) => {
                    debug!(
                        task_id = %req.task_id,
                        "Last result has will_not_resume=true, clearing resume_session_id"
                    );
                    None
                }
                Ok(Some(last_result))
                    if last_result.message_type == crate::message::models::MessageType::Error =>
                {
                    debug!(
                        task_id = %req.task_id,
                        "Last result is an error, clearing resume_session_id to start fresh"
                    );
                    None
                }
                _ => task_resume_session_id,
            }
        } else {
            None
        };

        // Convert DB rows to proto messages.
        // message_content is a JSON array: [{"type":"text","text":"..."}, ...]
        let messages: Vec<sandbox_proto::PendingMessage> = pending
            .iter()
            .filter_map(|row| {
                let content = parse_content_blocks(&row.message_content);
                if content.is_empty() {
                    warn!(task_id = %req.task_id, "Pending message has no parseable content blocks");
                    return None;
                }

                // Per-message override: the user toggled "Start fresh" on
                // this specific message. Force resume off regardless of the
                // task-wide derivation.
                let (resume_session_id, resume) = if row.start_fresh {
                    debug!(
                        task_id = %req.task_id,
                        uuid = %row.uuid,
                        "Message has start_fresh=true, suppressing resume"
                    );
                    (None, Some(false))
                } else if task_resume_session_id.is_none() {
                    // When will_not_resume is set or last turn errored, clear
                    // the per-message resume flag so the sandbox starts fresh.
                    (None, Some(false))
                } else {
                    (task_resume_session_id.clone(), Some(row.resume))
                };

                Some(sandbox_proto::PendingMessage {
                    content,
                    model: row.model.clone(),
                    system_prompt: None,
                    resume,
                    agent_type: Some(task_agent_type.clone()),
                    is_error: None,
                    resume_session_id,
                    uuid: Some(row.uuid.clone()),
                    agent_mode: row.agent_mode.as_ref().map(|m| m.0.clone()),
                    file_ids: row.file_ids(),
                    start_fresh: Some(row.start_fresh),
                })
            })
            .collect();

        Ok(Response::new(GetPendingMessagesResponse { messages }))
    }

    /// Acknowledge that pending messages have been delivered to an agent session.
    ///
    /// Sets `sent_at`, `session_id`, and `agent_session_id` on the specified
    /// messages, transitioning them from "queued" to "sent".
    async fn acknowledge_messages(
        &self,
        request: Request<AcknowledgeMessagesRequest>,
    ) -> Result<Response<AcknowledgeMessagesResponse>, Status> {
        if self.task_token_secret.is_some() {
            let _claims = self.verify_auth(&request)?;
        }

        let req = request.into_inner();

        let Some(ref messages) = self.messages else {
            return Ok(Response::new(AcknowledgeMessagesResponse {}));
        };

        match messages
            .acknowledge_messages(&req.task_id, &req.message_uuids, &req.session_id)
            .await
        {
            Ok(count) => {
                info!(
                    task_id = %req.task_id,
                    session_id = %req.session_id,
                    requested = req.message_uuids.len(),
                    acknowledged = count,
                    "Messages acknowledged"
                );
            }
            Err(e) => {
                warn!(
                    task_id = %req.task_id,
                    error = %e,
                    "Failed to acknowledge messages"
                );
                return Err(Status::internal("Failed to acknowledge messages"));
            }
        }

        Ok(Response::new(AcknowledgeMessagesResponse {}))
    }

    /// Return credentials for sandbox initialization.
    ///
    /// Resolution chain:
    /// 1. Scoped global/team/user secrets.
    /// 2. Team-level provider_tokens compatibility reads.
    /// 3. User-level provider_tokens compatibility reads.
    async fn get_credentials(
        &self,
        request: Request<GetCredentialsRequest>,
    ) -> Result<Response<GetCredentialsResponse>, Status> {
        let claims = self.verify_auth(&request)?;
        let req = request.into_inner();

        debug!(
            task_id = %req.task_id,
            user_id = %claims.user_id,
            "GetCredentials request"
        );

        let team_id = resolve_task_team_id(self.tasks.as_ref(), &req.task_id).await;
        let credentials = resolve_task_credentials(TaskCredentialInputs {
            task_id: &req.task_id,
            user_id: &claims.user_id,
            team_id: team_id.as_deref(),
            secrets: self.secrets.as_ref(),
            provider_tokens: self.provider_tokens.as_ref(),
            github_app_service: self.github_app_service.as_ref(),
            provider_token_auth: self.provider_token_auth.as_ref(),
        })
        .await;
        let server_env =
            resolve_server_env(&claims.user_id, team_id.as_deref(), self.secrets.as_ref()).await;

        info!(
            task_id = %req.task_id,
            user_id = %claims.user_id,
            has_github = credentials.contains_key(CredentialKey::GITHUB_TOKEN),
            has_gitlab = credentials.contains_key(CredentialKey::GITLAB_TOKEN),
            has_gitness = credentials.contains_key(CredentialKey::GITNESS_TOKEN),
            has_codex_auth = credentials.contains_key(CredentialKey::CODEX_AUTH_JSON),
            has_openai = credentials.contains_key(CredentialKey::OPENAI_API_KEY),
            has_anthropic = credentials.contains_key(CredentialKey::ANTHROPIC_API_KEY),
            has_claude_oauth = credentials.contains_key(CredentialKey::CLAUDE_CODE_OAUTH_TOKEN),
            server_env_count = server_env.len(),
            "Credentials resolved"
        );

        Ok(Response::new(GetCredentialsResponse {
            credentials,
            server_env,
        }))
    }

    /// Persist a credential rotated locally inside the sandbox back into
    /// `provider_tokens`.
    ///
    /// Called by the sandbox when a runtime (currently Codex) rotates its OAuth
    /// refresh token. Without write-back the next backend-driven refresh would
    /// overwrite the rotated value with the now-stale stored copy, permanently
    /// invalidating the credential.
    ///
    /// Resolution mirrors `resolve_task_credentials`: if the task has a team
    /// and the team owns a token for the provider, the team-level row is
    /// updated; otherwise the user-level row is updated.
    async fn update_credential(
        &self,
        request: Request<UpdateCredentialRequest>,
    ) -> Result<Response<UpdateCredentialResponse>, Status> {
        let claims = self.verify_auth(&request)?;
        let req = request.into_inner();

        let Some(provider) = credential_key_to_provider(&req.key) else {
            warn!(
                task_id = %req.task_id,
                key = %req.key,
                "UpdateCredential rejected: unsupported credential key"
            );
            return Err(Status::invalid_argument(format!(
                "Unsupported credential key: {}",
                req.key
            )));
        };

        let Some(ref provider_tokens) = self.provider_tokens else {
            return Err(Status::failed_precondition(
                "Provider token repository not configured",
            ));
        };

        let team_id = resolve_task_team_id(self.tasks.as_ref(), &req.task_id).await;

        // When the credential service is wired (production path), route through
        // it so the broadcast fan-out fires for every running sandbox the user
        // owns — minus the originating one (race-loop guard).
        if team_id.is_none() {
            if let Some(ref svc) = self.credentials_svc {
                if let Err(e) = svc
                    .store(
                        &claims.user_id,
                        &req.key,
                        &req.value,
                        WriteSource::SandboxRotation {
                            task_id: req.task_id.clone(),
                        },
                    )
                    .await
                {
                    warn!(
                        error = %e,
                        task_id = %req.task_id,
                        user_id = %claims.user_id,
                        provider,
                        "CredentialService::store failed"
                    );
                    return Err(Status::internal("Failed to persist credential"));
                }
                info!(
                    task_id = %req.task_id,
                    user_id = %claims.user_id,
                    provider,
                    scope = "user",
                    "Persisted rotated credential via CredentialService"
                );
                return Ok(Response::new(UpdateCredentialResponse {}));
            }
        }

        // Match the read-side precedence in `resolve_task_credentials`: prefer
        // the team-level row when one exists, fall back to user-level otherwise.
        let team_row_exists = if let Some(ref tid) = team_id {
            match provider_tokens.find_team_token(tid, provider).await {
                Ok(row) => row.is_some(),
                Err(e) => {
                    warn!(
                        error = %e,
                        team_id = %tid,
                        provider,
                        "UpdateCredential: failed to probe team token"
                    );
                    false
                }
            }
        } else {
            false
        };

        let result = if team_row_exists {
            // SAFETY: team_id checked above for `team_row_exists` to be true.
            let tid = team_id
                .as_deref()
                .ok_or_else(|| Status::internal("team_id missing after team_row_exists check"))?;
            provider_tokens
                .upsert_team_token(tid, provider, &req.value, None, None, None)
                .await
        } else {
            provider_tokens
                .upsert_token(&claims.user_id, provider, &req.value, None, None, None)
                .await
        };

        if let Err(e) = result {
            warn!(
                error = %e,
                task_id = %req.task_id,
                user_id = %claims.user_id,
                provider,
                "Failed to persist rotated credential"
            );
            return Err(Status::internal("Failed to persist credential"));
        }

        info!(
            task_id = %req.task_id,
            user_id = %claims.user_id,
            provider,
            scope = if team_row_exists { "team" } else { "user" },
            "Persisted rotated credential"
        );

        Ok(Response::new(UpdateCredentialResponse {}))
    }

    /// Build and return the assembled append-system-prompt for a task.
    ///
    /// The sandbox calls this once per agent run (in `start_session`) and
    /// caches the result for fallback on subsequent failures. Assembly
    /// reads `Task.name`, `Task.append_system_prompt`, `Repository.system_prompt`,
    /// `Repository.servers`, and the configured frontend/proxy URLs.
    async fn get_system_prompt(
        &self,
        request: Request<GetSystemPromptRequest>,
    ) -> Result<Response<GetSystemPromptResponse>, Status> {
        let _claims = self.verify_auth(&request)?;
        let req = request.into_inner();

        let task = if let Some(ref tasks) = self.tasks {
            tasks.find_by_id_internal(&req.task_id).await.ok().flatten()
        } else {
            None
        };

        let system_prompt = prompt::build_for_task(
            &req.task_id,
            task.as_ref(),
            self.repositories.as_ref(),
            self.backend_config.as_ref(),
        )
        .await;

        Ok(Response::new(GetSystemPromptResponse { system_prompt }))
    }

    /// Stream uploaded-file bytes to the sandbox.
    ///
    /// The sandbox calls this for every `file_id` carried on a pending
    /// message before invoking the agent, so the on-disk path advertised in
    /// the prompt ("Saved to: …") is valid by the time the agent reads it.
    ///
    /// Auth: task-scoped JWT via gRPC metadata.
    /// Defense in depth: the file_id must be referenced by a message
    /// belonging to this task — prevents one task's sandbox from grabbing
    /// another's uploads.
    async fn download_upload(
        &self,
        request: Request<DownloadUploadRequest>,
    ) -> Result<Response<DownloadUploadResponse>, Status> {
        let _claims = self.verify_auth(&request)?;
        let req = request.into_inner();

        let Some(ref s3_client) = self.s3 else {
            return Err(Status::failed_precondition("S3 not configured"));
        };
        let Some(ref bucket) = self.s3_bucket else {
            return Err(Status::failed_precondition("S3 bucket not configured"));
        };
        let Some(ref files) = self.files else {
            return Err(Status::failed_precondition(
                "File repository not configured",
            ));
        };
        let Some(ref messages) = self.messages else {
            return Err(Status::failed_precondition(
                "Message repository not configured",
            ));
        };

        // Scope check: the file must be referenced by a message belonging to
        // this task. This is defense in depth on top of the JWT — even a
        // valid token shouldn't be able to pull another task's uploads.
        let used = messages
            .file_id_used_in_task(&req.task_id, &req.file_id)
            .await
            .map_err(|e| {
                warn!(error = %e, task_id = %req.task_id, file_id = %req.file_id, "file_id_used_in_task query failed");
                Status::internal("Failed to check file scope")
            })?;
        if !used {
            return Err(Status::not_found("File not attached to this task"));
        }

        let meta = files
            .find_by_id(&req.file_id)
            .await
            .map_err(|e| {
                warn!(error = %e, file_id = %req.file_id, "file metadata lookup failed");
                Status::internal("Failed to look up file metadata")
            })?
            .ok_or_else(|| Status::not_found("File metadata not found"))?;

        let s3_key = format!("{}/{}", self.s3_key_prefix, req.file_id);
        let (data, _content_type) = file_s3::get_object(s3_client, bucket, &s3_key)
            .await
            .map_err(|e| {
                warn!(error = %e, file_id = %req.file_id, "S3 get_object failed");
                Status::internal("Failed to fetch file from S3")
            })?;

        debug!(
            task_id = %req.task_id,
            file_id = %req.file_id,
            bytes = data.len(),
            "Served file upload to sandbox"
        );

        Ok(Response::new(DownloadUploadResponse {
            data,
            filename: meta.filename,
            mime_type: meta.mime_type,
        }))
    }

    /// Detect an interrupted agent run and return the data needed to replay it.
    ///
    /// Called by the sandbox once on startup, **before** pending-message
    /// dispatch. An interrupted run is a user-turn whose `agent_session_id`
    /// is set (the sandbox acknowledged it) but whose turn never reached a
    /// terminal `agent.completed/error/stopped` event row — i.e. the prior
    /// pod was killed mid-run.
    ///
    /// Note: turns whose terminal event was `agent.error` are excluded by
    /// `find_interrupted_turn` (the SQL filters on `sub_type IN
    /// ('completed', 'error', 'stopped')`), so a turn that already errored
    /// will never be replayed by the recovery path. Pair this with the
    /// auto-suppression of `resume_session_id` when the last result was an
    /// `Error` row in `get_pending_messages` to ensure the next user message
    /// also starts a fresh session instead of looping the same resume failure.
    ///
    /// Bumps `recovery_attempts` on each successful return. When the
    /// `MAX_RECOVERY_ATTEMPTS` cap is hit, inserts a synthetic Aborted Result
    /// row and publishes `agent.recovery_exhausted` so the UI can surface the
    /// failure clearly. This stops OOM-loop pods from hammering recovery.
    async fn recover_interrupted_run(
        &self,
        request: Request<RecoverInterruptedRunRequest>,
    ) -> Result<Response<RecoverInterruptedRunResponse>, Status> {
        if self.task_token_secret.is_some() {
            let _claims = self.verify_auth(&request)?;
        }

        let req = request.into_inner();

        let Some(ref messages) = self.messages else {
            return Ok(Response::new(RecoverInterruptedRunResponse { turn: None }));
        };

        let interrupted = match messages.find_interrupted_turn(&req.task_id).await {
            Ok(Some(turn)) => turn,
            Ok(None) => {
                debug!(task_id = %req.task_id, "RecoverInterruptedRun: no interrupted turn");
                return Ok(Response::new(RecoverInterruptedRunResponse { turn: None }));
            }
            Err(e) => {
                warn!(task_id = %req.task_id, error = %e, "find_interrupted_turn failed");
                return Err(Status::internal("Failed to query interrupted turn"));
            }
        };

        const MAX_RECOVERY_ATTEMPTS: i32 = 3;
        if interrupted.recovery_attempts >= MAX_RECOVERY_ATTEMPTS {
            warn!(
                task_id = %req.task_id,
                turn_number = interrupted.turn_number,
                attempts = interrupted.recovery_attempts,
                "RecoverInterruptedRun: cap reached, marking aborted"
            );
            let abort_text = format!(
                "Recovery abandoned after {} attempts. The agent run was \
                 interrupted by repeated sandbox restarts.",
                interrupted.recovery_attempts
            );
            if let Err(e) = messages
                .insert_aborted_result(&req.task_id, interrupted.turn_number, &abort_text)
                .await
            {
                warn!(error = %e, task_id = %req.task_id, "Failed to insert aborted result row");
            }
            if let Some(ref publisher) = self.events {
                publisher
                    .publish_agent_event(
                        &req.task_id,
                        AgentSubType::RecoveryExhausted,
                        None,
                        serde_json::json!({
                            "turn_number": interrupted.turn_number,
                            "session_id": interrupted.session_id,
                            "attempts": interrupted.recovery_attempts,
                        }),
                    )
                    .await;
            }
            return Ok(Response::new(RecoverInterruptedRunResponse { turn: None }));
        }

        let new_attempts = match messages
            .increment_recovery_attempts(interrupted.task_message_id)
            .await
        {
            Ok(n) => n,
            Err(e) => {
                warn!(error = %e, task_id = %req.task_id, "increment_recovery_attempts failed");
                return Err(Status::internal("Failed to increment recovery attempts"));
            }
        };

        let task_agent_type = if let Some(ref tasks) = self.tasks {
            tasks
                .find_by_id_internal(&req.task_id)
                .await
                .ok()
                .flatten()
                .map(|t| t.agent_type.0)
        } else {
            None
        };

        if let Some(ref publisher) = self.events {
            publisher
                .publish_agent_event(
                    &req.task_id,
                    AgentSubType::RecoveryStarted,
                    task_agent_type.as_deref(),
                    serde_json::json!({
                        "turn_number": interrupted.turn_number,
                        "session_id": interrupted.session_id,
                        "attempt": new_attempts,
                        "max_attempts": MAX_RECOVERY_ATTEMPTS,
                    }),
                )
                .await;
        }

        info!(
            task_id = %req.task_id,
            session_id = %interrupted.session_id,
            turn_number = interrupted.turn_number,
            attempt = new_attempts,
            "RecoverInterruptedRun: replaying interrupted turn"
        );

        Ok(Response::new(RecoverInterruptedRunResponse {
            turn: Some(ProtoInterruptedTurn {
                session_id: interrupted.session_id,
                turn_number: i64::from(interrupted.turn_number),
                prompt_text: interrupted.prompt_text,
                agent_type: task_agent_type,
                agent_mode: interrupted.agent_mode,
            }),
        }))
    }
}

/// Map a credential env-var key (the public namespace shared with sandbox env)
/// to the provider name used in the `provider_tokens` table.
///
/// Returns `None` for keys that are not write-back-eligible — only OAuth-style
/// rotating credentials need this path. Static API keys are managed entirely
/// in the backend.
fn credential_key_to_provider(key: &str) -> Option<&'static str> {
    match key {
        CredentialKey::CODEX_AUTH_JSON => Some("codex"),
        _ => None,
    }
}

/// Typed representation of a content block stored in the DB.
#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum DbContentBlock {
    Text {
        #[serde(default)]
        text: String,
    },
    Image {
        #[serde(default)]
        url: String,
        #[serde(default)]
        media_type: String,
    },
    /// Tool result from AskUserQuestion / ExitPlanMode responses.
    ToolResult {
        #[serde(default)]
        content: String,
    },
    #[serde(other)]
    Other,
}

/// Parse a `message_content` JSON value into proto `ContentBlock`s.
///
/// The DB stores content as a JSON array:
/// ```json
/// [{"type": "text", "text": "Hello"}, {"type": "image", "url": "...", "media_type": "image/png"}]
/// ```
/// Also handles a bare string as a single text block (edge case / legacy).
fn parse_content_blocks(value: &Value) -> Vec<sandbox_proto::ContentBlock> {
    match value {
        Value::Array(items) => items
            .iter()
            .filter_map(|item| {
                match serde_json::from_value::<DbContentBlock>(item.clone()) {
                    Ok(DbContentBlock::Text { text }) if !text.is_empty() => {
                        Some(sandbox_proto::ContentBlock {
                            block: Some(sandbox_proto::content_block::Block::Text(
                                sandbox_proto::TextBlock { text },
                            )),
                        })
                    }
                    Ok(DbContentBlock::Image { url, media_type }) => {
                        Some(sandbox_proto::ContentBlock {
                            block: Some(sandbox_proto::content_block::Block::Image(
                                sandbox_proto::ImageBlock { url, media_type },
                            )),
                        })
                    }
                    Ok(DbContentBlock::ToolResult { content }) if !content.is_empty() => {
                        Some(sandbox_proto::ContentBlock {
                            block: Some(sandbox_proto::content_block::Block::Text(
                                sandbox_proto::TextBlock { text: content },
                            )),
                        })
                    }
                    Ok(DbContentBlock::ToolResult { .. }) => None, // empty tool result
                    Ok(DbContentBlock::Other) => {
                        info!("Unknown content block type, skipping");
                        None
                    }
                    Ok(DbContentBlock::Text { .. }) => None, // empty text
                    Err(e) => {
                        warn!(error = %e, "Failed to parse content block, skipping");
                        None
                    }
                }
            })
            .collect(),
        Value::String(s) if !s.is_empty() => {
            vec![sandbox_proto::ContentBlock {
                block: Some(sandbox_proto::content_block::Block::Text(
                    sandbox_proto::TextBlock { text: s.clone() },
                )),
            }]
        }
        _ => vec![],
    }
}
