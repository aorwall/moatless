//! gRPC server implementing `BackendSandboxService`.
//!
//! Receives events from sandbox pods via gRPC (replaces direct Redis XADD
//! from the sandbox) and serves pending messages (replaces HTTP polling).

use std::collections::HashMap;
use std::env;
use std::sync::Arc;

use serde_json::Value;
use tonic::{Request, Response, Status};
use tracing::{debug, info, warn};

use sandbox_proto::backend_sandbox_service_server::BackendSandboxService;
use sandbox_proto::{
    GetCredentialsRequest, GetCredentialsResponse, GetPendingMessagesRequest,
    GetPendingMessagesResponse, PublishEventRequest, PublishEventResponse,
};

use crate::auth::provider_token_dao::ProviderTokenRepository;
use crate::message::dao::MessageRepository;
use crate::task::dao::TaskRepository;
use crate::events::EventPublisher;
use crate::sandbox::jwt;

/// Known credential env var names.
///
/// Used as map keys in `GetCredentialsResponse` for type-safe credential handling.
pub struct CredentialKey;

impl CredentialKey {
    pub const GITHUB_TOKEN: &str = "GITHUB_TOKEN";
    pub const GITLAB_TOKEN: &str = "GITLAB_TOKEN";
    pub const GITNESS_TOKEN: &str = "GITNESS_TOKEN";
    pub const ANTHROPIC_API_KEY: &str = "ANTHROPIC_API_KEY";
    pub const CLAUDE_CODE_OAUTH_TOKEN: &str = "CLAUDE_CODE_OAUTH_TOKEN";
}

/// gRPC server state — holds event publisher, repository trait objects, and JWT secret.
pub struct BackendSandboxServiceImpl {
    pub events: Option<Arc<dyn EventPublisher>>,
    pub messages: Option<Arc<dyn MessageRepository>>,
    pub provider_tokens: Option<Arc<dyn ProviderTokenRepository>>,
    pub tasks: Option<Arc<dyn TaskRepository>>,
    /// JWT signing key for verifying sandbox auth tokens.
    pub task_token_secret: Option<String>,
}

impl BackendSandboxServiceImpl {
    /// Verify the authorization header from gRPC metadata.
    ///
    /// Extracts `Bearer <token>` from the `authorization` metadata key,
    /// verifies the JWT, and returns the claims.
    #[allow(clippy::result_large_err)]
    fn verify_auth<T>(
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

        jwt::verify_user_auth_token(token, secret)
            .map_err(|e| Status::unauthenticated(format!("Invalid token: {e}")))
    }

