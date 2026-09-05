//! Identity's side of the seams declared in `moatless_ports::identity`.
//!
//! Each adapter wraps one store and narrows it to the values a peer crate
//! reads, converting `sqlx::Error` into a `StoreError` that boxes it. The
//! composition root builds these once and hands out the port a consumer needs.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use moatless_adapter_model::apps::{AdapterAppReader, AdapterAppSecrets};
use moatless_core::config::Config;
use moatless_core::db::DbPool;
use moatless_ports::identity::{
    AppCredentials, AppToken, CodexScopeReader, CommitIdentity, GitIdentity, HarnessCredentials,
    LinkedUser, ProviderIdentityLink, RunAsUser, SecretCatalog, SecretResolver, UserDisplay,
    UserResolver,
};
use moatless_types::access_control::ScopeContext;
use moatless_types::agent::CodexScope;
use moatless_types::store::StoreError;
use moatless_types::user::UserRole;
use uuid::Uuid;

use crate::auth::provider_token_dao::ProviderTokenDao;
use crate::auth::users::{User, UserDao};
use crate::secret::service::SecretService;

/// The `users` table, narrowed to the four identity ports peers read.
///
/// It also holds the token store, because one of those ports —
/// `ProviderIdentityLink` — answers "who is behind this provider account" from
/// two places: the directory link a person establishes, and the account behind a
/// stored credential.
pub struct UserDirectory {
    users: Arc<dyn UserDao>,
    tokens: Arc<dyn ProviderTokenDao>,
}

impl UserDirectory {
    pub fn new(users: Arc<dyn UserDao>, tokens: Arc<dyn ProviderTokenDao>) -> Self {
        Self { users, tokens }
    }

    /// Resolve an actor id string to its row. A string that is neither a uuid
    /// nor a legacy provider id is an error, so callers can tell it apart from
    /// a user that does not exist.
    async fn actor(&self, user_id: &str) -> Result<Option<User>, StoreError> {
        let Ok(uuid) = user_id.parse::<Uuid>() else {
            return Err(StoreError::new(format!("'{user_id}' is not a user id")));
        };
        self.users
            .get_user(&uuid)
            .await
            .map_err(|e| StoreError::with_source(format!("look up user {user_id}"), e))
    }
}

fn display_name(user: User) -> String {
    user.name.unwrap_or(user.login)
}

#[async_trait]
impl UserDisplay for UserDirectory {
    async fn names(&self, user_ids: &[Uuid]) -> Result<HashMap<Uuid, String>, StoreError> {
        if user_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let users = self
            .users
            .get_users_by_ids(user_ids)
            .await
            .map_err(|e| StoreError::with_source("batch look up user display names", e))?;
        Ok(users.into_iter().map(|u| (u.id, display_name(u))).collect())
    }

    async fn name(&self, user_id: &Uuid) -> Result<Option<String>, StoreError> {
        let user = self
            .users
            .get_user(user_id)
            .await
            .map_err(|e| StoreError::with_source(format!("look up user {user_id}"), e))?;
        Ok(user.map(display_name))
    }

    async fn email(&self, user_id: &Uuid) -> Result<Option<String>, StoreError> {
        let user = self
            .users
            .get_user(user_id)
            .await
            .map_err(|e| StoreError::with_source(format!("look up user {user_id}"), e))?;
        Ok(user.and_then(|u| u.email))
    }
}

#[async_trait]
impl UserResolver for UserDirectory {
    async fn canonical_id(&self, reference: &str) -> Result<Option<Uuid>, StoreError> {
        let user = match reference.parse::<Uuid>() {
            Ok(id) => self.users.get_user(&id).await,
            Err(_) => self.users.find_user_by_legacy_id(reference).await,
        }
        .map_err(|e| StoreError::with_source(format!("resolve user reference {reference}"), e))?;
        Ok(user.map(|u| u.id))
    }

