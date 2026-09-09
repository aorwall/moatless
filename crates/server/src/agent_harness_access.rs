//! Agent-harness credential administration and device login.

use std::sync::Arc;

use async_trait::async_trait;
use moatless_types::access_control::ScopeContext;

use moatless_identity::auth::codex_auth_json::build_codex_metadata;
use moatless_identity::auth::codex_auth_json::parse_codex_auth_json;
use moatless_identity::auth::error::AuthError;
use moatless_identity::credentials::codex::oauth as codex_oauth;
use moatless_identity::credentials::codex::oauth::CodexOAuthError;
use moatless_identity::credentials::codex::oauth::DeviceCodeStart;
use moatless_identity::credentials::codex::oauth::PollStatus;
use moatless_identity::credentials::{CredentialError, CredentialService};
use moatless_ports::identity::UserDisplay;
use moatless_tasks::credential_sync::CredentialSync;
use moatless_types::agent::CodexOAuthConfig;
use moatless_types::agent::CodexScope;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentHarnessCredentialStatus {
    pub connected: bool,
    pub token_type: Option<String>,
    pub auth_method: Option<String>,
    pub email: Option<String>,
    pub plan_type: Option<String>,
    pub needs_reconnect: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeviceLoginPoll {
    Pending,
    Success {
        connected: bool,
        email: Option<String>,
        plan_type: Option<String>,
    },
    Expired,
}

#[derive(Debug, thiserror::Error)]
pub enum AgentHarnessAccessError {
    #[error("Forbidden: {0}")]
    Forbidden(String),
    #[error("Forbidden")]
    AuthForbidden,
    #[error("Bad request: {0}")]
    BadRequest(String),
    #[error("Database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("Database error: {0}")]
    Store(#[from] moatless_types::store::StoreError),
    #[error("Provider error: {0}")]
    OAuth(#[source] CodexOAuthError),
}

#[async_trait]
pub trait CodexDeviceLoginProvider: Send + Sync {
    async fn start(
        &self,
        issuer: &str,
        client_id: &str,
    ) -> Result<DeviceCodeStart, CodexOAuthError>;

    async fn poll(
        &self,
        issuer: &str,
        client_id: &str,
        device_auth_id: &str,
        user_code: &str,
    ) -> Result<PollStatus, CodexOAuthError>;
}

pub struct ProviderCodexDeviceLogin {
    client: reqwest::Client,
}

impl Default for ProviderCodexDeviceLogin {
    fn default() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl CodexDeviceLoginProvider for ProviderCodexDeviceLogin {
    async fn start(
        &self,
        issuer: &str,
        client_id: &str,
    ) -> Result<DeviceCodeStart, CodexOAuthError> {
        codex_oauth::request_device_code(&self.client, issuer, client_id).await
    }

    async fn poll(
        &self,
        issuer: &str,
        client_id: &str,
        device_auth_id: &str,
        user_code: &str,
    ) -> Result<PollStatus, CodexOAuthError> {
        codex_oauth::poll_once(&self.client, issuer, client_id, device_auth_id, user_code).await
    }
}

/// Restrict credential access to the caller's own user scope or an administrator.
fn authorize_scope(
    ctx: &ScopeContext,
    scope: &CodexScope,
    action: &str,
) -> Result<(), AgentHarnessAccessError> {
    if ctx.is_admin || matches!(scope, CodexScope::User(id) if id == &ctx.user_id) {
        return Ok(());
    }
    let message = match scope {
        CodexScope::Global => format!("Admin role required to {action} the global Codex blob"),
        CodexScope::User(_) => {
            "Admin role required to access another user's Codex blob".to_string()
        }
    };
    Err(AgentHarnessAccessError::Forbidden(message))
}

pub struct AgentHarnessAccess {
    credentials: Arc<CredentialService>,
    credential_sync: Arc<CredentialSync>,
    oauth_config: Arc<CodexOAuthConfig>,
    device_login: Arc<dyn CodexDeviceLoginProvider>,
    users: Arc<dyn UserDisplay>,
}

impl AgentHarnessAccess {
    pub fn new(
        credentials: Arc<CredentialService>,
        credential_sync: Arc<CredentialSync>,
        oauth_config: Arc<CodexOAuthConfig>,
        device_login: Arc<dyn CodexDeviceLoginProvider>,
        users: Arc<dyn UserDisplay>,
    ) -> Self {
        Self {
            credentials,
            credential_sync,
            oauth_config,
            device_login,
            users,
        }
    }

    pub async fn status(
        &self,
        ctx: &ScopeContext,
        scope: &CodexScope,
    ) -> Result<AgentHarnessCredentialStatus, AgentHarnessAccessError> {
        authorize_scope(ctx, scope, "read")?;
        let stored = self
            .credentials
            .codex_credential(scope)
            .await
            .map_err(credential_error)?;
        let Some(row) = stored else {
            return Ok(disconnected());
        };
        let metadata = row.metadata.as_ref();
        Ok(AgentHarnessCredentialStatus {
            connected: true,
            token_type: row.token_type,
            auth_method: metadata_string(metadata, "auth_method"),
            email: metadata_string(metadata, "email"),
            plan_type: metadata_string(metadata, "plan_type"),
            needs_reconnect: Some(
                metadata.is_some_and(|value| value.get("refresh_failure").is_some()),
            ),
        })
    }

    pub async fn save(
        &self,
        ctx: &ScopeContext,
        scope: CodexScope,
        auth_json: String,
    ) -> Result<AgentHarnessCredentialStatus, AgentHarnessAccessError> {
        authorize_scope(ctx, &scope, "set")?;
        let parsed = parse_codex_auth_json(&auth_json).map_err(|error| match error {
            AuthError::BadRequest(message) => AgentHarnessAccessError::BadRequest(message),
            other => AgentHarnessAccessError::BadRequest(other.to_string()),
        })?;
        let metadata = build_codex_metadata(&parsed);
        let serialized = parsed.to_string();
        self.credentials
            .store_codex_credential(&scope, &serialized, &metadata)
            .await
            .map_err(credential_error)?;
        self.credential_sync
            .broadcast_codex_rotation(&scope, &serialized)
            .await;
        Ok(AgentHarnessCredentialStatus {
            connected: true,
            token_type: Some("auth_json".to_string()),
            auth_method: Some("auth_json".to_string()),
            email: None,
            plan_type: None,
            needs_reconnect: Some(false),
        })
    }

    pub async fn save_for_user(
        &self,
        ctx: &ScopeContext,
        user_id: &str,
        auth_json: String,
    ) -> Result<AgentHarnessCredentialStatus, AgentHarnessAccessError> {
        if !ctx.is_admin {
            return Err(AgentHarnessAccessError::AuthForbidden);
        }
        let target = match user_id.parse::<uuid::Uuid>() {
            Ok(user_id) => self.users.name(&user_id).await?,
            Err(_) => None,
        };
        if target.is_none() {
            return Err(AgentHarnessAccessError::BadRequest(format!(
                "Unknown target user '{user_id}'"
            )));
        }
        self.save(ctx, CodexScope::user(user_id.to_string()), auth_json)
            .await
    }

    pub async fn delete(
        &self,
        ctx: &ScopeContext,
        scope: &CodexScope,
    ) -> Result<AgentHarnessCredentialStatus, AgentHarnessAccessError> {
        authorize_scope(ctx, scope, "delete")?;
        self.credentials
            .delete_codex_credential(scope)
            .await
            .map_err(credential_error)?;
        Ok(disconnected())
    }

    pub async fn start_device_login(&self) -> Result<DeviceCodeStart, AgentHarnessAccessError> {
        self.device_login
            .start(&self.oauth_config.issuer, &self.oauth_config.client_id)
            .await
            .map_err(AgentHarnessAccessError::OAuth)
    }

    pub async fn poll_device_login(
        &self,
        ctx: &ScopeContext,
        device_auth_id: &str,
        user_code: &str,
    ) -> Result<DeviceLoginPoll, AgentHarnessAccessError> {
        let outcome = self
            .device_login
            .poll(
                &self.oauth_config.issuer,
                &self.oauth_config.client_id,
                device_auth_id,
                user_code,
            )
            .await
            .map_err(AgentHarnessAccessError::OAuth)?;
        match outcome {
            PollStatus::Pending => Ok(DeviceLoginPoll::Pending),
            PollStatus::Expired => Ok(DeviceLoginPoll::Expired),
            PollStatus::Success {
                auth_json,
                id_token_claims,
            } => {
                let metadata = serde_json::json!({
                    "auth_method": "device_code",
                    "has_tokens": true,
                    "email": id_token_claims.email,
                    "plan_type": id_token_claims.chatgpt_plan_type,
                    "account_id": id_token_claims.chatgpt_account_id,
                });
                let serialized = auth_json.to_string();
                let scope = CodexScope::user(ctx.user_id.clone());
                self.credentials
                    .store_codex_credential(&scope, &serialized, &metadata)
                    .await
                    .map_err(credential_error)?;
                self.credential_sync
                    .broadcast_codex_rotation(&scope, &serialized)
                    .await;
                Ok(DeviceLoginPoll::Success {
                    connected: true,
                    email: id_token_claims.email,
                    plan_type: id_token_claims.chatgpt_plan_type,
                })
            }
        }
    }
}

/// The three Codex operations above reach nothing but the store, so
/// [`CredentialError::Database`] is the only variant they raise; any other is a
/// wiring mistake and surfaces with its context rather than being flattened.
fn credential_error(error: CredentialError) -> AgentHarnessAccessError {
    match error {
        CredentialError::Database(source) => AgentHarnessAccessError::Db(source),
        other => AgentHarnessAccessError::Store(moatless_types::store::StoreError::with_source(
            "Codex credential store",
            other,
        )),
    }
}

fn disconnected() -> AgentHarnessCredentialStatus {
    AgentHarnessCredentialStatus {
        connected: false,
        token_type: None,
        auth_method: None,
        email: None,
        plan_type: None,
        needs_reconnect: None,
    }
}

fn metadata_string(metadata: Option<&serde_json::Value>, key: &str) -> Option<String> {
    metadata
        .and_then(|value| value.get(key))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use moatless_ports::sandbox::{CredentialBroadcast, SandboxError};
    use tokio::sync::{Mutex, mpsc};

    use super::*;
    use moatless_identity::auth::test_support::StubProviderTokens;
    use moatless_identity::credentials::codex::oauth::IdTokenClaims;
    use moatless_identity::credentials::test_support::credentials_over_stub;
    use moatless_ports::identity::MockUserDisplay;
    use moatless_tasks::task::test_support::ScriptedTasks;

    struct FakeDeviceLogin {
        poll: Mutex<Option<PollStatus>>,
    }

    #[async_trait]
    impl CodexDeviceLoginProvider for FakeDeviceLogin {
        async fn start(
            &self,
            _issuer: &str,
            _client_id: &str,
        ) -> Result<DeviceCodeStart, CodexOAuthError> {
            unreachable!("these tests only exercise polling")
        }

        async fn poll(
            &self,
            _issuer: &str,
            _client_id: &str,
            _device_auth_id: &str,
            _user_code: &str,
        ) -> Result<PollStatus, CodexOAuthError> {
            Ok(self
                .poll
                .lock()
                .await
                .take()
                .expect("one poll result should be configured"))
        }
    }

    struct PersistObservingBroadcast {
        tokens: Arc<StubProviderTokens>,
        observed: mpsc::UnboundedSender<bool>,
    }

    #[async_trait]
    impl CredentialBroadcast for PersistObservingBroadcast {
        async fn refresh_credential(
            &self,
            _task_id: &str,
            _key: &str,
            _value: &str,
        ) -> Result<(), SandboxError> {
            let _ = self
                .observed
                .send(self.tokens.writes.load(Ordering::Acquire) > 0);
            Err(SandboxError::NotReady("test broadcast".to_string()))
        }
    }

    fn access(
        tokens: Arc<StubProviderTokens>,
        poll: PollStatus,
        open_codex_tasks: Vec<String>,
        broadcast: Arc<dyn CredentialBroadcast>,
    ) -> AgentHarnessAccess {
        let credentials = credentials_over_stub(tokens);
        AgentHarnessAccess::new(
            Arc::clone(&credentials),
            ScriptedTasks::new()
                .open_codex_tasks(open_codex_tasks)
                .credential_sync(credentials, broadcast),
            Arc::new(CodexOAuthConfig {
                issuer: "https://auth.example".to_string(),
                client_id: "client".to_string(),
            }),
            Arc::new(FakeDeviceLogin {
                poll: Mutex::new(Some(poll)),
            }),
            Arc::new(MockUserDisplay::new()),
        )
    }

    #[tokio::test]
    async fn save_persists_before_sandbox_fan_out() {
        let tokens = Arc::new(StubProviderTokens::default());
        let (tx, mut rx) = mpsc::unbounded_channel();
        let broadcast = Arc::new(PersistObservingBroadcast {
            tokens: Arc::clone(&tokens),
            observed: tx,
        });
        let access = access(
            tokens,
            PollStatus::Pending,
            vec!["task-1".to_string()],
            broadcast,
        );

        access
            .save(
                &ScopeContext {
                    user_id: "u1".into(),
                    is_admin: false,
                },
                CodexScope::user("u1"),
                r#"{"tokens":{"access_token":"a","refresh_token":"r"}}"#.to_string(),
            )
            .await
            .expect("credential should save");

        let persisted = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .expect("sandbox fan-out should run")
            .expect("broadcast should report the observation");
        assert!(persisted, "persistence must precede sandbox fan-out");
    }

    #[tokio::test]
    async fn direct_non_admin_calls_cannot_read_or_mutate_other_scopes() {
        let tokens = Arc::new(StubProviderTokens::default());
        let access = access(
            Arc::clone(&tokens),
            PollStatus::Pending,
            Vec::new(),
            Arc::new(PersistObservingBroadcast {
                tokens: Arc::clone(&tokens),
                observed: mpsc::unbounded_channel().0,
            }),
        );
        let ctx = ScopeContext {
            user_id: "caller".into(),
            is_admin: false,
        };
        for scope in [CodexScope::Global, CodexScope::user("other")] {
            assert!(matches!(
                access.status(&ctx, &scope).await,
                Err(AgentHarnessAccessError::Forbidden(_))
            ));
            assert!(matches!(
                access
                    .save(&ctx, scope.clone(), "invalid JSON".into())
                    .await,
                Err(AgentHarnessAccessError::Forbidden(_))
            ));
            assert!(matches!(
                access.delete(&ctx, &scope).await,
                Err(AgentHarnessAccessError::Forbidden(_))
            ));
        }
        assert!(matches!(
            access
                .save_for_user(&ctx, "other", "invalid JSON".into())
                .await,
            Err(AgentHarnessAccessError::AuthForbidden)
        ));
        assert_eq!(tokens.writes.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn device_poll_only_writes_on_success() {
        for poll in [PollStatus::Pending, PollStatus::Expired] {
            let tokens = Arc::new(StubProviderTokens::default());
            let access = access(
                Arc::clone(&tokens),
                poll,
                Vec::new(),
                Arc::new(PersistObservingBroadcast {
                    tokens: Arc::clone(&tokens),
                    observed: mpsc::unbounded_channel().0,
                }),
            );

            access
                .poll_device_login(
                    &ScopeContext {
                        user_id: "u1".into(),
                        is_admin: false,
                    },
                    "device",
                    "code",
                )
                .await
                .expect("poll should succeed");
            assert_eq!(tokens.writes.load(Ordering::Relaxed), 0);
        }

        let tokens = Arc::new(StubProviderTokens::default());
        let access = access(
            Arc::clone(&tokens),
            PollStatus::Success {
                auth_json: serde_json::json!({"tokens": {"access_token": "a"}}),
                id_token_claims: IdTokenClaims::default(),
            },
            Vec::new(),
            Arc::new(PersistObservingBroadcast {
                tokens: Arc::clone(&tokens),
                observed: mpsc::unbounded_channel().0,
            }),
        );
        access
            .poll_device_login(
                &ScopeContext {
                    user_id: "u1".into(),
                    is_admin: false,
                },
                "device",
                "code",
            )
            .await
            .expect("successful poll should persist");
        assert_eq!(tokens.writes.load(Ordering::Relaxed), 1);
    }
}