    /// Look up the team_id for a task. Returns None on any failure.
    async fn resolve_task_team_id(&self, task_id: &str) -> Option<String> {
        let tasks = self.tasks.as_ref()?;
        match tasks.find_by_id_internal(task_id).await {
            Ok(Some(task)) => task.team_id,
            Ok(None) => {
                warn!(task_id, "resolve_task_team_id: task not found");
                None
            }
            Err(e) => {
                warn!(task_id, error = %e, "resolve_task_team_id: DB error");
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

        // Fetch pending user messages from DB
        let pending = messages.get_pending_user(&req.task_id)
            .await
            .map_err(|e| {
                warn!(error = %e, task_id = %req.task_id, "Failed to fetch pending messages");
                Status::internal("Failed to fetch pending messages")
            })?;

        // Look up latest agent_session_id for --resume <id>
        let resume_session_id = messages.get_latest_agent_session_id(&req.task_id)
            .await
            .unwrap_or(None);

        // Check will_not_resume on the last result — if the previous session
        // ended with an unrecoverable error, don't pass a resume session ID.
        let resume_session_id = if resume_session_id.is_some() {
            match messages.get_last_result(&req.task_id).await {
                Ok(Some(last_result)) if last_result.will_not_resume == Some(true) => {
                    debug!(
                        task_id = %req.task_id,
                        "Last result has will_not_resume=true, clearing resume_session_id"
                    );
                    None
                }
                _ => resume_session_id,
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

                // When will_not_resume is set, also clear the per-message resume flag
                // so the sandbox starts a fresh session.
                let resume = if resume_session_id.is_none() {
                    Some(false)
                } else {
                    Some(row.resume)
                };

                Some(sandbox_proto::PendingMessage {
                    content,
                    model: row.model.clone(),
                    system_prompt: None,
                    resume,
                    agent_type: row.agent_mode.as_ref().map(|m| m.0.clone()),
                    is_error: None,
                    resume_session_id: resume_session_id.clone(),
                })
            })
            .collect();

        Ok(Response::new(GetPendingMessagesResponse { messages }))
    }

    /// Return credentials for sandbox initialization.
    ///
    /// Resolution chain:
    /// 1. Environment variables (from moatless-secrets) — admin-configured, take priority
    /// 2. Team-level provider_tokens (if task belongs to a team)
    /// 3. User-level provider_tokens (user-linked OAuth tokens)
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

        let mut credentials: HashMap<String, String> = HashMap::new();

        // Step 1: Environment variables (from moatless-secrets) take priority
        if let Ok(token) = env::var("GITHUB_PAT") {
            credentials.insert(CredentialKey::GITHUB_TOKEN.to_string(), token);
        }
        if let Ok(token) = env::var("GITLAB_PAT") {
            credentials.insert(CredentialKey::GITLAB_TOKEN.to_string(), token);
        }
        if let Ok(token) = env::var("GITNESS_PAT") {
            credentials.insert(CredentialKey::GITNESS_TOKEN.to_string(), token);
        }
        if let Ok(key) = env::var("ANTHROPIC_API_KEY") {
            credentials.insert(CredentialKey::ANTHROPIC_API_KEY.to_string(), key);
        }
        if let Ok(token) = env::var("CLAUDE_CODE_OAUTH_TOKEN") {
            credentials.insert(
                CredentialKey::CLAUDE_CODE_OAUTH_TOKEN.to_string(),
                token,
            );
        }

        // Step 2: Team-level provider_tokens (if task belongs to a team)
        let team_id = self.resolve_task_team_id(&req.task_id).await;
        if let (Some(ref team_id), Some(ref provider_tokens)) = (&team_id, &self.provider_tokens) {
            for (provider, cred_key) in [
                ("github", CredentialKey::GITHUB_TOKEN),
                ("gitlab", CredentialKey::GITLAB_TOKEN),
                ("gitness", CredentialKey::GITNESS_TOKEN),
            ] {
                if !credentials.contains_key(cred_key) {
                    match provider_tokens.find_team_token(team_id, provider).await {
                        Ok(Some(row)) => {
                            credentials.insert(cred_key.to_string(), row.access_token);
                        }
                        Ok(None) => {}
                        Err(e) => {
                            warn!(error = %e, team_id, provider, "Failed to query team provider_token");
                        }
                    }
                }
            }

            if !credentials.contains_key(CredentialKey::CLAUDE_CODE_OAUTH_TOKEN)
                && !credentials.contains_key(CredentialKey::ANTHROPIC_API_KEY)
            {
                match provider_tokens.find_team_token(team_id, "claude").await {
                    Ok(Some(row)) => {
                        credentials.insert(
                            CredentialKey::CLAUDE_CODE_OAUTH_TOKEN.to_string(),
                            row.access_token,
                        );
                    }
                    Ok(None) => {}
                    Err(e) => {
                        warn!(error = %e, team_id, "Failed to query team claude provider_token");
                    }
                }
            }
        }

        // Step 3: User-level provider_tokens (fills remaining gaps)
        if let Some(ref provider_tokens) = self.provider_tokens {
            for (provider, cred_key) in [
                ("github", CredentialKey::GITHUB_TOKEN),
                ("gitlab", CredentialKey::GITLAB_TOKEN),
                ("gitness", CredentialKey::GITNESS_TOKEN),
            ] {
                if !credentials.contains_key(cred_key) {
                    match provider_tokens.find_provider_token(&claims.user_id, provider).await {
                        Ok(Some(row)) => {
                            credentials.insert(cred_key.to_string(), row.access_token);
                        }
                        Ok(None) => {}
                        Err(e) => {
                            warn!(error = %e, user_id = %claims.user_id, provider, "Failed to query provider_token");
                        }
                    }
                }
            }

            if !credentials.contains_key(CredentialKey::CLAUDE_CODE_OAUTH_TOKEN)
                && !credentials.contains_key(CredentialKey::ANTHROPIC_API_KEY)
            {
                match provider_tokens.find_provider_token(&claims.user_id, "claude").await
                {
                    Ok(Some(row)) => {
                        credentials.insert(
                            CredentialKey::CLAUDE_CODE_OAUTH_TOKEN.to_string(),
                            row.access_token,
                        );
                    }
                    Ok(None) => {}
                    Err(e) => {
                        warn!(error = %e, user_id = %claims.user_id, "Failed to query claude provider_token");
                    }
                }
            }
        }

        info!(
            task_id = %req.task_id,
            user_id = %claims.user_id,
            has_github = credentials.contains_key(CredentialKey::GITHUB_TOKEN),
            has_gitlab = credentials.contains_key(CredentialKey::GITLAB_TOKEN),
            has_gitness = credentials.contains_key(CredentialKey::GITNESS_TOKEN),
            has_anthropic = credentials.contains_key(CredentialKey::ANTHROPIC_API_KEY),
            has_claude_oauth = credentials.contains_key(CredentialKey::CLAUDE_CODE_OAUTH_TOKEN),
            "Credentials resolved"
        );

        Ok(Response::new(GetCredentialsResponse { credentials }))
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
                    sandbox_proto::TextBlock {
                        text: s.clone(),
                    },
                )),
            }]
        }
        _ => vec![],
    }
}