    async fn run_as_target(&self, reference: &str) -> Result<Option<RunAsUser>, StoreError> {
        // A login is tried only when the reference is not a uuid, so a login
        // shaped like one still resolves to the row it names.
        let user = match reference.parse::<Uuid>() {
            Ok(id) => self.users.get_user(&id).await,
            Err(_) => self.users.get_user_by_login(reference).await,
        }
        .map_err(|e| StoreError::with_source(format!("resolve run-as target {reference}"), e))?;
        Ok(user.map(|u| RunAsUser {
            id: u.id,
            login: u.login,
            is_bot: u.is_bot,
        }))
    }
}

#[async_trait]
impl CommitIdentity for UserDirectory {
    async fn git_identity(&self, user_id: &str) -> Result<Option<GitIdentity>, StoreError> {
        Ok(self.actor(user_id).await?.map(|u| GitIdentity {
            is_bot: u.is_bot,
            login: u.login,
            name: u.name,
            email: u.email,
        }))
    }

    async fn is_platform_admin(&self, user_id: &str) -> Result<bool, StoreError> {
        Ok(matches!(self.actor(user_id).await?, Some(u) if u.role == UserRole::Admin))
    }
}

/// The `provider_tokens` table, narrowed to the access tokens an agent harness
/// runs with.
pub struct HarnessTokens {
    tokens: Arc<dyn ProviderTokenDao>,
}

impl HarnessTokens {
    pub fn new(tokens: Arc<dyn ProviderTokenDao>) -> Self {
        Self { tokens }
    }
}

#[async_trait]
impl HarnessCredentials for HarnessTokens {
    async fn codex_blob(&self, scope: &CodexScope) -> Result<Option<String>, StoreError> {
        let row = self
            .tokens
            .find_codex_blob(scope)
            .await
            .map_err(|e| StoreError::with_source(format!("read codex blob at {scope}"), e))?;
        Ok(row.map(|r| r.access_token))
    }

    async fn provider_token(
        &self,
        user_id: &str,
        provider: &str,
    ) -> Result<Option<String>, StoreError> {
        let row = self
            .tokens
            .find_provider_token(user_id, provider)
            .await
            .map_err(|e| {
                StoreError::with_source(format!("read {provider} token for user {user_id}"), e)
            })?;
        Ok(row.map(|r| r.access_token))
    }
}

#[async_trait]
impl ProviderIdentityLink for UserDirectory {
    async fn user_at(
        &self,
        provider: &str,
        provider_user_id: &str,
    ) -> Result<Option<LinkedUser>, StoreError> {
        let user = self
            .users
            .find_user_by_provider_id(provider, provider_user_id)
            .await
            .map_err(|e| {
                StoreError::with_source(
                    format!("read the user at {provider}:{provider_user_id}"),
                    e,
                )
            })?;
        Ok(user.map(|user| LinkedUser {
            id: user.id,
            login: user.login,
        }))
    }

    async fn user_by_email(&self, email: &str) -> Result<Option<LinkedUser>, StoreError> {
        let user = self
            .users
            .find_user_by_email(email)
            .await
            .map_err(|e| StoreError::with_source("read the user at an email address", e))?;
        Ok(user.map(|user| LinkedUser {
            id: user.id,
            login: user.login,
        }))
    }

    async fn provider_id_for_login(
        &self,
        provider: &str,
        login: &str,
    ) -> Result<Option<String>, StoreError> {
        let Some(user) =
            self.users.get_user_by_login(login).await.map_err(|e| {
                StoreError::with_source(format!("read the user with login {login}"), e)
            })?
        else {
            return Ok(None);
        };
        self.users
            .get_provider_id(&user.id, provider)
            .await
            .map_err(|e| {
                StoreError::with_source(
                    format!("read the {provider} id for user with login {login}"),
                    e,
                )
            })
    }

    async fn user_who_connected(
        &self,
        provider: &str,
        provider_user_id: &str,
    ) -> Result<Option<String>, StoreError> {
        self.tokens
            .find_user_by_provider_account(provider, provider_user_id)
            .await
            .map_err(|e| {
                StoreError::with_source(
                    format!("read the user who connected {provider}:{provider_user_id}"),
                    e,
                )
            })
    }

