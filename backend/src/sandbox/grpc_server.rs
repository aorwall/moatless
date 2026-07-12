//! gRPC server implementing `BackendSandboxService`.
//!
//! Receives events from sandbox pods via gRPC (replaces direct Redis XADD
//! from the sandbox) and serves pending messages (replaces HTTP polling).

use std::pin::Pin;
use std::sync::Arc;

use futures::{Stream, StreamExt};
use serde_json::Value;
use tonic::{Request, Response, Status, Streaming};
use tracing::{debug, info, warn};

use moatless_events::AgentSubType;
use sandbox_proto::backend_sandbox_service_server::BackendSandboxService;
use sandbox_proto::{
    AcknowledgeMessagesRequest, AcknowledgeMessagesResponse, DownloadTranscriptRequest,
    DownloadUploadRequest, DownloadUploadResponse, GetCredentialsRequest, GetCredentialsResponse,
    GetPendingMessagesRequest, GetPendingMessagesResponse, GetSystemPromptRequest,
    GetSystemPromptResponse, GetWorkspaceSyncUrlsRequest, GetWorkspaceSyncUrlsResponse,
    InterruptedTurn as ProtoInterruptedTurn, PublishEventRequest, PublishEventResponse,
    RecoverInterruptedRunRequest, RecoverInterruptedRunResponse, ReportWorkspaceSyncRequest,
    ReportWorkspaceSyncResponse, TranscriptChunk, UpdateCredentialRequest, UpdateCredentialResponse,
    UploadTranscriptResponse,
};

use crate::agent::prompt;
use crate::auth::github_app_registry::GitHubAppRegistry;
use crate::auth::github_app_service::GitHubAppService;
use crate::auth::provider_token_dao::ProviderTokenRepository;
use crate::auth::provider_tokens::ProviderTokenAuthRepository;
use crate::auth::users::UserRepository;
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
    resolve_server_env, resolve_task_credentials, resolve_task_repo_owner, resolve_task_ssh_keys,
    resolve_task_team_id, TaskCredentialInputs,
};
use crate::secret::service::SecretService;
use crate::shared_models::AgentType;
use crate::task::dao::TaskRepository;
use crate::task::relations::TaskRelationRepository;

/// Known credential env var names.
///
/// Used as map keys in `GetCredentialsResponse` for type-safe credential handling.
pub struct CredentialKey;

impl CredentialKey {
    /// Long-lived user auth token the sandbox uses to authenticate its gRPC
    /// calls back to the backend. Delivered over this channel (instead of the pod
    /// spec) and swapped in by the sandbox after its bootstrap-authenticated
    /// initial fetch; re-minted on every fetch so the 15-min refresher keeps it
    /// fresh.
    pub const USER_AUTH_TOKEN: &str = "USER_AUTH_TOKEN";
    pub const GITHUB_TOKEN: &str = "GITHUB_TOKEN";
    pub const GITLAB_TOKEN: &str = "GITLAB_TOKEN";
    pub const GITNESS_TOKEN: &str = "GITNESS_TOKEN";
    pub const CODEX_AUTH_JSON: &str = "CODEX_AUTH_JSON";
    pub const OPENAI_API_KEY: &str = "OPENAI_API_KEY";
    pub const ANTHROPIC_API_KEY: &str = "ANTHROPIC_API_KEY";
    pub const CLAUDE_CODE_OAUTH_TOKEN: &str = "CLAUDE_CODE_OAUTH_TOKEN";
}

/// Build the S3 key for a task's transcript archive:
/// `<prefix>/<task_id>/transcripts.tar`. The `<prefix>/<task_id>/` portion is the
/// same one `provider.rs::cleanup` deletes, so uploads and cleanup stay aligned.
fn transcript_key(prefix: &str, task_id: &str) -> String {
    format!(
        "{}/{}/transcripts.tar",
        prefix.trim_end_matches('/'),
        task_id
    )
}

