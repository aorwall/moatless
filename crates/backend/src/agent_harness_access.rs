//! Agent-harness credential administration and device login.

use std::sync::Arc;

use async_trait::async_trait;

use crate::auth::codex_auth_json::{build_codex_metadata, parse_codex_auth_json};
use crate::auth::codex_scope::CodexScope;
use crate::auth::error::AuthError;
use crate::auth::provider_token_dao::ProviderTokenDao;
use crate::auth::users::UserDao;
use crate::credential_sync::CredentialSync;
use crate::credentials::codex::oauth::{
    self as codex_oauth, CodexOAuthError, DeviceCodeStart, PollStatus,
};
use moatless_core::config::CodexOAuthConfig;

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
    #[error("Bad request: {0}")]
    BadRequest(String),
    #[error("Database error: {0}")]
    Db(#[from] sqlx::Error),
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

pub struct AgentHarnessAccess {
    provider_tokens: Arc<dyn ProviderTokenDao>,
    credential_sync: Arc<CredentialSync>,
    oauth_config: Arc<CodexOAuthConfig>,
    device_login: Arc<dyn CodexDeviceLoginProvider>,
    users: Arc<dyn UserDao>,
}

impl AgentHarnessAccess {
    pub fn new(
        provider_tokens: Arc<dyn ProviderTokenDao>,
        credential_sync: Arc<CredentialSync>,
        oauth_config: Arc<CodexOAuthConfig>,
        device_login: Arc<dyn CodexDeviceLoginProvider>,
        users: Arc<dyn UserDao>,
    ) -> Self {
        Self {
            provider_tokens,
            credential_sync,
            oauth_config,
            device_login,
            users,
        }
    }

    pub async fn status(
        &self,
        scope: &CodexScope,
    ) -> Result<AgentHarnessCredentialStatus, AgentHarnessAccessError> {
        let token = self.provider_tokens.find_codex_blob(scope).await?;
        let Some(row) = token else {
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
        scope: CodexScope,
        auth_json: String,
    ) -> Result<AgentHarnessCredentialStatus, AgentHarnessAccessError> {
        let parsed = parse_codex_auth_json(&auth_json).map_err(|error| match error {
            AuthError::BadRequest(message) => AgentHarnessAccessError::BadRequest(message),
            other => AgentHarnessAccessError::BadRequest(other.to_string()),
        })?;
        let metadata = build_codex_metadata(&parsed);
        let serialized = parsed.to_string();
        self.provider_tokens
            .upsert_codex_blob(
                &scope,
                &serialized,
                None,
                Some("auth_json"),
                Some(&metadata),
            )
            .await?;
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
        user_id: &str,
        auth_json: String,
    ) -> Result<AgentHarnessCredentialStatus, AgentHarnessAccessError> {
        let target = match user_id.parse::<uuid::Uuid>() {
            Ok(user_id) => self.users.get_user(&user_id).await?,
            Err(_) => None,
        };
        if target.is_none() {
            return Err(AgentHarnessAccessError::BadRequest(format!(
                "Unknown target user '{user_id}'"
            )));
        }
        self.save(CodexScope::user(user_id.to_string()), auth_json)
            .await
    }

    pub async fn delete(
        &self,
        scope: &CodexScope,
    ) -> Result<AgentHarnessCredentialStatus, AgentHarnessAccessError> {
        self.provider_tokens.delete_codex_blob(scope).await?;
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
        user_id: &str,
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
                let scope = CodexScope::user(user_id.to_string());
                self.provider_tokens
                    .upsert_codex_blob(
                        &scope,
                        &serialized,
                        None,
                        Some("auth_json"),
                        Some(&metadata),
                    )
                    .await?;
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

    use moatless_backend_core::sandbox::{SandboxClient, SandboxConnector, SandboxError};
    use tokio::sync::{Mutex, mpsc};

    use super::*;
    use crate::auth::test_support::StubProviderTokens;
    use crate::auth::users::MockUserDao;
    use crate::credentials::CredentialService;
    use crate::credentials::codex::oauth::IdTokenClaims;
    use crate::task::dao::MockTaskDao;

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

    struct PersistObservingConnector {
        tokens: Arc<StubProviderTokens>,
        observed: mpsc::UnboundedSender<bool>,
    }

    #[async_trait]
    impl SandboxConnector for PersistObservingConnector {
        async fn sandbox_client(&self, _task_id: &str) -> Result<SandboxClient, SandboxError> {
            let _ = self
                .observed
                .send(self.tokens.writes.load(Ordering::Acquire) > 0);
            Err(SandboxError::NotReady("test connector".to_string()))
        }
    }

    fn access(
        tokens: Arc<StubProviderTokens>,
        poll: PollStatus,
        tasks: MockTaskDao,
        connector: Arc<dyn SandboxConnector>,
    ) -> AgentHarnessAccess {
        let provider_tokens: Arc<dyn ProviderTokenDao> = tokens;
        let credentials = Arc::new(CredentialService::new(Arc::clone(&provider_tokens)));
        AgentHarnessAccess::new(
            Arc::clone(&provider_tokens),
            Arc::new(CredentialSync::new(credentials, Arc::new(tasks), connector)),
            Arc::new(CodexOAuthConfig {
                issuer: "https://auth.example".to_string(),
                client_id: "client".to_string(),
            }),
            Arc::new(FakeDeviceLogin {
                poll: Mutex::new(Some(poll)),
            }),
            Arc::new(MockUserDao::new()),
        )
    }

    #[tokio::test]
    async fn save_persists_before_sandbox_fan_out() {
        let tokens = Arc::new(StubProviderTokens::default());
        let (tx, mut rx) = mpsc::unbounded_channel();
        let connector = Arc::new(PersistObservingConnector {
            tokens: Arc::clone(&tokens),
            observed: tx,
        });
        let mut tasks = MockTaskDao::new();
        tasks
            .expect_list_open_codex_tasks_for_scope()
            .returning(|_, _, _| Ok(vec!["task-1".to_string()]));
        let access = access(tokens, PollStatus::Pending, tasks, connector);

        access
            .save(
                CodexScope::user("u1"),
                r#"{"tokens":{"access_token":"a","refresh_token":"r"}}"#.to_string(),
            )
            .await
            .expect("credential should save");

        let persisted = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .expect("sandbox fan-out should run")
            .expect("connector should report the observation");
        assert!(persisted, "persistence must precede sandbox fan-out");
    }

    #[tokio::test]
    async fn device_poll_only_writes_on_success() {
        for poll in [PollStatus::Pending, PollStatus::Expired] {
            let tokens = Arc::new(StubProviderTokens::default());
            let access = access(
                Arc::clone(&tokens),
                poll,
                MockTaskDao::new(),
                Arc::new(PersistObservingConnector {
                    tokens: Arc::clone(&tokens),
                    observed: mpsc::unbounded_channel().0,
                }),
            );

            access
                .poll_device_login("u1", "device", "code")
                .await
                .expect("poll should succeed");
            assert_eq!(tokens.writes.load(Ordering::Relaxed), 0);
        }

        let tokens = Arc::new(StubProviderTokens::default());
        let mut tasks = MockTaskDao::new();
        tasks
            .expect_list_open_codex_tasks_for_scope()
            .returning(|_, _, _| Ok(Vec::new()));
        let access = access(
            Arc::clone(&tokens),
            PollStatus::Success {
                auth_json: serde_json::json!({"tokens": {"access_token": "a"}}),
                id_token_claims: IdTokenClaims::default(),
            },
            tasks,
            Arc::new(PersistObservingConnector {
                tokens: Arc::clone(&tokens),
                observed: mpsc::unbounded_channel().0,
            }),
        );
        access
            .poll_device_login("u1", "device", "code")
            .await
            .expect("successful poll should persist");
        assert_eq!(tokens.writes.load(Ordering::Relaxed), 1);
    }
}