    async fn user_with_login(
        &self,
        provider: &str,
        login: &str,
    ) -> Result<Option<String>, StoreError> {
        self.tokens
            .find_user_by_provider_login(provider, login)
            .await
            .map_err(|e| {
                StoreError::with_source(
                    format!("read the user who connected as {provider}:{login}"),
                    e,
                )
            })
    }

    async fn link(
        &self,
        user_id: &Uuid,
        provider: &str,
        provider_user_id: &str,
    ) -> Result<bool, StoreError> {
        self.users
            .insert_provider_id_if_absent(user_id, provider, provider_user_id)
            .await
            .map_err(|e| {
                StoreError::with_source(
                    format!("link user {user_id} to {provider}:{provider_user_id}"),
                    e,
                )
            })
    }
}

#[async_trait]
impl AppCredentials for HarnessTokens {
    async fn app_token(&self, provider: &str) -> Result<Option<AppToken>, StoreError> {
        let row =
            self.tokens.find_app_token(provider).await.map_err(|e| {
                StoreError::with_source(format!("read the {provider} app token"), e)
            })?;
        Ok(row.map(|row| AppToken {
            access_token: row.access_token,
            refresh_token: row.refresh_token,
            token_type: row.token_type,
            expires_at: row.expires_at,
            metadata: row.metadata,
        }))
    }

    async fn store_app_token(&self, provider: &str, token: &AppToken) -> Result<(), StoreError> {
        self.tokens
            .upsert_app_token(
                provider,
                &token.access_token,
                token.refresh_token.as_deref(),
                token.token_type.as_deref(),
                token.metadata.as_ref(),
                token.expires_at,
            )
            .await
            .map_err(|e| StoreError::with_source(format!("write the {provider} app token"), e))
    }
}

#[async_trait]
impl CodexScopeReader for HarnessTokens {
    async fn healthy_scopes(&self) -> Result<Vec<CodexScope>, StoreError> {
        self.tokens
            .healthy_codex_scopes()
            .await
            .map_err(|e| StoreError::with_source("list the healthy codex scopes", e))
    }
}

/// The `secrets` table is behind [`SecretService`], which owns the scope walk,
/// the envelope decrypt and the use-marking. Both seams narrow that service
/// rather than the store, so a peer never gets a path around the walk.
#[async_trait]
impl SecretResolver for SecretService {
    async fn plaintext(&self, user_id: &str) -> Result<HashMap<String, String>, StoreError> {
        self.resolve_accessible_plaintext(user_id)
            .await
            .map_err(|e| StoreError::with_source(format!("resolve secrets for {user_id}"), e))
    }

    async fn ssh_keys(&self, user_id: &str) -> Result<HashMap<String, String>, StoreError> {
        self.resolve_accessible_ssh_keys(user_id)
            .await
            .map_err(|e| StoreError::with_source(format!("resolve ssh keys for {user_id}"), e))
    }
}

#[async_trait]
impl SecretCatalog for SecretService {
    async fn list(&self, ctx: &ScopeContext) -> Result<Vec<String>, StoreError> {
        self.visible_keys(ctx)
            .await
            .map_err(|e| StoreError::with_source("list the visible secret keys", e))
    }
}

/// The `adapter_apps` and `adapter_app_secrets` tables, narrowed to the one
/// read an Adapter makes. Holds `Config` because the envelope key that decrypts
/// a stored secret comes off it.
pub struct AdapterApps {
    db: DbPool,
    config: Arc<Config>,
}

impl AdapterApps {
    pub fn new(db: DbPool, config: Arc<Config>) -> Self {
        Self { db, config }
    }
}

#[async_trait]
impl AdapterAppReader for AdapterApps {
    async fn app(
        &self,
        adapter_kind: &str,
        app_key: &str,
    ) -> Result<Option<AdapterAppSecrets>, StoreError> {
        crate::adapter_apps::stored_app(&self.db, &self.config, adapter_kind, app_key)
            .await
            .map_err(|e| {
                StoreError::with_source(format!("read the {adapter_kind} app '{app_key}'"), e)
            })
    }
}
