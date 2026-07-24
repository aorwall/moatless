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
    ReportWorkspaceSyncResponse, TranscriptChunk, UpdateCredentialRequest,
    UpdateCredentialResponse, UploadTranscriptResponse,
};

use moatless_backend_core::scope::ScopeContext;

use crate::access_control;
use crate::agent::prompt;
use crate::auth::github_app_registry::GitHubAppRegistry;
use crate::auth::github_app_service::GitHubAppService;
use crate::auth::models::UserRole;
use crate::auth::provider_token_dao::ProviderTokenRepository;
use crate::auth::provider_tokens::ProviderTokenAuthRepository;
use crate::auth::users::UserRepository;
use crate::config::Config;
use crate::credentials::{CredentialService, WriteSource};
use crate::events::EventPublisher;
use crate::file_storage::dao::FileRepository;
use crate::file_storage::s3 as file_s3;
use crate::message::dao::MessageRepository;
use crate::repository::dao::RepoRepository;
use crate::sandbox::jwt;
use crate::sandbox::secrets::{
    resolve_server_env, resolve_task_credentials, resolve_task_repository, resolve_task_ssh_keys,
    resolve_task_team_id, TaskCredentialInputs,
};
use crate::secret::service::SecretService;
use crate::shared_models::AgentType;
use crate::task::dao::TaskRepository;
use crate::task::relations::TaskRelationRepository;
use crate::task_event::dao::{InsertTaskEvent, TaskEventRepository};
use crate::task_turn::dao::{TaskTurnRepository, TurnStateEvent};
use crate::task_turn::models::classify_turn_transition;
use crate::team::dao::TeamRepository;
use crate::workspace::dao::WorkspaceRepository;

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
fn sandbox_key(prefix: &str, task_id: &str) -> String {
    format!("{}/{}/workspace.tar", prefix.trim_end_matches('/'), task_id)
}

/// Lifetime of the presigned workspace-sync URLs. The sandbox re-mints them on
/// every sync tick, so this only needs to cover a single transfer. Kept short to
/// bound the blast radius of a leaked URL, but generous enough for a large tar
/// on a slow link.
const WORKSPACE_PRESIGN_TTL: std::time::Duration = std::time::Duration::from_mins(15);

/// gRPC server state — holds event publisher, repository trait objects, and JWT secret.
pub struct BackendSandboxServiceImpl {
    pub events: Option<Arc<dyn EventPublisher>>,
    pub messages: Option<Arc<dyn MessageRepository>>,
    pub provider_tokens: Option<Arc<dyn ProviderTokenRepository>>,
    pub tasks: Option<Arc<dyn TaskRepository>>,
    /// Team membership resolver. Used by `assert_task_access` to authorize
    /// team-scoped tasks (the sandbox JWT carries no team membership, so this
    /// requires a DB lookup).
    pub teams: Option<Arc<dyn TeamRepository>>,
    /// Authoritative turn-state writer. State-bearing events commit their turn /
    /// task-status / event rows here transactionally before being published to
    /// Redis (transactional-outbox ordering).
    pub task_turns: Option<Arc<dyn TaskTurnRepository>>,
    /// Event-row writer for events that bear no turn state (`agent.message` and
    /// friends). Commits before the Redis publish so Postgres holds the full
    /// payload even when the live tail carries a truncated stub.
    pub task_events: Option<Arc<dyn TaskEventRepository>>,
    pub task_relations: Option<Arc<dyn TaskRelationRepository>>,
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
    /// Repository DAO — used to resolve a task's repo owner for credentials.
    pub repositories: Option<Arc<dyn RepoRepository>>,
    /// Workspace DAO — used to load the workspace's `system_prompt` and `servers`
    /// when assembling the agent's append system prompt.
    pub workspaces: Option<Arc<dyn WorkspaceRepository>>,
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

    /// Assert that the authenticated caller may operate on `task_id`, returning
    /// the loaded task on success.
    ///
    /// The sandbox JWT authenticates *who* is calling (`claims.user_id` — the
    /// task's creator, minted at provision time) but carries no task binding —
    /// `task_id` arrives in the request body and is attacker-controllable.
    /// Without this check a sandbox provisioned for one task could pass a
    /// different task's id to read its credentials, pending messages, or
    /// transcripts (cross-tenant escalation). Every RPC that acts on a
    /// caller-supplied `task_id` must gate on this.
    ///
    /// The check is the HTTP task API's exact two-step: hydrate the caller's
    /// `ScopeContext`, load the task through the scoped DAO read (an invisible
    /// task is a plain NotFound, like the HTTP 404), then apply the shared
    /// `can_control_task` policy (admin / owner / managing team role). Public
    /// *visibility* grants viewing over HTTP but never sandbox control —
    /// credentials, messages, and workspace writes stay owner/manager-only.
    async fn assert_task_access(
        &self,
        claims: &jwt::UserTokenClaims,
        task_id: &str,
    ) -> Result<crate::task::models::Task, Status> {
        let tasks = self
            .tasks
            .as_ref()
            .ok_or_else(|| Status::internal("Tasks repository not configured"))?;
        let teams = self
            .teams
            .as_ref()
            .ok_or_else(|| Status::internal("Teams repository not configured"))?;

        let ctx = self.scope_context_for(claims).await?;
        let task = tasks
            .find_by_id(task_id, &ctx)
            .await
            .map_err(|e| {
                warn!(error = %e, task_id, "assert_task_access: task lookup failed");
                Status::internal("Failed to load task")
            })?
            .ok_or_else(|| Status::not_found("Task not found"))?;

        let allowed =
            access_control::can_control_task_ctx(teams.as_ref(), &ctx, &task.scoped_resource())
                .await
                .map_err(|e| {
                    warn!(error = %e, task_id, "assert_task_access: team role lookup failed");
                    Status::internal("Failed to check task access")
                })?;

        if allowed {
            Ok(task)
        } else {
            warn!(
                task_id,
                user_id = %claims.user_id,
                scope = %task.scope,
                "Rejected sandbox gRPC call: caller not authorized for task"
            );
            Err(Status::permission_denied("Not authorized for this task"))
        }
    }