/// Build the S3 key for a task's workspace archive:
/// `<prefix>/<task_id>/workspace.tar`. The `<prefix>/<task_id>/` portion is the
/// same one `provider.rs::cleanup` deletes, so uploads and cleanup stay aligned.
fn workspace_key(prefix: &str, task_id: &str) -> String {
    format!("{}/{}/workspace.tar", prefix.trim_end_matches('/'), task_id)
}

/// Lifetime of the presigned workspace-sync URLs. The sandbox re-mints them on
/// every sync tick, so this only needs to cover a single transfer. Kept short to
/// bound the blast radius of a leaked URL, but generous enough for a large tar
/// on a slow link.
const WORKSPACE_PRESIGN_TTL: std::time::Duration = std::time::Duration::from_secs(15 * 60);

/// gRPC server state — holds event publisher, repository trait objects, and JWT secret.
pub struct BackendSandboxServiceImpl {
    pub events: Option<Arc<dyn EventPublisher>>,
    pub messages: Option<Arc<dyn MessageRepository>>,
    pub provider_tokens: Option<Arc<dyn ProviderTokenRepository>>,
    pub tasks: Option<Arc<dyn TaskRepository>>,
    pub task_relations: Option<Arc<dyn TaskRelationRepository>>,
    pub commands: Option<Arc<dyn CommandRepository>>,
    /// JWT signing key for verifying sandbox auth tokens.
    pub task_token_secret: Option<String>,
    /// GitHub App service for refreshing expired user access tokens
    /// (default user-auth App). Convenience handle to
    /// `github_apps.default_service()`.
    pub github_app_service: Option<Arc<GitHubAppService>>,
    /// Registry of all configured GitHub Apps — used to resolve installation
    /// tokens and bot identities for bot-user `provider_tokens` rows.
    pub github_apps: Arc<GitHubAppRegistry>,
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
    /// S3 key prefix for sandbox Claude transcripts (`config.s3.transcript_prefix`).
    /// Objects land at `<prefix>/<task_id>/transcripts.tar` in the same bucket.
    pub transcript_s3_prefix: String,
    /// S3 key prefix for `s3`-backend workspace archives (`config.s3.workspace_prefix`).
    /// Objects land at `<prefix>/<task_id>/workspace.tar` in the same bucket;
    /// the sandbox uploads/downloads them directly via presigned URLs.
    pub workspace_s3_prefix: String,
    /// User repository — used to render `<sent_by>` headers on pending
    /// messages so the agent sees the inbound sender's moatless login.
    pub users: Option<Arc<dyn UserRepository>>,
    /// Ingress dedup for sandbox-published events. The sandbox outbox delivers
    /// at-least-once (retries reuse the same `event_id`); this drops retried
    /// duplicates before they reach Redis so SSE consumers don't see them twice.
    pub event_dedup: Arc<crate::sandbox::event_dedup::EventDedup>,
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

    /// Verify auth for the `GetCredentials` call, which uniquely also accepts a
    /// short-lived bootstrap token (the sandbox presents it before it has the
    /// long-lived user token). During the sandbox-image rollout an old sandbox
    /// still presents its long-lived token here, so both are honoured.
    #[allow(clippy::result_large_err)]
    fn verify_auth_bootstrap_or_user<T>(
        &self,
        request: &Request<T>,
    ) -> Result<jwt::UserTokenClaims, Status> {
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

        jwt::verify_bootstrap_or_user_token(token, secret)
            .map_err(|e| Status::unauthenticated(format!("Invalid token: {e}")))
    }

