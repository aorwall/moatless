//! Identity capabilities, built over the shared pool.
//!
//! A host passes the pool and takes a capability handle. The stores behind each
//! handle stay private to this crate.

use std::sync::Arc;

use moatless_core::db::DbPool;
use moatless_core::http_auth::UserTokenVerifier;
use moatless_types::auth::AuthConfig;

use crate::adapter_apps::dao::{AdapterAppDao, PgAdapterAppDao};
use crate::auth::api_keys_dao::PgApiKeyDao;
use crate::auth::github::registry::GitHubAppRegistry;
use crate::auth::github::service::GitHubAppService;
use crate::auth::pairing::DevicePairing;
use crate::auth::pairing_dao::{PgPairingApiKeyIssuer, PgPairingCodeDao};
use crate::auth::password::bootstrap::BootstrapError;
use crate::auth::password::dao::PgUserCredentialDao;
use crate::auth::provider_token_auth_dao::PgProviderTokenAuthDao;
use crate::auth::provider_token_dao::PgProviderTokenDao;
use crate::auth::users_dao::PgUserDao;
use crate::credentials::CredentialService;
use crate::forgejo::access::ForgejoAccess;
use crate::forgejo::instances::ForgejoInstances;
use crate::git_host_access::{GitHostAccess, GitHostTokenValidator};
use crate::git_host_tokens::GitHostTokens;
use crate::ports::{HarnessTokens, UserDirectory};
use crate::secret::audit::SecretAudit;
use crate::secret::audit_dao::PgSecretAuditDao;
use crate::secret::dao::PgSecretDao;
use crate::secret::service::{SecretCipher, SecretService};
use crate::team::dao::PgTeamDao;
use moatless_ports::workspace::WorkspaceCredentialReader;

/// The `users` and `provider_tokens` tables behind the identity ports peers read.
pub fn user_directory(db: DbPool) -> Arc<UserDirectory> {
    Arc::new(UserDirectory::new(
        Arc::new(PgUserDao(db.clone())),
        Arc::new(PgProviderTokenDao(db)),
    ))
}

/// The one way to read, persist and refresh a provider-token credential.
///
/// One instance per process: the per-`(user, key)` lock that serializes a
/// concurrent store lives inside it, so a second instance over the same pool
/// serializes nothing.
pub fn credentials(
    db: DbPool,
    secrets: Arc<SecretService>,
    codex_http: Arc<reqwest::Client>,
) -> Arc<CredentialService> {
    Arc::new(
        CredentialService::new(Arc::new(PgProviderTokenDao(db)))
            .with_codex_refresh(codex_http)
            .with_secrets(secrets),
    )
}

/// The settings-facing view of a caller's git-host connection: what it is
/// connected as, and the Personal Access Token that overrides it.
pub fn git_host_access(
    db: DbPool,
    secrets: Arc<SecretService>,
    validator: Arc<dyn GitHostTokenValidator>,
    app_connect_available: bool,
) -> Arc<GitHostAccess> {
    Arc::new(GitHostAccess::new(
        Arc::new(PgProviderTokenDao(db)),
        secrets,
        validator,
        app_connect_available,
    ))
}

/// The `provider_tokens` table narrowed to what an agent harness runs with.
pub fn harness_credentials(db: DbPool) -> Arc<HarnessTokens> {
    Arc::new(HarnessTokens::new(Arc::new(PgProviderTokenDao(db))))
}

/// The one way to a git-host token, over this crate's own provider-token stores.
///
/// `workspace_credentials` is `None` in a deployment that wires no workspace
/// credential store, which resolves every identity to its own credential.
pub fn git_host_tokens(
    db: DbPool,
    github_app_service: Option<Arc<GitHubAppService>>,
    github_apps: Arc<GitHubAppRegistry>,
    secrets: Arc<SecretService>,
    workspace_credentials: Option<Arc<dyn WorkspaceCredentialReader>>,
    forgejo: Arc<ForgejoInstances>,
) -> Arc<GitHostTokens> {
    let tokens = GitHostTokens::new(
        Arc::new(PgProviderTokenDao(db.clone())),
        Arc::new(PgProviderTokenAuthDao(db)),
        github_app_service,
        github_apps,
        secrets,
    )
    .with_forgejo(forgejo);
    Arc::new(match workspace_credentials {
        Some(reader) => tokens.with_workspace_credentials(reader),
        None => tokens,
    })
}

/// The registered Forgejo OAuth2 applications, one per instance.
pub fn forgejo_instances(
    db: DbPool,
    cipher: Option<Arc<dyn SecretCipher>>,
) -> Arc<ForgejoInstances> {
    Arc::new(ForgejoInstances::new(
        crate::forgejo::dao::PgForgejoInstanceDao(db),
        cipher,
    ))
}

/// One person's Forgejo credential, per instance: connect, disconnect, status.
pub fn forgejo_access(
    db: DbPool,
    secrets: Arc<SecretService>,
    instances: Arc<ForgejoInstances>,
) -> Arc<ForgejoAccess> {
    Arc::new(ForgejoAccess::new(
        Arc::new(PgProviderTokenDao(db)),
        secrets,
        instances,
    ))
}

/// A harness-credential handle over a caller-supplied token store, for a
/// consumer's tests.
// `test` stays first so the census keeps scoring this fake as test-context; the
// feature exposes it to other crates' tests (see Cargo.toml `test-support`).
#[cfg(any(test, feature = "test-support"))]
pub fn harness_over(
    tokens: Arc<crate::auth::test_support::StubProviderTokens>,
) -> Arc<HarnessTokens> {
    Arc::new(HarnessTokens::new(tokens))
}

