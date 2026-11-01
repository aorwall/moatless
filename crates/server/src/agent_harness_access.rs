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
use moatless_types::agent::CodexOAuthConfig;
use moatless_types::agent::CodexScope;
use moatless_types::http::HttpTimeouts;

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

impl ProviderCodexDeviceLogin {
    /// Panics when the TLS backend cannot be loaded, as `reqwest::Client::new`
    /// does.
    #[expect(
        clippy::disallowed_methods,
        clippy::expect_used,
        reason = "sets the timeouts `Config::http` holds, and fails as `Client::new` would"
    )]
    pub fn new(timeouts: HttpTimeouts) -> Self {
        Self {
            client: reqwest::Client::builder()
                .connect_timeout(timeouts.connect)
                .timeout(timeouts.total)
                .build()
                .expect("the TLS backend initialises"),
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

/// Restrict credential access to the caller's own user scope or an
/// administrator.
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
    oauth_config: Arc<CodexOAuthConfig>,
    device_login: Arc<dyn CodexDeviceLoginProvider>,
    users: Arc<dyn UserDisplay>,
}

impl AgentHarnessAccess {
    pub fn new(
        credentials: Arc<CredentialService>,
        oauth_config: Arc<CodexOAuthConfig>,
        device_login: Arc<dyn CodexDeviceLoginProvider>,
        users: Arc<dyn UserDisplay>,
    ) -> Self {
        Self {
            credentials,
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

    use tokio::sync::Mutex;

    use super::*;
    use moatless_identity::auth::test_support::StubProviderTokens;
    use moatless_identity::credentials::codex::oauth::IdTokenClaims;
    use moatless_identity::credentials::test_support::credentials_over_stub;
    use moatless_ports::identity::MockUserDisplay;

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

    fn access(tokens: Arc<StubProviderTokens>, poll: PollStatus) -> AgentHarnessAccess {
        AgentHarnessAccess::new(
            credentials_over_stub(tokens),
            Arc::new(CodexOAuthConfig {
                issuer: "https://auth.example".to_string(),
                client_id: "client".to_string(),
                refresh_url: moatless_core::config::CODEX_REFRESH_URL_DEFAULT.to_string(),
            }),
            Arc::new(FakeDeviceLogin {
                poll: Mutex::new(Some(poll)),
            }),
            Arc::new(MockUserDisplay::new()),
        )
    }

    #[tokio::test]
    async fn direct_non_admin_calls_cannot_read_or_mutate_other_scopes() {
        let tokens = Arc::new(StubProviderTokens::default());
        let access = access(Arc::clone(&tokens), PollStatus::Pending);
        let ctx = ScopeContext::user("caller");
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
            let access = access(Arc::clone(&tokens), poll);

            access
                .poll_device_login(&ScopeContext::user("u1"), "device", "code")
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
        );
        access
            .poll_device_login(&ScopeContext::user("u1"), "device", "code")
            .await
            .expect("successful poll should persist");
        assert_eq!(tokens.writes.load(Ordering::Relaxed), 1);
    }
}