    /// Hydrate the caller's `ScopeContext` from the DB. The sandbox JWT carries
    /// only `user_id`/`login`, so team membership and platform role need
    /// lookups — the HTTP side gets the same context for free from the auth
    /// middleware's `AuthIdentity`.
    async fn scope_context_for(
        &self,
        claims: &jwt::UserTokenClaims,
    ) -> Result<ScopeContext, Status> {
        let user_uuid = claims
            .user_id
            .parse::<uuid::Uuid>()
            .map_err(|_| Status::unauthenticated("Invalid user id in token"))?;
        let team_ids = match self.teams.as_ref() {
            Some(teams) => teams
                .find_team_ids_for_user(&user_uuid)
                .await
                .map_err(|e| {
                    warn!(error = %e, user_id = %claims.user_id, "team membership lookup failed");
                    Status::internal("Failed to resolve caller teams")
                })?,
            None => Vec::new(),
        };
        Ok(ScopeContext {
            user_id: claims.user_id.clone(),
            team_ids,
            is_admin: self.user_is_admin(&user_uuid).await,
        })
    }

    /// Platform-admin check for `assert_task_access`. Missing repo or a lookup
    /// error is treated as "not admin" (fail closed).
    async fn user_is_admin(&self, user_id: &uuid::Uuid) -> bool {
        match self.users.as_ref() {
            Some(users) => {
                matches!(users.get_user(user_id).await, Ok(Some(u)) if u.role == UserRole::Admin)
            }
            None => false,
        }
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
    /// The turn an inbound event belongs to.
    ///
    /// An event published from inside an agent run carries that run's id, which
    /// resolves to its turn — allocating it if this is the run's first signal
    /// to arrive. That is what makes attribution exact: a terminal event
    /// delayed in the sandbox outbox past the start of the next run still
    /// closes its own turn rather than whichever turn is open on arrival.
    ///
    /// Events published outside a run carry no id and bear no turn state; they
    /// fall back to the task's latest turn.
    async fn resolve_turn_for_event(&self, task_id: &str, run_id: &str) -> Option<i32> {
        let turns = self.task_turns.as_ref()?;
        if run_id.is_empty() {
            return None;
        }
        match turns.resolve_turn_for_run(task_id, run_id).await {
            Ok(turn_number) => Some(turn_number),
            Err(e) => {
                warn!(
                    task_id = %task_id,
                    run_id = %run_id,
                    error = %e,
                    "Failed to resolve turn for run"
                );
                None
            }
        }
    }

    /// The task's latest turn — the fallback for a state-bearing event that
    /// reached the ingress without a run id.
    async fn latest_turn(&self, task_id: &str) -> Option<i32> {
        self.task_turns
            .as_ref()?
            .latest_turn_number(task_id)
            .await
            .unwrap_or_else(|e| {
                warn!(task_id = %task_id, error = %e, "Failed to read latest turn number");
                None
            })
    }

    /// The task's configured agent type, for stamping onto turn rows.
    async fn resolve_agent_type(&self, task_id: &str) -> Option<String> {
        self.tasks
            .as_ref()?
            .find_by_id_internal(task_id)
            .await
            .ok()
            .flatten()
            .map(|task| task.agent_type.0)
    }
}

/// Transactional-outbox write: commit the `task_events` row — plus turn state
/// when the event bears any — then publish to Redis. The DB commit is always
/// awaited *before* the publish, so an event is never fanned out ahead of its
/// committed state. A failed commit is logged and the publish still happens —
/// the Redis consumer re-applies the same row and state idempotently as a
/// fallback.
///
/// Committing on *both* branches is what lets the publisher treat Redis as a
/// disposable live tail and truncate oversized payloads: Postgres always holds
/// the full event first.
#[allow(clippy::too_many_arguments)]
async fn write_state_then_publish(
    task_turns: Option<&Arc<dyn TaskTurnRepository>>,
    task_events: Option<&Arc<dyn TaskEventRepository>>,
    publisher: &Arc<dyn EventPublisher>,
    event_id: &str,
    task_id: &str,
    event_type: &str,
    sub_type: &str,
    turn_number: Option<i32>,
    payload: Value,
    state: Option<TurnStateEvent>,
) {
    let is_agent_message = crate::event_handler::skeleton::is_agent_message(event_type, sub_type);

    // Tracks whether `task_events` actually holds the full payload. Only then
    // may the publisher truncate an oversized one; on a failed commit the
    // consumer's fallback insert is the last route to Postgres and must see
    // the whole thing. `agent.message` is persisted as a skeleton, so its full
    // payload is never durable in `task_events` — see the `None` arm below.
    let payload_persisted = match state {
        // State-bearing: event row, turn projection, and task-status flip all
        // commit in one transaction.
        Some(ev) => match task_turns {
            Some(turns) => match turns.apply_ingress_event(&ev).await {
                Ok(()) => true,
                Err(e) => {
                    warn!(
                        task_id = %task_id,
                        event_id = %event_id,
                        error = %e,
                        "Transactional turn-state write failed; falling back to consumer"
                    );
                    false
                }
            },
            None => false,
        },
        // Everything else still needs its row committed before the fan-out.
        // `agent.message` lands here, and it carries the largest payloads — so it
        // is persisted as a metadata skeleton (ADR-0003). Its full content stays
        // on the Redis stream for the ingestion consumer to build the Message
        // store; `task_events` keeps only the skeleton, so the publisher must
        // NOT truncate the Redis payload (there is no full copy to fall back to).
        None => match task_events {
            Some(events) => {
                let stored_payload = if is_agent_message {
                    crate::event_handler::skeleton::agent_message_skeleton(&payload)
                } else {
                    payload.clone()
                };
                let insert = InsertTaskEvent {
                    event_id: event_id.to_string(),
                    task_id: task_id.to_string(),
                    event_type: event_type.to_string(),
                    sub_type: sub_type.to_string(),
                    is_error: sub_type == "error" || sub_type == "failed",
                    agent_type: None,
                    turn_number,
                    payload: stored_payload,
                    timestamp: chrono::Utc::now()
                        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                };
                match events.insert_event(&insert).await {
                    // `agent.message`: task_events holds only the skeleton, so the
                    // Redis payload is not durable-elsewhere — keep it whole.
                    Ok(()) => !is_agent_message,
                    Err(e) => {
                        warn!(
                            task_id = %task_id,
                            event_id = %event_id,
                            error = %e,
                            "Ingress event-row write failed; falling back to consumer"
                        );
                        false
                    }
                }
            }
            None => false,
        },
    };
    publisher
        .publish_raw_event_with_id(
            event_id,
            task_id,
            event_type,
            sub_type,
            None,
            turn_number,
            payload,
            payload_persisted,
        )
        .await;
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

        // Use the sandbox's stable event_id so outbox retries dedupe on a
        // consistent id (ingress gate above + consumer's `ON CONFLICT
        // (event_id) DO NOTHING`), and so a transactionally-written
        // `task_events` row and the published Redis event share one id.
        // Fall back to a fresh id for legacy senders that predate the
        // outbox and don't set one.
        let event_id = if event.event_id.is_empty() {
            uuid::Uuid::new_v4().to_string()
        } else {
            event.event_id.clone()
        };

        // Resolve the turn for every event published from inside an agent run,
        // not just the state-bearing ones: the Redis consumer then receives an
        // explicit turn number on `agent.message` too and never has to guess.
        // Events from outside a run (sandbox/init/heartbeat) bear no turn.
        let mut turn_number = self
            .resolve_turn_for_event(&event.task_id, &event.run_id)
            .await;

        // State-bearing events (taskTurn.*, agent.completed/error/stopped) commit
        // their turn / task-status / event rows in ONE transaction BEFORE the
        // Redis fan-out. Everything else just publishes.
        let transition = classify_turn_transition(event_type_str, &event.sub_type);
        if transition.is_some() && turn_number.is_none() {
            // A state-bearing event that arrived without a run id — a publisher
            // that doesn't front the actor. Attribute it to the open turn.
            turn_number = self.latest_turn(&event.task_id).await;
        }
        let state = if let (Some(transition), Some(turn_number)) = (transition, turn_number) {
            let agent_type = self.resolve_agent_type(&event.task_id).await;
            Some(TurnStateEvent {
                event_id: event_id.clone(),
                task_id: event.task_id.clone(),
                event_type: event_type_str.to_string(),
                sub_type: event.sub_type.clone(),
                is_error: event.sub_type == "error" || event.sub_type == "failed",
                agent_type,
                turn_number,
                payload: payload.clone(),
                timestamp: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                transition,
            })
        } else {
            None
        };

        write_state_then_publish(
            self.task_turns.as_ref(),
            self.task_events.as_ref(),
            publisher,
            &event_id,
            &event.task_id,
            event_type_str,
            &event.sub_type,
            turn_number,
            payload,
            state,
        )
        .await;

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
        let claims = if self.task_token_secret.is_some() {
            let claims = self.verify_auth(&request)?;
            tracing::Span::current().record("user_id", claims.user_id.as_str());
            Some(claims)
        } else {
            None
        };

        let req = request.into_inner();
        tracing::Span::current().record("task_id", req.task_id.as_str());

        // Cross-tenant guard: the authenticated caller must own/have access to
        // the requested task before we hand back its queued messages.
        if let Some(ref claims) = claims {
            self.assert_task_access(claims, &req.task_id).await?;
        }

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
        let pending = messages.get_pending_user(&req.task_id).await.map_err(|e| {
            warn!(error = %e, task_id = %req.task_id, "Failed to fetch pending messages");
            Status::internal("Failed to fetch pending messages")
        })?;

        // Look up latest agent_session_id for --resume <id>
        let task_resume_session_id = messages
            .get_latest_agent_session_id(&req.task_id)
            .await
            .unwrap_or(None);

        // Check will_not_resume on the last result — if explicitly set,
        // don't pass a resume session ID. Also
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
                        let Ok(uuid) = uid.parse::<uuid::Uuid>() else {
                            continue;
                        };
                        match users.get_user(&uuid).await {
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
        let claims = if self.task_token_secret.is_some() {
            Some(self.verify_auth(&request)?)
        } else {
            None
        };

        let req = request.into_inner();

        // Cross-tenant guard: only the task's owner/members may acknowledge
        // (mark sent) its queued messages.
        if let Some(ref claims) = claims {
            self.assert_task_access(claims, &req.task_id).await?;
        }

        let Some(ref messages) = self.messages else {
            return Ok(Response::new(AcknowledgeMessagesResponse {}));
        };

        // The acknowledging run decides the turn: whichever of the ack and the
        // run's `taskTurn.active` reaches the backend first allocates it, the
        // other joins. Messages injected into a run in flight ack with that
        // run's id and so share its turn.
        let Some(turn_number) = self.resolve_turn_for_event(&req.task_id, &req.run_id).await else {
            warn!(
                task_id = %req.task_id,
                run_id = %req.run_id,
                "Cannot acknowledge messages without a resolvable turn"
            );
            return Err(Status::internal("Failed to resolve turn for run"));
        };

        match messages
            .acknowledge_messages(
                &req.task_id,
                &req.message_uuids,
                &req.session_id,
                turn_number,
            )
            .await
        {
            Ok(count) => {
                info!(
                    task_id = %req.task_id,
                    session_id = %req.session_id,
                    run_id = %req.run_id,
                    turn_number,
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

        // Cross-tenant guard: the caller must own/have access to this task
        // before we resolve and return its (team/user) provider credentials.
        // Without this a sandbox could request another team's tokens by id.
        self.assert_task_access(&claims, &req.task_id).await?;

        let team_id = resolve_task_team_id(self.tasks.as_ref(), &req.task_id).await;
        let repository = resolve_task_repository(
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
            repository: repository.as_ref(),
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

        // Cross-tenant guard: the caller must own/have access to this task
        // before we persist a rotated credential against its team/user row.
        self.assert_task_access(&claims, &req.task_id).await?;

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
        if req.key == crate::sandbox::secrets::GITHUB_TOKEN && team_id.is_some() {
            return Err(Status::invalid_argument(
                "Team GitHub credentials cannot be updated from a sandbox",
            ));
        }

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

        let result: Result<(), Box<dyn std::error::Error + Send + Sync>> = if team_row_exists {
            // SAFETY: team_id checked above for `team_row_exists` to be true.
            let tid = team_id
                .as_deref()
                .ok_or_else(|| Status::internal("team_id missing after team_row_exists check"))?;
            provider_tokens
                .upsert_team_token(tid, provider, "manual", &req.value, None, None, None)
                .await
                .map_err(Into::into)
        } else if let Some(ref svc) = self.credentials_svc {
            // Routes GITHUB_TOKEN into the encrypted store instead of writing
            // it plaintext to `provider_tokens.access_token`.
            svc.store(
                &claims.user_id,
                &req.key,
                &req.value,
                WriteSource::SandboxRotation {
                    task_id: req.task_id.clone(),
                },
            )
            .await
            .map_err(Into::into)
        } else if req.key == crate::sandbox::secrets::GITHUB_TOKEN {
            // No credential service means no encrypted store; refuse rather
            // than resurrect a plaintext GitHub token.
            warn!(
                task_id = %req.task_id,
                user_id = %claims.user_id,
                "UpdateCredential rejected: cannot persist a GitHub token without the credential service"
            );
            return Err(Status::failed_precondition(
                "Cannot persist a GitHub token without the credential service",
            ));
        } else {
            provider_tokens
                .upsert_token(&claims.user_id, provider, &req.value, None, None, None)
                .await
                .map_err(Into::into)
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
    /// reads `Task.name`, `Task.append_system_prompt`, the workspace's
    /// `system_prompt` + `servers`, and the configured frontend/proxy URLs.
    async fn get_system_prompt(
        &self,
        request: Request<GetSystemPromptRequest>,
    ) -> Result<Response<GetSystemPromptResponse>, Status> {
        let claims = self.verify_auth(&request)?;
        let req = request.into_inner();

        // Cross-tenant guard: the assembled system prompt embeds task/repo
        // details, so only the task's owner/members may fetch it.
        let task = Some(self.assert_task_access(&claims, &req.task_id).await?);

        let system_prompt = prompt::build_for_task(
            &req.task_id,
            task.as_ref(),
            self.workspaces.as_ref(),
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
        let claims = self.verify_auth(&request)?;
        let req = request.into_inner();
        self.assert_task_access(&claims, &req.task_id).await?;

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
        let claims = self.verify_auth(&request)?;

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

        // The task_id arrives in-band from the stream, so authorize only once it
        // is known — a valid JWT must still own the task it is writing to.
        self.assert_task_access(&claims, &task_id).await?;

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
        let claims = self.verify_auth(&request)?;
        let req = request.into_inner();
        self.assert_task_access(&claims, &req.task_id).await?;

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
        let claims = self.verify_auth(&request)?;
        let req = request.into_inner();
        self.assert_task_access(&claims, &req.task_id).await?;

        let Some(ref s3_client) = self.s3 else {
            return Err(Status::failed_precondition("S3 not configured"));
        };
        let Some(ref bucket) = self.s3_bucket else {
            return Err(Status::failed_precondition("S3 bucket not configured"));
        };

        let key = sandbox_key(&self.workspace_s3_prefix, &req.task_id);

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
        let claims = if self.task_token_secret.is_some() {
            Some(self.verify_auth(&request)?)
        } else {
            None
        };

        let req = request.into_inner();

        // Cross-tenant guard: recovery replays another turn's messages and burns
        // a recovery attempt on the task, so gate it on task access.
        if let Some(ref claims) = claims {
            self.assert_task_access(claims, &req.task_id).await?;
        }

        let Some(ref messages) = self.messages else {
            return Ok(Response::new(RecoverInterruptedRunResponse { turn: None }));
        };

        // Load the task once: gate Closed tasks and tasks that no longer desire
        // a running sandbox (terminal/intentional stop — no replay, and don't
        // burn a recovery attempt) and reuse the row for `agent_type` below.
        // Placed before `find_interrupted_turn` / `increment_recovery_attempts`.
        // Fail open on unknown (None / DB-error): only confirmed terminal or
        // stopped intent is gated.
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
        // Desired state lives in the `sandboxes` row now. No row (never
        // requested) reads as None here — fail open and proceed, matching the
        // old default; only an explicit Stopped/Removed intent gates recovery.
        let sandbox_desired_state = if let Some(ref tasks) = self.tasks {
            tasks
                .get_sandbox_intents(std::slice::from_ref(&req.task_id))
                .await
                .ok()
                .and_then(|mut m| m.remove(&req.task_id))
                .map(|v| v.desired_state)
        } else {
            None
        };
        if matches!(
            sandbox_desired_state,
            Some(crate::task::models::SandboxDesiredState::Stopped)
                | Some(crate::task::models::SandboxDesiredState::Removed)
        ) {
            debug!(
                task_id = %req.task_id,
                "recover_interrupted_run: task does not desire a running sandbox, skipping recovery"
            );
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

        let Some(ref task_turns) = self.task_turns else {
            warn!(task_id = %req.task_id, "Cannot record a recovery attempt without task_turns");
            return Ok(Response::new(RecoverInterruptedRunResponse { turn: None }));
        };
        let new_attempts = match task_turns
            .increment_recovery_attempts(&req.task_id, interrupted.turn_number)
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
        let header = build_message_header("msg_abc", Some("ext-id"), None, None, None, &cache);
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
        prepend_header(
            &mut content,
            "<message_id>msg_1</message_id>\n<source>slack</source>\n\n",
        );
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
        prepend_header(
            &mut content,
            "<message_id>msg_1</message_id>\n<source>slack</source>\n\n",
        );
        assert_eq!(content.len(), 2);
        assert_eq!(
            extract_text(&content[0]),
            "<message_id>msg_1</message_id>\n<source>slack</source>"
        );
    }

    // =========================================================================
    // write_state_then_publish — transactional-outbox ordering
    // =========================================================================

    use std::sync::Mutex as StdMutex;

    use async_trait::async_trait;
    use chrono::{DateTime, Utc};
    use moatless_events::{
        SandboxEventPayload, SandboxStatus, ServerEventPayload, ServerSubType, TaskSubType,
        TaskTurnSubType,
    };

    use crate::task_turn::dao::TaskTurnRepository;
    use crate::task_turn::models::{TurnStatus, TurnTransition, UnclosedTurn};

    /// Shared ordered log of side effects, so we can assert commit-before-publish.
    type Recorder = Arc<StdMutex<Vec<&'static str>>>;

    /// Turn repo double that records a `"commit"` on `apply_ingress_event`, then
    /// optionally fails to exercise the publish-still-happens fallback.
    struct RecordingTurns {
        rec: Recorder,
        fail: bool,
    }

    #[async_trait]
    impl TaskTurnRepository for RecordingTurns {
        async fn apply_ingress_event(&self, _ev: &TurnStateEvent) -> Result<(), sqlx::Error> {
            self.rec.lock().expect("recorder poisoned").push("commit");
            if self.fail {
                return Err(sqlx::Error::PoolClosed);
            }
            Ok(())
        }

        async fn project_turn(
            &self,
            _task_id: &str,
            _turn_number: i32,
            _agent_type: Option<String>,
            _transition: TurnTransition,
            _at: DateTime<Utc>,
        ) -> Result<(), sqlx::Error> {
            unreachable!("project_turn is the consumer fallback, not the ingress path")
        }

        async fn list_unclosed_active_turns(
            &self,
            _older_than: DateTime<Utc>,
        ) -> Result<Vec<UnclosedTurn>, sqlx::Error> {
            unreachable!("list_unclosed_active_turns is the reaper path")
        }

        async fn resolve_turn_for_run(
            &self,
            _task_id: &str,
            _run_id: &str,
        ) -> Result<i32, sqlx::Error> {
            Ok(1)
        }

        async fn latest_turn_number(&self, _task_id: &str) -> Result<Option<i32>, sqlx::Error> {
            Ok(Some(1))
        }

        async fn set_turn_metrics(
            &self,
            _task_id: &str,
            _turn_number: i32,
            _metrics: &serde_json::Value,
        ) -> Result<bool, sqlx::Error> {
            unreachable!("set_turn_metrics is the completion path, not the ingress path")
        }

        async fn list_turn_metrics(
            &self,
            _task_id: &str,
        ) -> Result<Vec<serde_json::Value>, sqlx::Error> {
            unreachable!("list_turn_metrics is the statistics path, not the ingress path")
        }

        async fn increment_recovery_attempts(
            &self,
            _task_id: &str,
            _turn_number: i32,
        ) -> Result<i32, sqlx::Error> {
            unreachable!("increment_recovery_attempts is the recovery path")
        }
    }

    /// Publisher double that records a `"publish"` on `publish_raw_event_with_id`.
    /// Every other trait method is an unused no-op stub.
    struct RecordingPublisher {
        rec: Recorder,
    }

    #[async_trait]
    impl EventPublisher for RecordingPublisher {
        async fn publish_sandbox_event(&self, _: &str, _: SandboxStatus, _: &str) {}
        async fn publish_sandbox_event_with_payload(
            &self,
            _: &str,
            _: SandboxStatus,
            _: &SandboxEventPayload,
        ) {
        }
        async fn publish_server_event(&self, _: &str, _: ServerSubType, _: &ServerEventPayload) {}
        async fn publish_task_event(&self, _: &str, _: TaskSubType, _: Option<&str>, _: Value) {}
        async fn publish_agent_event(&self, _: &str, _: AgentSubType, _: Option<&str>, _: Value) {}
        async fn publish_task_turn_event(
            &self,
            _: &str,
            _: TaskTurnSubType,
            _: Option<&str>,
            _: Option<i32>,
            _: Value,
        ) {
        }
        async fn publish_raw_event(
            &self,
            _: &str,
            _: &str,
            _: &str,
            _: Option<&str>,
            _: Option<i32>,
            _: Value,
        ) {
        }
        async fn publish_raw_event_with_id(
            &self,
            _event_id: &str,
            _task_id: &str,
            _event_type: &str,
            _sub_type: &str,
            _agent_type: Option<&str>,
            _turn_number: Option<i32>,
            _payload: Value,
            _payload_persisted: bool,
        ) {
            self.rec.lock().expect("recorder poisoned").push("publish");
        }
        async fn health_check(&self) -> Result<(), String> {
            Ok(())
        }
    }

    /// Publisher double capturing the `payload_persisted` flag it was handed —
    /// the signal that decides whether an oversized payload may be truncated.
    struct FlagCapturingPublisher {
        seen: Arc<StdMutex<Vec<bool>>>,
    }

    #[async_trait]
    impl EventPublisher for FlagCapturingPublisher {
        async fn publish_sandbox_event(&self, _: &str, _: SandboxStatus, _: &str) {}
        async fn publish_sandbox_event_with_payload(
            &self,
            _: &str,
            _: SandboxStatus,
            _: &SandboxEventPayload,
        ) {
        }
        async fn publish_server_event(&self, _: &str, _: ServerSubType, _: &ServerEventPayload) {}
        async fn publish_task_event(&self, _: &str, _: TaskSubType, _: Option<&str>, _: Value) {}
        async fn publish_agent_event(&self, _: &str, _: AgentSubType, _: Option<&str>, _: Value) {}
        async fn publish_task_turn_event(
            &self,
            _: &str,
            _: TaskTurnSubType,
            _: Option<&str>,
            _: Option<i32>,
            _: Value,
        ) {
        }
        async fn publish_raw_event(
            &self,
            _: &str,
            _: &str,
            _: &str,
            _: Option<&str>,
            _: Option<i32>,
            _: Value,
        ) {
        }
        async fn publish_raw_event_with_id(
            &self,
            _event_id: &str,
            _task_id: &str,
            _event_type: &str,
            _sub_type: &str,
            _agent_type: Option<&str>,
            _turn_number: Option<i32>,
            _payload: Value,
            payload_persisted: bool,
        ) {
            self.seen
                .lock()
                .expect("recorder poisoned")
                .push(payload_persisted);
        }
        async fn health_check(&self) -> Result<(), String> {
            Ok(())
        }
    }

    fn sample_state(event_id: &str) -> TurnStateEvent {
        TurnStateEvent {
            event_id: event_id.to_string(),
            task_id: "task-1".to_string(),
            event_type: "taskTurn".to_string(),
            sub_type: "active".to_string(),
            is_error: false,
            agent_type: Some("claude-code".to_string()),
            turn_number: 1,
            payload: serde_json::json!({}),
            timestamp: "2026-07-12T00:00:00.000Z".to_string(),
            transition: TurnTransition::Open,
        }
    }

    #[tokio::test]
    async fn state_is_committed_before_redis_publish() {
        let rec: Recorder = Arc::new(StdMutex::new(Vec::new()));
        let turns: Arc<dyn TaskTurnRepository> = Arc::new(RecordingTurns {
            rec: rec.clone(),
            fail: false,
        });
        let publisher: Arc<dyn EventPublisher> = Arc::new(RecordingPublisher { rec: rec.clone() });

        write_state_then_publish(
            Some(&turns),
            None,
            &publisher,
            "evt-1",
            "task-1",
            "taskTurn",
            "active",
            Some(1),
            serde_json::json!({}),
            Some(sample_state("evt-1")),
        )
        .await;

        // Transactional-outbox invariant: the DB commit is observed strictly
        // before the Redis fan-out, never the other way round.
        assert_eq!(*rec.lock().unwrap(), vec!["commit", "publish"]);
    }

    #[tokio::test]
    async fn publish_still_happens_when_commit_fails() {
        let rec: Recorder = Arc::new(StdMutex::new(Vec::new()));
        let turns: Arc<dyn TaskTurnRepository> = Arc::new(RecordingTurns {
            rec: rec.clone(),
            fail: true,
        });
        let publisher: Arc<dyn EventPublisher> = Arc::new(RecordingPublisher { rec: rec.clone() });

        write_state_then_publish(
            Some(&turns),
            None,
            &publisher,
            "evt-2",
            "task-1",
            "taskTurn",
            "active",
            Some(1),
            serde_json::json!({}),
            Some(sample_state("evt-2")),
        )
        .await;

        // A failed commit is logged and the publish still runs — the consumer
        // re-applies the projection idempotently as the fallback.
        assert_eq!(*rec.lock().unwrap(), vec!["commit", "publish"]);
    }

    /// Event-row writer double recording an `"event-row"` on `insert_event`.
    fn recording_events(rec: &Recorder, fail: bool) -> Arc<dyn TaskEventRepository> {
        let mut events = crate::task_event::dao::MockTaskEventRepository::new();
        let rec = rec.clone();
        events.expect_insert_event().returning(move |_| {
            rec.lock().expect("recorder poisoned").push("event-row");
            if fail {
                return Err(sqlx::Error::PoolClosed);
            }
            Ok(())
        });
        Arc::new(events)
    }

    #[tokio::test]
    async fn non_state_event_commits_its_row_before_publish() {
        let rec: Recorder = Arc::new(StdMutex::new(Vec::new()));
        let turns: Arc<dyn TaskTurnRepository> = Arc::new(RecordingTurns {
            rec: rec.clone(),
            fail: false,
        });
        let events = recording_events(&rec, false);
        let publisher: Arc<dyn EventPublisher> = Arc::new(RecordingPublisher { rec: rec.clone() });

        // No TurnStateEvent → no turn projection, but the event row must still
        // land before the fan-out so the published payload stays disposable.
        write_state_then_publish(
            Some(&turns),
            Some(&events),
            &publisher,
            "evt-3",
            "task-1",
            "agent",
            "message",
            None,
            serde_json::json!({}),
            None,
        )
        .await;

        assert_eq!(*rec.lock().unwrap(), vec!["event-row", "publish"]);
    }

    #[tokio::test]
    async fn non_state_event_publishes_untruncated_when_row_write_fails() {
        let rec: Recorder = Arc::new(StdMutex::new(Vec::new()));
        let events = recording_events(&rec, true);
        let persisted: Arc<StdMutex<Vec<bool>>> = Arc::new(StdMutex::new(Vec::new()));
        let publisher: Arc<dyn EventPublisher> = Arc::new(FlagCapturingPublisher {
            seen: persisted.clone(),
        });

        write_state_then_publish(
            None,
            Some(&events),
            &publisher,
            "evt-4",
            "task-1",
            "agent",
            "message",
            None,
            serde_json::json!({}),
            None,
        )
        .await;

        // The row never committed, so Redis is the only route to Postgres —
        // the publisher must be told not to truncate.
        assert_eq!(*persisted.lock().unwrap(), vec![false]);
    }

    #[test]
    fn terminal_status_str_roundtrips() {
        // Guards the DB status strings the projection writes.
        assert_eq!(TurnStatus::Completed.as_str(), "completed");
        assert_eq!(TurnStatus::Error.as_str(), "error");
        assert_eq!(TurnStatus::Stopped.as_str(), "stopped");
    }

    // ---------------------------------------------------------------------
    // assert_task_access — cross-tenant authorization gate shared by every
    // task-scoped gRPC handler (credentials, messages, transcripts, uploads,
    // workspace-sync URLs). Mirrors the HTTP task API: scoped read +
    // `can_control_task` (admin / owner / managing team role). The scoped
    // read's SQL filter is covered by `scope.rs` tests; these exercise the
    // control-gate composition on tasks the read returned.
    // ---------------------------------------------------------------------

    use moatless_backend_core::scope::Scope;

    use crate::auth::users::{MockUserRepository, User};
    use crate::task::dao::MockTaskRepository;
    use crate::task::models::{Task, Visibility};
    use crate::team::dao::TeamRepository;
    use crate::team::models::{Team, TeamMember};

    /// Minimal `TeamRepository` stub for one test team (`team-1`): only the
    /// membership/role methods `assert_task_access` consults are meaningful.
    struct StubTeamRepo {
        /// `(user_id, role)` rows, role as stored in the DB (`owner`/`admin`/`member`).
        members: Vec<(uuid::Uuid, String)>,
    }

    impl StubTeamRepo {
        fn with_roles(members: &[(&str, &str)]) -> Self {
            Self {
                members: members
                    .iter()
                    .map(|(u, r)| (uid(u), (*r).to_string()))
                    .collect(),
            }
        }

        fn empty() -> Self {
            Self {
                members: Vec::new(),
            }
        }
    }

    #[async_trait::async_trait]
    impl TeamRepository for StubTeamRepo {
        async fn is_member(
            &self,
            _team_id: &str,
            user_id: &uuid::Uuid,
        ) -> Result<bool, sqlx::Error> {
            Ok(self.members.iter().any(|(u, _)| u == user_id))
        }
        async fn find_members(&self, team_id: &str) -> Result<Vec<TeamMember>, sqlx::Error> {
            Ok(self
                .members
                .iter()
                .map(|(u, r)| TeamMember {
                    team_id: team_id.to_string(),
                    user_id: *u,
                    role: r.clone(),
                    created_at: epoch(),
                })
                .collect())
        }
        async fn find_team_ids_for_user(
            &self,
            user_id: &uuid::Uuid,
        ) -> Result<Vec<String>, sqlx::Error> {
            Ok(self
                .members
                .iter()
                .filter(|(u, _)| u == user_id)
                .map(|_| "team-1".to_string())
                .collect())
        }
        async fn find_by_id(&self, _id: &str) -> Result<Option<Team>, sqlx::Error> {
            unimplemented!()
        }
        async fn find_by_slug(&self, _slug: &str) -> Result<Option<Team>, sqlx::Error> {
            unimplemented!()
        }
        async fn find_by_external_id(
            &self,
            _external_id: &str,
            _provider: &str,
        ) -> Result<Option<Team>, sqlx::Error> {
            unimplemented!()
        }
        async fn list_all(&self) -> Result<Vec<Team>, sqlx::Error> {
            unimplemented!()
        }
        async fn insert(&self, _team: &Team) -> Result<(), sqlx::Error> {
            unimplemented!()
        }
        async fn insert_if_absent(&self, _team: &Team) -> Result<(), sqlx::Error> {
            unimplemented!()
        }
        async fn update(
            &self,
            _id: &str,
            _name: &str,
            _description: Option<&str>,
        ) -> Result<(), sqlx::Error> {
            unimplemented!()
        }
        async fn delete(&self, _id: &str) -> Result<(), sqlx::Error> {
            unimplemented!()
        }
        async fn find_teams_for_user(
            &self,
            _user_id: &uuid::Uuid,
        ) -> Result<Vec<Team>, sqlx::Error> {
            unimplemented!()
        }
        async fn add_member(
            &self,
            _team_id: &str,
            _user_id: &uuid::Uuid,
            _role: &str,
        ) -> Result<(), sqlx::Error> {
            unimplemented!()
        }
        async fn remove_member(
            &self,
            _team_id: &str,
            _user_id: &uuid::Uuid,
        ) -> Result<(), sqlx::Error> {
            unimplemented!()
        }
        async fn upsert_by_external(
            &self,
            _external_id: &str,
            _provider: &str,
            _name: &str,
            _slug: &str,
        ) -> Result<Team, sqlx::Error> {
            unimplemented!()
        }
    }

    fn epoch() -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::from_timestamp(0, 0).expect("epoch is a valid timestamp")
    }

    /// Deterministic uuid for a readable test name: distinct names map to
    /// distinct ids, equal names to equal ids.
    fn uid(name: &str) -> uuid::Uuid {
        uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, name.as_bytes())
    }

    /// A `Task` fixture with the given scope/owner/team; all other columns are
    /// inert defaults irrelevant to the access check.
    fn task_fixture(scope: Scope, owner_user_id: Option<&str>, team_id: Option<&str>) -> Task {
        Task {
            task_id: "task-1".to_string(),
            short_id: None,
            created_at: String::new(),
            last_message_at: String::new(),
            agent_type: crate::shared_models::AgentType("claude-code".to_string()),
            repository_id: "repo-1".to_string(),
            status: crate::task::models::TaskStatus::Open,
            total_cost_usd: 0.0,
            site_url: None,
            template_id: None,
            vibe: None,
            name: None,
            description: None,
            site_id: None,
            site_provider: None,
            branch: None,
            created_by: owner_user_id.map(|u| uid(u).to_string()),
            visibility: crate::task::models::Visibility::Private,
            scope,
            team_id: team_id.map(str::to_string),
            owner_user_id: owner_user_id.map(|u| uid(u).to_string()),
            created_at_db: None,
            updated_at: None,
            append_system_prompt: None,
            headless: false,
            ttl_minutes: 0,
            inactive_ttl_days: 0,
            trigger_id: None,
            workspace_id: None,
            disabled_servers: None,
        }
    }

    fn user_fixture(id: &uuid::Uuid, role: UserRole) -> User {
        User {
            id: *id,
            provider_user_id: None,
            login: id.to_string(),
            email: None,
            name: None,
            avatar_url: None,
            provider: "test".to_string(),
            role,
            created_at: epoch(),
            last_login_at: epoch(),
            settings: serde_json::json!({}),
            is_bot: false,
        }
    }

    fn claims_for(user_id: &str) -> jwt::UserTokenClaims {
        jwt::UserTokenClaims {
            user_id: uid(user_id).to_string(),
            login: user_id.to_string(),
            iat: 0,
            exp: u64::MAX,
            iss: "moatless-vibe-backend".to_string(),
            aud: None,
        }
    }

    /// Build a service with only the repositories `assert_task_access` consults
    /// wired; everything else is `None`/default.
    fn service_with(
        tasks: MockTaskRepository,
        teams: StubTeamRepo,
        users: MockUserRepository,
    ) -> BackendSandboxServiceImpl {
        BackendSandboxServiceImpl {
            events: None,
            messages: None,
            provider_tokens: None,
            tasks: Some(Arc::new(tasks)),
            teams: Some(Arc::new(teams)),
            task_turns: None,
            task_events: None,
            task_relations: None,
            task_token_secret: None,
            github_app_service: None,
            github_apps: Arc::new(GitHubAppRegistry::default()),
            provider_token_auth: None,
            secrets: None,
            credentials_svc: None,
            repositories: None,
            workspaces: None,
            backend_config: None,
            s3: None,
            files: None,
            s3_bucket: None,
            s3_key_prefix: String::new(),
            transcript_s3_prefix: String::new(),
            workspace_s3_prefix: String::new(),
            users: Some(Arc::new(users)),
            event_dedup: Arc::new(crate::sandbox::event_dedup::EventDedup::default()),
        }
    }

    /// Mock task repo whose scoped `find_by_id` returns `task` for any caller —
    /// i.e. "the scope filter let the read through". Pass `None` to model a
    /// task that is missing or invisible to the caller.
    fn tasks_returning(task: Option<Task>) -> MockTaskRepository {
        let mut m = MockTaskRepository::new();
        m.expect_find_by_id()
            .returning(move |_, _| Ok(task.clone()));
        m
    }

    fn users_regular() -> MockUserRepository {
        let mut m = MockUserRepository::new();
        m.expect_get_user()
            .returning(|id| Ok(Some(user_fixture(id, UserRole::User))));
        m
    }

    fn users_admin() -> MockUserRepository {
        let mut m = MockUserRepository::new();
        m.expect_get_user()
            .returning(|id| Ok(Some(user_fixture(id, UserRole::Admin))));
        m
    }

    #[tokio::test]
    async fn assert_task_access_allows_user_task_for_owner() {
        let svc = service_with(
            tasks_returning(Some(task_fixture(Scope::User, Some("owner"), None))),
            StubTeamRepo::empty(),
            users_regular(),
        );
        assert!(svc
            .assert_task_access(&claims_for("owner"), "task-1")
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn assert_task_access_denies_user_task_for_non_owner() {
        // A user-scoped task belongs only to its owner; a different caller with a
        // valid JWT must be rejected even though the token itself is genuine.
        let svc = service_with(
            tasks_returning(Some(task_fixture(Scope::User, Some("owner"), None))),
            StubTeamRepo::empty(),
            users_regular(),
        );
        let err = svc
            .assert_task_access(&claims_for("attacker"), "task-1")
            .await
            .expect_err("non-owner must be denied");
        assert_eq!(err.code(), tonic::Code::PermissionDenied);
    }

    #[tokio::test]
    async fn assert_task_access_public_visibility_never_grants_control() {
        // A public task is *viewable* by anyone (the scoped read's visibility
        // bypass, modelled here by the mock returning the row), but sandbox
        // control — credentials, messages, workspace writes — stays with the
        // owner. This is the regression test for reusing the plain read gate.
        let mut task = task_fixture(Scope::User, Some("owner"), None);
        task.visibility = Visibility::Public;
        let svc = service_with(
            tasks_returning(Some(task)),
            StubTeamRepo::empty(),
            users_regular(),
        );
        let err = svc
            .assert_task_access(&claims_for("stranger"), "task-1")
            .await
            .expect_err("public visibility must not grant sandbox control");
        assert_eq!(err.code(), tonic::Code::PermissionDenied);
    }

    #[tokio::test]
    async fn assert_task_access_team_task_requires_managing_role() {
        // Mirrors HTTP `can_control_task`: a managing team role (owner/admin)
        // controls the sandbox; a plain member (who can view and message the
        // task over HTTP) does not hold its credentials-bearing JWT surface.
        let teams = &[("manager", "admin"), ("member", "member")];

        let manager_svc = service_with(
            tasks_returning(Some(task_fixture(Scope::Team, None, Some("team-1")))),
            StubTeamRepo::with_roles(teams),
            users_regular(),
        );
        assert!(manager_svc
            .assert_task_access(&claims_for("manager"), "task-1")
            .await
            .is_ok());

        let member_svc = service_with(
            tasks_returning(Some(task_fixture(Scope::Team, None, Some("team-1")))),
            StubTeamRepo::with_roles(teams),
            users_regular(),
        );
        let err = member_svc
            .assert_task_access(&claims_for("member"), "task-1")
            .await
            .expect_err("plain member must not control someone else's sandbox");
        assert_eq!(err.code(), tonic::Code::PermissionDenied);
    }

    #[tokio::test]
    async fn assert_task_access_team_task_creator_member_controls_own_sandbox() {
        // Regression: `create_task` lets any team member create a team-scoped
        // task (inheriting the repo's team scope), storing `created_by` with
        // `owner_user_id = None`, and the sandbox JWT is minted for that
        // creator. The creator must control their own sandbox even with a
        // plain `member` role — while an unrelated member (previous test)
        // stays denied.
        let teams = &[("creator", "member")];
        let mut task = task_fixture(Scope::Team, None, Some("team-1"));
        task.created_by = Some(uid("creator").to_string());

        let svc = service_with(
            tasks_returning(Some(task)),
            StubTeamRepo::with_roles(teams),
            users_regular(),
        );
        assert!(svc
            .assert_task_access(&claims_for("creator"), "task-1")
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn assert_task_access_global_task_requires_admin() {
        // Global-scoped tasks are admin-created (`can_create_scope`) and
        // admin-controlled; being globally *visible* doesn't hand out control.
        let denied = service_with(
            tasks_returning(Some(task_fixture(Scope::Global, Some("owner"), None))),
            StubTeamRepo::empty(),
            users_regular(),
        );
        let err = denied
            .assert_task_access(&claims_for("stranger"), "task-1")
            .await
            .expect_err("non-admin must not control a global task");
        assert_eq!(err.code(), tonic::Code::PermissionDenied);

        let allowed = service_with(
            tasks_returning(Some(task_fixture(Scope::Global, Some("owner"), None))),
            StubTeamRepo::empty(),
            users_admin(),
        );
        assert!(allowed
            .assert_task_access(&claims_for("admin"), "task-1")
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn assert_task_access_admin_bypasses_scope() {
        // A platform admin reaches a team task they are not a member of.
        let svc = service_with(
            tasks_returning(Some(task_fixture(Scope::Team, None, Some("team-1")))),
            StubTeamRepo::empty(),
            users_admin(),
        );
        assert!(svc
            .assert_task_access(&claims_for("admin"), "task-1")
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn assert_task_access_missing_task_is_not_found() {
        // Covers both a nonexistent task and one the scoped read filtered out —
        // an invisible task is indistinguishable from a missing one, like the
        // HTTP 404.
        let svc = service_with(
            tasks_returning(None),
            StubTeamRepo::empty(),
            users_regular(),
        );
        let err = svc
            .assert_task_access(&claims_for("anyone"), "ghost")
            .await
            .expect_err("missing task must error");
        assert_eq!(err.code(), tonic::Code::NotFound);
    }
}