/// Secret validation, encryption and resolution over the two secret tables.
///
/// `cipher` is `None` in a deployment that configures no encryption key, which
/// leaves every write of a secret value rejected.
pub fn secret_service(db: DbPool, cipher: Option<Arc<dyn SecretCipher>>) -> Arc<SecretService> {
    Arc::new(SecretService::new(
        Arc::new(PgSecretDao(db.clone())),
        secret_audit(db),
        cipher,
    ))
}

/// The one way to append to `secret_audit_events`.
pub fn secret_audit(db: DbPool) -> Arc<SecretAudit> {
    Arc::new(SecretAudit::new(Arc::new(PgSecretAuditDao(db))))
}

/// The registry of configured GitHub Apps: the deployment's login App from
/// `config`, plus the admin-registered Apps in the `adapter_apps` tables.
///
/// `db` is `None` in a process with no database, which leaves the login App as
/// the whole registry. `cipher` opens a stored App's signing key.
pub fn github_app_registry(
    config: &moatless_core::config::Config,
    db: Option<DbPool>,
    cipher: Option<Arc<dyn SecretCipher>>,
) -> Arc<GitHubAppRegistry> {
    Arc::new(GitHubAppRegistry::new(
        GitHubAppService::from_config(config).map(Arc::new),
        db.map(|db| Arc::new(PgAdapterAppDao(db)) as Arc<dyn AdapterAppDao>),
        cipher,
    ))
}

/// Mints and redeems the codes that pair a client on another device.
pub fn device_pairing(db: DbPool) -> Arc<DevicePairing> {
    Arc::new(DevicePairing::new(
        Arc::new(PgPairingCodeDao(db.clone())),
        Arc::new(PgPairingApiKeyIssuer(Arc::new(PgApiKeyDao(db)))),
    ))
}

/// The team routes and their OpenAPI metadata, over this crate's own store.
pub fn team_router(
    db: DbPool,
    token_verifier: Arc<dyn UserTokenVerifier>,
) -> (axum::Router, utoipa::openapi::OpenApi) {
    crate::team::routes::team_router(crate::team::state::TeamState {
        teams: Arc::new(PgTeamDao(db)),
        token_verifier,
    })
}

/// Seed the bootstrap administrator for local password auth.
///
/// See [`crate::auth::password::bootstrap`] for what an existing credential
/// does to the seed.
pub async fn ensure_local_admin(db: DbPool, config: &AuthConfig) -> Result<(), BootstrapError> {
    crate::auth::password::bootstrap::ensure_local_admin(
        &PgUserDao(db.clone()),
        &PgUserCredentialDao(db),
        config,
    )
    .await
}

/// Seed the programmatic test user that owns `raw_key`.
pub async fn ensure_test_user(db: DbPool, raw_key: &str) {
    crate::auth::test_user::ensure_test_user(&PgUserDao(db.clone()), &PgApiKeyDao(db), raw_key)
        .await;
}

/// A local password account that can log in, for a consumer's PostgreSQL tests.
///
/// The account holds `password` and its email is `<login>@example.com`.
#[cfg(any(test, feature = "test-support"))]
#[expect(
    clippy::expect_used,
    reason = "a database fixture fails the test loudly"
)]
pub async fn seed_local_account(
    db: DbPool,
    login: &str,
    role: moatless_types::user::UserRole,
    password: &str,
) -> uuid::Uuid {
    use crate::auth::password::dao::UserCredentialDao;
    use crate::auth::users_dao::UserDao;

    let id = uuid::Uuid::new_v4();
    let now = chrono::Utc::now();
    PgUserDao(db.clone())
        .upsert_user(&crate::auth::users::User {
            id,
            login: login.to_string(),
            email: Some(format!("{login}@example.com")),
            name: None,
            avatar_url: None,
            provider: "local".to_string(),
            provider_user_id: None,
            role,
            created_at: now,
            last_login_at: now,
            settings: serde_json::json!({}),
            is_bot: false,
            status: moatless_types::user::UserStatus::Active,
        })
        .await
        .expect("the test account is written");
    let encoded = crate::auth::password::hash::hash_password(password.to_string())
        .await
        .expect("the test password is hashed");
    PgUserCredentialDao(db)
        .upsert(&id, login, &encoded, false)
        .await
        .expect("the test credential is written");
    id
}

/// The `user_credentials` row for `user_id`, for a consumer's PostgreSQL tests.
#[cfg(any(test, feature = "test-support"))]
#[expect(
    clippy::expect_used,
    reason = "a database fixture fails the test loudly"
)]
pub async fn local_credential(
    db: DbPool,
    user_id: &uuid::Uuid,
) -> Option<crate::auth::password::models::UserCredential> {
    use crate::auth::password::dao::UserCredentialDao;

    PgUserCredentialDao(db)
        .find_by_user_id(user_id)
        .await
        .expect("the credential read succeeds")
}

/// A fresh API key for `user_id`, for a consumer's PostgreSQL tests. Returns the
/// raw key a caller sends as a bearer token.
#[cfg(any(test, feature = "test-support"))]
#[expect(
    clippy::expect_used,
    reason = "a database fixture fails the test loudly"
)]
pub async fn issue_api_key(db: DbPool, user_id: &str, name: &str) -> String {
    crate::auth::api_key::generate_key(&PgApiKeyDao(db), user_id, name, None, None, None)
        .await
        .expect("the test API key is minted")
        .key
}