    /// Mint the long-lived user auth token delivered over `GetCredentials`.
    ///
    /// The sandbox swaps this in for its bootstrap token and uses it for all
    /// subsequent gRPC calls. Returns `None` (with a warning) if the secret is
    /// unset — the sandbox then falls back to whatever token it already holds.
    fn mint_user_auth_token(&self, user_id: &str, login: &str) -> Option<String> {
        let secret = self.task_token_secret.as_ref()?;
        let ttl = self
            .backend_config
            .as_ref()
            .map_or(7 * 24 * 3600, |c| c.sandbox.user_token_ttl_secs);
        match jwt::generate_user_auth_token(user_id, login, secret, ttl) {
            Ok(token) => Some(token),
            Err(e) => {
                warn!(error = %e, user_id, "Failed to mint user auth token for credentials");
                None
            }
        }
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
    #[tracing::instrument(
        skip_all,
        fields(request_id = %uuid::Uuid::new_v4(), task_id = tracing::field::Empty)
    )]
    async fn publish_event(
        &self,
        request: Request<PublishEventRequest>,
    ) -> Result<Response<PublishEventResponse>, Status> {
        let event = request.into_inner();
        tracing::Span::current().record("task_id", event.task_id.as_str());

        // Log sequence gaps/resets for observability (never gates publishing).
        self.event_dedup
            .observe_sequence(&event.task_id, event.sequence)
            .await;

        // Drop retried duplicates at ingress. The outbox reuses `event_id`
        // across retries, so a duplicate here means the previous delivery was
        // already published to Redis — re-publishing would double it for SSE
        // consumers. Ack with the same id so the sandbox stops retrying.
        if self
            .event_dedup
            .is_duplicate(&event.task_id, &event.event_id)
            .await
        {
            debug!(
                task_id = %event.task_id,
                event_id = %event.event_id,
                sub_type = %event.sub_type,
                "Duplicate event dropped at ingress"
            );
            return Ok(Response::new(PublishEventResponse {
                event_id: event.event_id,
            }));
        }

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

        // Echo the sandbox's stable event_id back so retries dedupe on a
        // consistent id. Fall back to a fresh id for legacy senders that
        // predate the outbox and don't set one.
        let event_id = if event.event_id.is_empty() {
            uuid::Uuid::new_v4().to_string()
        } else {
            event.event_id
        };

        Ok(Response::new(PublishEventResponse { event_id }))
    }

    /// Return pending messages for a task.
    ///
    /// The sandbox polls this instead of the HTTP endpoint. The response
    /// format matches what the sandbox consumer expects.
    #[tracing::instrument(
        skip_all,
        fields(
            request_id = %uuid::Uuid::new_v4(),
            task_id = tracing::field::Empty,
            user_id = tracing::field::Empty
        )
    )]
    async fn get_pending_messages(
        &self,
        request: Request<GetPendingMessagesRequest>,
    ) -> Result<Response<GetPendingMessagesResponse>, Status> {
        // Verify auth if secret is configured
        if self.task_token_secret.is_some() {
            let claims = self.verify_auth(&request)?;
            tracing::Span::current().record("user_id", claims.user_id.as_str());
        }

        let req = request.into_inner();
        tracing::Span::current().record("task_id", req.task_id.as_str());

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

        // Gate: a Closed task is terminal — never start a turn on it. The
        // sandbox dispatcher no-ops on an empty list, so returning empty here
        // means no `agent.started`, no turn — even if the pod is still finishing
        // its termination grace period. Fail open on unknown (None / DB-error,
        // both collapsed by `.ok().flatten()`): only a confirmed-Closed task is
        // gated, mirroring `send.rs`.
        if matches!(
            task.as_ref().map(|t| t.status),
            Some(crate::task::models::TaskStatus::Closed)
        ) {
            debug!(task_id = %req.task_id, "get_pending_messages: task is Closed, returning no messages");
            return Ok(Response::new(GetPendingMessagesResponse {
                messages: vec![],
            }));
        }

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

        // Pre-resolve sent_by user logins for *internal* rows. External rows
        // (those with an `external_event_id`) carry `<message_id>`/`<source>`
        // headers instead and don't need a sender lookup.
        let mut login_cache: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        if let Some(ref users) = self.users {
            let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
            for row in &pending {
                if row.external_event_id.is_some() {
                    continue;
                }
                if let Some(uid) = row.sent_by.as_deref() {
                    if seen.insert(uid) {
                        match users.get_user(uid).await {
                            Ok(Some(user)) => {
                                login_cache.insert(uid.to_string(), user.login);
                            }
                            Ok(None) => {
                                debug!(user_id = %uid, "sent_by user not found, omitting <sent_by> tag");
                            }
                            Err(e) => {
                                warn!(user_id = %uid, error = %e, "user lookup failed, omitting <sent_by> tag");
                            }
                        }
                    }
                }
            }
        }

        // Convert DB rows to proto messages.
        // message_content is a JSON array: [{"type":"text","text":"..."}, ...]
        let messages: Vec<sandbox_proto::PendingMessage> = pending
            .iter()
            .filter_map(|row| {
                let mut content = parse_content_blocks(&row.message_content);
                if content.is_empty() {
                    warn!(task_id = %req.task_id, "Pending message has no parseable content blocks");
                    return None;
                }

                let header = build_message_header(
                    &row.uuid,
                    row.external_event_id.as_deref(),
                    row.external_adapter.as_deref(),
                    row.external_subject_key.as_deref(),
                    row.sent_by.as_deref(),
                    &login_cache,
                );
                if !header.is_empty() {
                    prepend_header(&mut content, &header);
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
                    model: effective_model(row.model.clone(), &task_agent_type),
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
    #[tracing::instrument(
        skip_all,
        fields(
            request_id = %uuid::Uuid::new_v4(),
            task_id = tracing::field::Empty,
            user_id = tracing::field::Empty
        )
    )]
    async fn get_credentials(
        &self,
        request: Request<GetCredentialsRequest>,
    ) -> Result<Response<GetCredentialsResponse>, Status> {
        // GetCredentials uniquely accepts the short-lived bootstrap token (and,
        // during rollout, the legacy long-lived token) — this is where the
        // sandbox trades up to the long-lived user token delivered below.
        let claims = self.verify_auth_bootstrap_or_user(&request)?;
        let req = request.into_inner();
        tracing::Span::current().record("task_id", req.task_id.as_str());
        tracing::Span::current().record("user_id", claims.user_id.as_str());

        debug!(
            task_id = %req.task_id,
            user_id = %claims.user_id,
            "GetCredentials request"
        );

        let team_id = resolve_task_team_id(self.tasks.as_ref(), &req.task_id).await;
        let repo_owner = resolve_task_repo_owner(
            self.tasks.as_ref(),
            self.repositories.as_ref(),
            &req.task_id,
        )
        .await;
        let mut credentials = resolve_task_credentials(TaskCredentialInputs {
            task_id: &req.task_id,
            user_id: &claims.user_id,
            team_id: team_id.as_deref(),
            secrets: self.secrets.as_ref(),
            provider_tokens: self.provider_tokens.as_ref(),
            github_app_service: self.github_app_service.as_ref(),
            provider_token_auth: self.provider_token_auth.as_ref(),
            github_apps: Some(&self.github_apps),
            repo_owner: repo_owner.as_deref(),
            users: self.users.as_ref(),
            credentials_svc: self.credentials_svc.as_ref(),
        })
        .await;

        // Deliver the long-lived user auth token over this channel rather than
        // the pod spec. The sandbox swaps its bootstrap token for this one and
        // re-fetches on the 15-min refresh loop, so a rotated token propagates.
        if let Some(user_token) = self.mint_user_auth_token(&claims.user_id, &claims.login) {
            credentials.insert(CredentialKey::USER_AUTH_TOKEN.to_string(), user_token);
        }
        let server_env =
            resolve_server_env(&claims.user_id, team_id.as_deref(), self.secrets.as_ref()).await;
        let ssh_keys =
            resolve_task_ssh_keys(self.secrets.as_ref(), &claims.user_id, team_id.as_deref()).await;

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
            ssh_key_count = ssh_keys.len(),
            "Credentials resolved"
        );

        Ok(Response::new(GetCredentialsResponse {
            credentials,
            server_env,
            ssh_keys,
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
    #[tracing::instrument(
        skip_all,
        fields(
            request_id = %uuid::Uuid::new_v4(),
            task_id = tracing::field::Empty,
            user_id = tracing::field::Empty
        )
    )]
    async fn update_credential(
        &self,
        request: Request<UpdateCredentialRequest>,
    ) -> Result<Response<UpdateCredentialResponse>, Status> {
        let claims = self.verify_auth(&request)?;
        let req = request.into_inner();
        tracing::Span::current().record("task_id", req.task_id.as_str());
        tracing::Span::current().record("user_id", claims.user_id.as_str());

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
                .upsert_team_token(tid, provider, "manual", &req.value, None, None, None)
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
            self.task_relations.as_ref(),
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

    /// Receive a streamed transcript tar from the sandbox and store it in S3.
    ///
    /// The sandbox is AWS-credential-less, so it proxies all transcript I/O
    /// through here. Client-streaming so a transcript bigger than tonic's unary
    /// cap still round-trips; the chunks are reassembled in memory (a task's tar
    /// is a few MB) and `put_object`ed to `<bucket>/<prefix>/<task_id>/transcripts.tar`.
    ///
    /// Auth: task-scoped JWT via gRPC metadata, like `download_upload`. The
    /// `task_id` is taken from the first chunk.
    async fn upload_transcript(
        &self,
        request: Request<Streaming<TranscriptChunk>>,
    ) -> Result<Response<UploadTranscriptResponse>, Status> {
        let _claims = self.verify_auth(&request)?;

        let Some(ref s3_client) = self.s3 else {
            return Err(Status::failed_precondition("S3 not configured"));
        };
        let Some(ref bucket) = self.s3_bucket else {
            return Err(Status::failed_precondition("S3 bucket not configured"));
        };

        let mut stream = request.into_inner();
        let mut task_id = String::new();
        let mut data: Vec<u8> = Vec::new();
        while let Some(chunk) = stream.message().await? {
            if task_id.is_empty() && !chunk.task_id.is_empty() {
                task_id = chunk.task_id;
            }
            data.extend_from_slice(&chunk.data);
        }

        if task_id.is_empty() {
            return Err(Status::invalid_argument(
                "task_id missing from transcript stream",
            ));
        }

        let bytes_received = data.len() as i64;
        let key = transcript_key(&self.transcript_s3_prefix, &task_id);
        file_s3::put_object(s3_client, bucket, &key, data, "application/x-tar")
            .await
            .map_err(|e| {
                warn!(error = %e, task_id = %task_id, "S3 put_object for transcript failed");
                Status::internal("Failed to store transcript in S3")
            })?;

        debug!(task_id = %task_id, bytes = bytes_received, key = %key, "Stored transcript in S3");
        Ok(Response::new(UploadTranscriptResponse { bytes_received }))
    }

    type DownloadTranscriptStream =
        Pin<Box<dyn Stream<Item = Result<TranscriptChunk, Status>> + Send>>;

    /// Stream the task's stored transcript tar back to the sandbox.
    ///
    /// Returns an empty stream when no transcript has been stored yet (first run,
    /// or the ephemeral tier's first boot) — the sandbox treats that as "nothing
    /// to restore". Reuses `AppState.s3` + the uploads bucket like `download_upload`.
    async fn download_transcript(
        &self,
        request: Request<DownloadTranscriptRequest>,
    ) -> Result<Response<Self::DownloadTranscriptStream>, Status> {
        let _claims = self.verify_auth(&request)?;
        let req = request.into_inner();

        let Some(ref s3_client) = self.s3 else {
            return Err(Status::failed_precondition("S3 not configured"));
        };
        let Some(ref bucket) = self.s3_bucket else {
            return Err(Status::failed_precondition("S3 bucket not configured"));
        };

        let key = transcript_key(&self.transcript_s3_prefix, &req.task_id);
        let data = file_s3::get_object_opt(s3_client, bucket, &key)
            .await
            .map_err(|e| {
                warn!(error = %e, task_id = %req.task_id, "S3 get_object for transcript failed");
                Status::internal("Failed to fetch transcript from S3")
            })?
            .unwrap_or_default();

        debug!(task_id = %req.task_id, bytes = data.len(), "Serving transcript to sandbox");

        // Chunk the in-memory tar so each gRPC message stays well under the default
        // limit. An empty tar yields no chunks → an empty stream.
        const CHUNK_SIZE: usize = 1024 * 1024; // 1 MiB
        let chunks: Vec<TranscriptChunk> = data
            .chunks(CHUNK_SIZE)
            .map(|c| TranscriptChunk {
                task_id: String::new(),
                data: c.to_vec(),
            })
            .collect();

        let stream = futures::stream::iter(chunks).map(Ok::<TranscriptChunk, Status>);
        Ok(Response::new(Box::pin(stream)))
    }

    /// Mint presigned PUT + GET URLs for the task's workspace archive.
    ///
    /// The `s3` storage backend uses this instead of the transcript-style gRPC
    /// proxy: the workspace tar can be hundreds of MB, so rather than stream it
    /// through the backend, we hand the sandbox short-lived URLs scoped to
    /// exactly `<prefix>/<task_id>/workspace.tar` and it transfers directly to
    /// S3. The sandbox stays AWS-credential-less; the URL is the only capability
    /// it holds and it expires. Auth: task-scoped JWT via gRPC metadata.
    async fn get_workspace_sync_urls(
        &self,
        request: Request<GetWorkspaceSyncUrlsRequest>,
    ) -> Result<Response<GetWorkspaceSyncUrlsResponse>, Status> {
        let _claims = self.verify_auth(&request)?;
        let req = request.into_inner();

        let Some(ref s3_client) = self.s3 else {
            return Err(Status::failed_precondition("S3 not configured"));
        };
        let Some(ref bucket) = self.s3_bucket else {
            return Err(Status::failed_precondition("S3 bucket not configured"));
        };

        let key = workspace_key(&self.workspace_s3_prefix, &req.task_id);

        let put_url = file_s3::presign_put(s3_client, bucket, &key, WORKSPACE_PRESIGN_TTL)
            .await
            .map_err(|e| {
                warn!(error = %e, task_id = %req.task_id, "presign PUT for workspace failed");
                Status::internal("Failed to presign workspace upload URL")
            })?;
        let get_url = file_s3::presign_get(s3_client, bucket, &key, WORKSPACE_PRESIGN_TTL)
            .await
            .map_err(|e| {
                warn!(error = %e, task_id = %req.task_id, "presign GET for workspace failed");
                Status::internal("Failed to presign workspace download URL")
            })?;

        debug!(task_id = %req.task_id, key = %key, "Minted workspace sync URLs");
        Ok(Response::new(GetWorkspaceSyncUrlsResponse {
            put_url,
            get_url,
            expires_in_secs: WORKSPACE_PRESIGN_TTL.as_secs() as i64,
        }))
    }

    /// Record a workspace-sync outcome reported by the sandbox as Prometheus
    /// metrics. The `s3` backend transfers directly to S3, so the backend never
    /// sees the bytes and must be told the size/duration/outcome to build the
    /// EBS-vs-S3 comparison. Best-effort: it only touches metrics, never fails
    /// the sandbox's sync. Auth: task-scoped JWT via gRPC metadata.
    async fn report_workspace_sync(
        &self,
        request: Request<ReportWorkspaceSyncRequest>,
    ) -> Result<Response<ReportWorkspaceSyncResponse>, Status> {
        let _claims = self.verify_auth(&request)?;
        let req = request.into_inner();

        crate::sandbox::metrics::record_workspace_sync(
            &req.direction,
            req.bytes.max(0) as u64,
            std::time::Duration::from_millis(req.duration_ms.max(0) as u64),
            req.ok,
        );

        debug!(
            task_id = %req.task_id,
            direction = %req.direction,
            bytes = req.bytes,
            duration_ms = req.duration_ms,
            ok = req.ok,
            "Recorded workspace sync report"
        );
        Ok(Response::new(ReportWorkspaceSyncResponse {}))
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

        // Load the task once: gate Closed tasks (terminal — no replay, and don't
        // burn a recovery attempt) and reuse the row for `agent_type` below.
        // Placed before `find_interrupted_turn` / `increment_recovery_attempts`.
        // Fail open on unknown (None / DB-error): only a confirmed-Closed task
        // is gated.
        let task = if let Some(ref tasks) = self.tasks {
            tasks.find_by_id_internal(&req.task_id).await.ok().flatten()
        } else {
            None
        };
        if matches!(
            task.as_ref().map(|t| t.status),
            Some(crate::task::models::TaskStatus::Closed)
        ) {
            debug!(task_id = %req.task_id, "recover_interrupted_run: task is Closed, skipping recovery");
            return Ok(Response::new(RecoverInterruptedRunResponse { turn: None }));
        }

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

        let task_agent_type = task.as_ref().map(|t| t.agent_type.0.clone());

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

/// Build the per-message header rendered into the first text block of a
/// pending user message.
///
/// Two shapes, switched on the message's origin:
///
/// **External** (`external_event_id` is set — message came in over an
/// adapter):
/// ```text
/// <message_id>msg_abc</message_id>
/// <source>slack</source>
///
/// ```
/// `<source>` carries the raw `external_adapter` string (`slack`, `telegram`,
/// `linear`, `github_pr`). The agent passes `<message_id>` to
/// `moat messages reply --message <id>` to reply on the original channel.
///
/// **Internal** (no `external_event_id`):
/// ```text
/// <sent_by>alice</sent_by>
///
/// ```
/// `<sent_by>` names the moatless user who initiated the message. There's no
/// external channel to call back to — the agent just emits a normal assistant
/// reply.
///
/// Returns an empty string when there's nothing to emit (internal row with no
/// resolvable `sent_by`). Callers must guard against the empty case before
/// prepending.
/// Resolve the model to deliver for a pending message.
///
/// An explicit per-message model is always preserved. When the message carries
/// no model, agents that pin a model (e.g. `claude-code-fable` → `fable`) get it
/// forced here, since the catalog `default_model` is only a UI hint and is not
/// otherwise applied at runtime.
fn effective_model(row_model: Option<String>, agent_type: &str) -> Option<String> {
    row_model.or_else(|| match agent_type {
        "claude-code-fable" => Some("fable".to_string()),
        _ => None,
    })
}

fn build_message_header(
    uuid: &str,
    external_event_id: Option<&str>,
    external_adapter: Option<&str>,
    subject_key: Option<&str>,
    sent_by: Option<&str>,
    login_cache: &std::collections::HashMap<String, String>,
) -> String {
    use std::fmt::Write;
    if external_event_id.is_some() {
        let mut header = format!("<message_id>{uuid}</message_id>\n");
        if let Some(source) = external_adapter {
            // Infallible: writing to String never errors.
            let _ = writeln!(header, "<source>{source}</source>");
        }
        // Stable per-thread identity so the agent can tell concurrent threads
        // apart and recognise continuations within the same ongoing task.
        if let Some(thread) = subject_key {
            let _ = writeln!(header, "<thread>{thread}</thread>");
        }
        header.push('\n');
        return header;
    }

    if let Some(uid) = sent_by {
        if let Some(login) = login_cache.get(uid) {
            return format!("<sent_by>{login}</sent_by>\n\n");
        }
    }
    String::new()
}

/// Prepend `header` to the first text block in `content`, synthesising one
/// when none exists. Image/tool-result blocks are left untouched.
fn prepend_header(content: &mut Vec<sandbox_proto::ContentBlock>, header: &str) {
    for block in content.iter_mut() {
        if let Some(sandbox_proto::content_block::Block::Text(ref mut text_block)) = block.block {
            text_block.text = format!("{header}{}", text_block.text);
            return;
        }
    }
    // No text block in the message — synthesise one at the front carrying just
    // the header so the agent still sees the message id / sender.
    let head = sandbox_proto::ContentBlock {
        block: Some(sandbox_proto::content_block::Block::Text(
            sandbox_proto::TextBlock {
                text: header.trim_end().to_string(),
            },
        )),
    };
    content.insert(0, head);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn text_block(text: &str) -> sandbox_proto::ContentBlock {
        sandbox_proto::ContentBlock {
            block: Some(sandbox_proto::content_block::Block::Text(
                sandbox_proto::TextBlock {
                    text: text.to_string(),
                },
            )),
        }
    }

    fn extract_text(block: &sandbox_proto::ContentBlock) -> &str {
        match block.block {
            Some(sandbox_proto::content_block::Block::Text(ref t)) => &t.text,
            _ => panic!("expected text block"),
        }
    }

    #[test]
    fn effective_model_forces_fable_when_unset() {
        // Fable agent with no per-message model is pinned to "fable".
        assert_eq!(
            effective_model(None, "claude-code-fable"),
            Some("fable".to_string())
        );
    }

    #[test]
    fn effective_model_preserves_explicit_model() {
        // An explicit per-message model always wins, even for the fable agent.
        assert_eq!(
            effective_model(Some("claude-opus-4-6".to_string()), "claude-code-fable"),
            Some("claude-opus-4-6".to_string())
        );
    }

    #[test]
    fn effective_model_leaves_other_agents_unset() {
        // Non-pinned agents keep their None (falls back to deployment default).
        assert_eq!(effective_model(None, "claude-code"), None);
        assert_eq!(effective_model(None, "codex"), None);
    }

    #[test]
    fn external_row_emits_message_id_and_source_only() {
        // External rows (external_event_id is set) get <message_id> + <source>
        // and no <sent_by>, even when sent_by would otherwise resolve.
        let mut cache = HashMap::new();
        cache.insert("u1".to_string(), "alice".to_string());
        let header = build_message_header(
            "msg_abc",
            Some("slack:ts:1700"),
            Some("slack"),
            None,
            Some("u1"),
            &cache,
        );
        assert_eq!(
            header,
            "<message_id>msg_abc</message_id>\n<source>slack</source>\n\n"
        );
    }

    #[test]
    fn external_row_with_subject_key_emits_thread_tag() {
        // When the inbound row carries its thread's subject_key, the header
        // surfaces it as a stable <thread> id so the agent can distinguish
        // concurrent threads on the same ongoing task.
        let cache = HashMap::new();
        let header = build_message_header(
            "msg_abc",
            Some("slack:ts:1700"),
            Some("slack"),
            Some("T06:C0A:1781593153.718929"),
            None,
            &cache,
        );
        assert_eq!(
            header,
            "<message_id>msg_abc</message_id>\n<source>slack</source>\n\
             <thread>T06:C0A:1781593153.718929</thread>\n\n"
        );
    }

    #[test]
    fn external_row_without_adapter_drops_source_tag() {
        let cache = HashMap::new();
        let header =
            build_message_header("msg_abc", Some("ext-id"), None, None, None, &cache);
        assert_eq!(header, "<message_id>msg_abc</message_id>\n\n");
    }

    #[test]
    fn internal_row_with_resolved_sent_by_emits_sent_by_only() {
        let mut cache = HashMap::new();
        cache.insert("u1".to_string(), "alice".to_string());
        let header = build_message_header("msg_abc", None, None, None, Some("u1"), &cache);
        assert_eq!(header, "<sent_by>alice</sent_by>\n\n");
    }

    #[test]
    fn internal_row_without_resolvable_sent_by_emits_nothing() {
        let cache = HashMap::new();
        let header = build_message_header("msg_abc", None, None, None, Some("u-missing"), &cache);
        assert_eq!(header, "");

        let header = build_message_header("msg_abc", None, None, None, None, &cache);
        assert_eq!(header, "");
    }

    #[test]
    fn prepend_header_targets_first_text_block() {
        let mut content = vec![text_block("hello")];
        prepend_header(&mut content, "<message_id>msg_1</message_id>\n<source>slack</source>\n\n");
        assert_eq!(
            extract_text(&content[0]),
            "<message_id>msg_1</message_id>\n<source>slack</source>\n\nhello"
        );
    }

    #[test]
    fn prepend_header_synthesises_text_block_when_none_exists() {
        let mut content = vec![sandbox_proto::ContentBlock {
            block: Some(sandbox_proto::content_block::Block::Image(
                sandbox_proto::ImageBlock {
                    url: "https://example/img".to_string(),
                    media_type: "image/png".to_string(),
                },
            )),
        }];
        prepend_header(&mut content, "<message_id>msg_1</message_id>\n<source>slack</source>\n\n");
        assert_eq!(content.len(), 2);
        assert_eq!(
            extract_text(&content[0]),
            "<message_id>msg_1</message_id>\n<source>slack</source>"
        );
    }
}
