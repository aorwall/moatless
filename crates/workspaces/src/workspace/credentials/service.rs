//! Workspace credential access control, encryption, and resolution.

use std::collections::HashMap;
use std::sync::Arc;

use tracing::{info, warn};

use moatless_identity::secret::audit::{AuditOutcome, CredentialAction, SecretAudit};
use moatless_identity::secret::service::SecretCipher;
use moatless_types::access_control::{ScopeContext, ScopedResource};

use super::dao::WorkspaceCredentialDao;
use super::models::{WorkspaceCredential, WorkspaceCredentialMetadata, WorkspaceCredentialWrite};
use crate::workspace::dao::WorkspaceDao;
use crate::workspace::error::WorkspaceError;
use crate::workspace::models::Workspace;

use moatless_types::workspace::WorkspaceCredentialProvider;

/// Reads, writes and resolves the provider credentials pinned to a Workspace.
///
/// Writes are gated on control of the owning workspace — the same predicate
/// that gates editing its run-config, because a credential *is* run-config with
/// a stricter blast radius. [`Self::resolve_env`] is the sandbox path and takes
/// no actor: by the time a task is being provisioned, the task's own
/// authorization already decided the workspace may be run.
pub struct WorkspaceCredentialService {
    credentials: Arc<dyn WorkspaceCredentialDao>,
    workspaces: Arc<dyn WorkspaceDao>,
    audits: Arc<SecretAudit>,
    /// Envelope cipher. `None` when the deployment configured no
    /// `SECRETS_ENCRYPTION_KEY`, which makes every path here a 503 rather than
    /// a silently unencrypted write.
    cipher: Option<Arc<dyn SecretCipher>>,
}

impl WorkspaceCredentialService {
    pub(crate) fn new(
        credentials: Arc<dyn WorkspaceCredentialDao>,
        workspaces: Arc<dyn WorkspaceDao>,
        audits: Arc<SecretAudit>,
        cipher: Option<Arc<dyn SecretCipher>>,
    ) -> Self {
        Self {
            credentials,
            workspaces,
            audits,
            cipher,
        }
    }

    /// Credential metadata for a workspace the caller may read. Values are
    /// never included.
    pub async fn list(
        &self,
        ctx: &ScopeContext,
        workspace_id: &str,
    ) -> Result<Vec<WorkspaceCredentialMetadata>, WorkspaceError> {
        self.readable_workspace(ctx, workspace_id).await?;
        let rows = self.credentials.list(workspace_id).await?;
        Ok(rows.iter().filter_map(metadata).collect())
    }

    /// Pin `value` to `(workspace, provider)`, replacing any current value.
    pub async fn set(
        &self,
        ctx: &ScopeContext,
        workspace_id: &str,
        provider: WorkspaceCredentialProvider,
        value: &str,
    ) -> Result<WorkspaceCredentialMetadata, WorkspaceError> {
        if value.trim().is_empty() {
            return Err(WorkspaceError::BadRequest(
                "Credential value must not be empty".to_string(),
            ));
        }
        self.controllable_workspace(ctx, workspace_id, provider, CredentialAction::Set)
            .await?;

        let cipher = self.cipher()?;
        // Reuse the existing row's id: it is the additional authenticated data
        // the ciphertext is bound to, so a replacement value must be encrypted
        // under the same id the upsert will keep.
        let existing = self
            .credentials
            .find(workspace_id, provider.as_str())
            .await?;
        let id = existing.map_or_else(
            || format!("wsc_{}", uuid::Uuid::new_v4()),
            |row| row.id.clone(),
        );
        let encrypted = cipher
            .encrypt(&id, value)
            .map_err(|e| crypto_error("encrypt", workspace_id, provider, &e))?;

        let row = self
            .credentials
            .upsert(WorkspaceCredentialWrite {
                id,
                workspace_id: workspace_id.to_string(),
                provider: provider.as_str().to_string(),
                value: encrypted,
                actor_id: ctx.user_id.clone(),
            })
            .await?;

        self.audits
            .record_workspace_credential(
                &ctx.user_id,
                CredentialAction::Set,
                workspace_id,
                provider,
                AuditOutcome::Success,
            )
            .await;
        info!(
            workspace_id,
            provider = provider.as_str(),
            actor_id = %ctx.user_id,
            "Pinned provider credential to workspace"
        );

        metadata(&row).ok_or_else(|| {
            WorkspaceError::BadRequest(format!(
                "Stored credential has an unsupported provider '{}'",
                row.provider
            ))
        })
    }

    /// Remove a pinned credential. The workspace falls back to the run-as
    /// user's credential from the next sandbox start.
    pub async fn delete(
        &self,
        ctx: &ScopeContext,
        workspace_id: &str,
        provider: WorkspaceCredentialProvider,
    ) -> Result<(), WorkspaceError> {
        self.controllable_workspace(ctx, workspace_id, provider, CredentialAction::Delete)
            .await?;

        if !self
            .credentials
            .delete(workspace_id, provider.as_str())
            .await?
        {
            return Err(WorkspaceError::NotFound(format!(
                "No {provider} credential on workspace '{workspace_id}'"
            )));
        }

        self.audits
            .record_workspace_credential(
                &ctx.user_id,
                CredentialAction::Delete,
                workspace_id,
                provider,
                AuditOutcome::Success,
            )
            .await;
        info!(
            workspace_id,
            provider = provider.as_str(),
            actor_id = %ctx.user_id,
            "Removed provider credential from workspace"
        );
        Ok(())
    }

    /// The credentials a sandbox on this workspace runs with, as an
    /// environment-variable map.
    ///
    /// Fails rather than returning a partial map. A workspace credential exists
    /// to make a task run as a *specific* account, so degrading to "whatever
    /// the run-as user holds" would hand the task a different identity than the
    /// one it was configured with — the failure mode this feature is meant to
    /// remove. Preflight turns the error into a refusal to start; the sandbox's
    /// `GetCredentials` turns it into a failed fetch.
    pub async fn resolve_env(
        &self,
        workspace_id: &str,
    ) -> Result<HashMap<String, String>, WorkspaceError> {
        let rows = self.credentials.list(workspace_id).await?;
        if rows.is_empty() {
            return Ok(HashMap::new());
        }
        let cipher = self.cipher()?;

        let mut env = HashMap::with_capacity(rows.len());
        for row in &rows {
            let Some(provider) = parse_provider(&row.provider) else {
                // A provider this build does not know about. Skipping is right
                // here and only here: there is no env slot to fill, so nothing
                // downstream can pick up the wrong identity from it.
                warn!(
                    workspace_id,
                    provider = %row.provider,
                    "Ignoring workspace credential for an unknown provider"
                );
                continue;
            };
            let value = cipher
                .decrypt_value(&row.id, &row.encrypted_value())
                .map_err(|e| crypto_error("decrypt", workspace_id, provider, &e))?;
            env.insert(provider.env_key().to_string(), value);
        }
        Ok(env)
    }

    fn cipher(&self) -> Result<&Arc<dyn SecretCipher>, WorkspaceError> {
        self.cipher
            .as_ref()
            .ok_or(WorkspaceError::EncryptionNotConfigured)
    }

    /// The workspace, if the caller may read it.
    async fn readable_workspace(
        &self,
        ctx: &ScopeContext,
        workspace_id: &str,
    ) -> Result<Workspace, WorkspaceError> {
        self.workspaces
            .find_by_id(workspace_id, ctx)
            .await?
            .ok_or_else(|| WorkspaceError::NotFound(workspace_id.to_string()))
    }

    /// The same read, plus the control check every write needs. A denied write
    /// is audited — an attempt to point a workspace at someone else's token is
    /// exactly the event the trail exists for.
    async fn controllable_workspace(
        &self,
        ctx: &ScopeContext,
        workspace_id: &str,
        provider: WorkspaceCredentialProvider,
        action: CredentialAction,
    ) -> Result<Workspace, WorkspaceError> {
        let workspace = self.readable_workspace(ctx, workspace_id).await?;
        let resource = ScopedResource::borrowed(
            workspace.scope,
            workspace
                .owner_user_id
                .as_deref()
                .or(workspace.created_by.as_deref()),
        );
        if !ctx.can_write(&resource) {
            self.audits
                .record_workspace_credential(
                    &ctx.user_id,
                    action,
                    workspace_id,
                    provider,
                    AuditOutcome::Denied,
                )
                .await;
            return Err(WorkspaceError::Forbidden(format!(
                "No control access to workspace '{workspace_id}'"
            )));
        }
        Ok(workspace)
    }
}

fn parse_provider(value: &str) -> Option<WorkspaceCredentialProvider> {
    value.parse().ok()
}

/// Metadata for a stored row, or `None` when the row names a provider this
/// build does not know.
fn metadata(row: &WorkspaceCredential) -> Option<WorkspaceCredentialMetadata> {
    let provider = parse_provider(&row.provider)?;
    Some(WorkspaceCredentialMetadata {
        workspace_id: row.workspace_id.clone(),
        provider,
        env_key: provider.env_key().to_string(),
        created_by: row.created_by.clone(),
        updated_by: row.updated_by.clone(),
        created_at: row.created_at.to_rfc3339(),
        updated_at: row.updated_at.to_rfc3339(),
    })
}

/// Log the cause once, return the opaque error. The cipher's own errors carry
/// no detail worth forwarding to a client, and the operator needs to know which
/// row failed.
fn crypto_error(
    operation: &str,
    workspace_id: &str,
    provider: WorkspaceCredentialProvider,
    error: &moatless_identity::secret::error::SecretError,
) -> WorkspaceError {
    tracing::error!(
        error = %error,
        operation,
        workspace_id,
        provider = provider.as_str(),
        "Workspace credential crypto error"
    );
    WorkspaceError::Crypto
}

#[cfg(test)]
mod tests {
    use super::*;

    use async_trait::async_trait;
    use chrono::Utc;
    use moatless_identity::secret::service::LocalEnvelopeSecretCipher;
    use moatless_identity::secret::test_support::noop_audit;
    use moatless_types::access_control::Scope;
    use moatless_types::secret::SecretsConfig;
    use std::sync::Mutex;

    use crate::workspace::dao::MockWorkspaceDao;

    // --- fakes -------------------------------------------------------------

    #[derive(Default)]
    struct FakeCredentials {
        rows: Mutex<Vec<WorkspaceCredential>>,
    }

    #[async_trait]
    impl WorkspaceCredentialDao for FakeCredentials {
        async fn list(&self, workspace_id: &str) -> Result<Vec<WorkspaceCredential>, sqlx::Error> {
            let rows = self.rows.lock().unwrap();
            Ok(rows
                .iter()
                .filter(|r| r.workspace_id == workspace_id)
                .cloned()
                .collect())
        }

        async fn find(
            &self,
            workspace_id: &str,
            provider: &str,
        ) -> Result<Option<WorkspaceCredential>, sqlx::Error> {
            let rows = self.rows.lock().unwrap();
            Ok(rows
                .iter()
                .find(|r| r.workspace_id == workspace_id && r.provider == provider)
                .cloned())
        }

        async fn upsert(
            &self,
            write: WorkspaceCredentialWrite,
        ) -> Result<WorkspaceCredential, sqlx::Error> {
            let mut rows = self.rows.lock().unwrap();
            let row = WorkspaceCredential {
                id: write.id,
                workspace_id: write.workspace_id,
                provider: write.provider,
                value_ciphertext: write.value.value_ciphertext,
                encryption_nonce: write.value.encryption_nonce,
                encrypted_data_key: write.value.encrypted_data_key,
                kms_key_id: write.value.kms_key_id,
                encryption_version: write.value.encryption_version,
                created_by: write.actor_id.clone(),
                updated_by: write.actor_id,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            };
            rows.retain(|r| !(r.workspace_id == row.workspace_id && r.provider == row.provider));
            rows.push(row.clone());
            Ok(row)
        }

        async fn delete(&self, workspace_id: &str, provider: &str) -> Result<bool, sqlx::Error> {
            let mut rows = self.rows.lock().unwrap();
            let before = rows.len();
            rows.retain(|r| !(r.workspace_id == workspace_id && r.provider == provider));
            Ok(rows.len() != before)
        }
    }

    /// A `WorkspaceDao` serving one row, to whoever the read rule admits.
    fn workspaces_holding(ws: Workspace) -> Arc<dyn WorkspaceDao> {
        let mut mock = MockWorkspaceDao::new();
        mock.expect_find_by_id().returning(move |id, ctx| {
            if id != ws.id {
                return Ok(None);
            }
            let resource = ScopedResource::borrowed(
                ws.scope,
                ws.owner_user_id.as_deref().or(ws.created_by.as_deref()),
            );
            Ok(ctx.can_read(&resource).then(|| ws.clone()))
        });
        Arc::new(mock)
    }

    // --- fixtures ----------------------------------------------------------

    fn workspace(owner: &str) -> Workspace {
        Workspace {
            id: "ws_1".to_string(),
            name: "ws".to_string(),
            description: None,
            icon: None,
            default_model: None,
            default_effort: None,
            kind: moatless_types::workspace::WorkspaceKind::Single,
            docker_image: None,
            docker_access: None,
            sandbox_service_account: None,
            system_prompt: None,
            setup_commands: None,
            env_vars: None,
            servers: None,
            scripts: None,
            resources: None,
            network_profile: None,
            source: moatless_types::workspace::WorkspaceSource::Manual,
            source_repository_id: None,
            source_key: None,
            source_config_path: None,
            scope: Scope::User,
            run_as_user_id: None,
            owner_user_id: Some(owner.to_string()),
            created_by: Some(owner.to_string()),
            deleted: false,
            created_at: None,
            updated_at: None,
        }
    }

    fn cipher() -> Option<Arc<dyn SecretCipher>> {
        LocalEnvelopeSecretCipher::from_config(&SecretsConfig {
            encryption_key: Some("0123456789abcdef0123456789abcdef".into()),
            kms_key_id: None,
        })
    }

    fn service(cipher: Option<Arc<dyn SecretCipher>>) -> WorkspaceCredentialService {
        WorkspaceCredentialService::new(
            Arc::new(FakeCredentials::default()),
            workspaces_holding(workspace("owner")),
            noop_audit(),
            cipher,
        )
    }

    fn ctx(user_id: &str) -> ScopeContext {
        ScopeContext {
            user_id: user_id.to_string(),
            is_admin: false,
        }
    }

    // --- tests -------------------------------------------------------------

    #[tokio::test]
    async fn a_stored_credential_resolves_to_its_env_var() {
        let svc = service(cipher());
        svc.set(
            &ctx("owner"),
            "ws_1",
            WorkspaceCredentialProvider::Gitlab,
            "glpat_workspace",
        )
        .await
        .expect("owner may set");

        let env = svc.resolve_env("ws_1").await.expect("resolves");
        assert_eq!(
            env.get("GITLAB_TOKEN").map(String::as_str),
            Some("glpat_workspace")
        );
    }

    #[tokio::test]
    async fn replacing_a_value_re_encrypts_under_the_same_row_and_still_decrypts() {
        // The row id is the ciphertext's AAD. An update that minted a fresh id
        // while the upsert kept the old one would store a value that can never
        // be decrypted again — and the failure would only surface at sandbox
        // start.
        let svc = service(cipher());
        let c = ctx("owner");
        svc.set(&c, "ws_1", WorkspaceCredentialProvider::Gitlab, "first")
            .await
            .unwrap();
        svc.set(&c, "ws_1", WorkspaceCredentialProvider::Gitlab, "second")
            .await
            .unwrap();

        let env = svc.resolve_env("ws_1").await.expect("resolves");
        assert_eq!(env.get("GITLAB_TOKEN").map(String::as_str), Some("second"));
        assert_eq!(env.len(), 1, "replacement must not leave a second row");
    }

    #[tokio::test]
    async fn a_github_credential_resolves_to_the_slot_the_git_host_seam_reads() {
        // `GitHostTokens` looks the pin up by this env key, so a task on the
        // workspace opens pull requests as the pinned account as well as
        // cloning as it.
        let svc = service(cipher());
        svc.set(
            &ctx("owner"),
            "ws_1",
            WorkspaceCredentialProvider::Github,
            "ghp_workspace",
        )
        .await
        .expect("owner may set");

        let env = svc.resolve_env("ws_1").await.expect("resolves");
        assert_eq!(
            env.get("GITHUB_TOKEN").map(String::as_str),
            Some("ghp_workspace")
        );
    }

    #[tokio::test]
    async fn several_providers_resolve_to_their_own_slots() {
        let svc = service(cipher());
        let c = ctx("owner");
        svc.set(&c, "ws_1", WorkspaceCredentialProvider::Gitlab, "glpat")
            .await
            .unwrap();
        svc.set(&c, "ws_1", WorkspaceCredentialProvider::Openai, "oa")
            .await
            .unwrap();

        let env = svc.resolve_env("ws_1").await.unwrap();
        assert_eq!(env.get("GITLAB_TOKEN").map(String::as_str), Some("glpat"));
        assert_eq!(env.get("OPENAI_API_KEY").map(String::as_str), Some("oa"));
    }

    #[tokio::test]
    async fn a_reader_who_cannot_control_the_workspace_cannot_set_a_credential() {
        let svc = service(cipher());
        let err = svc
            .set(
                &ctx("someone-else"),
                "ws_1",
                WorkspaceCredentialProvider::Gitlab,
                "glpat_x",
            )
            .await
            .expect_err("only a controller may pin a credential");
        // A user-scoped workspace is invisible to a non-owner, so the read
        // fails before the control check — either way nothing is written.
        assert!(matches!(
            err,
            WorkspaceError::NotFound(_) | WorkspaceError::Forbidden(_)
        ));
        assert!(svc.resolve_env("ws_1").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_non_owner_admin_may_set_a_credential() {
        let svc = service(cipher());
        let admin = ScopeContext {
            user_id: "root".to_string(),
            is_admin: true,
        };
        svc.set(
            &admin,
            "ws_1",
            WorkspaceCredentialProvider::Gitlab,
            "glpat_a",
        )
        .await
        .expect("admin controls every workspace");
    }

    #[tokio::test]
    async fn deleting_the_credential_empties_the_resolved_env() {
        let svc = service(cipher());
        let c = ctx("owner");
        svc.set(&c, "ws_1", WorkspaceCredentialProvider::Gitlab, "glpat_x")
            .await
            .unwrap();
        svc.delete(&c, "ws_1", WorkspaceCredentialProvider::Gitlab)
            .await
            .unwrap();

        assert!(svc.resolve_env("ws_1").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn deleting_a_credential_that_is_not_set_is_not_found() {
        let svc = service(cipher());
        let err = svc
            .delete(&ctx("owner"), "ws_1", WorkspaceCredentialProvider::Gitlab)
            .await
            .expect_err("nothing to remove");
        assert!(matches!(err, WorkspaceError::NotFound(_)));
    }

    #[tokio::test]
    async fn listing_returns_metadata_and_never_the_value() {
        let svc = service(cipher());
        let c = ctx("owner");
        svc.set(
            &c,
            "ws_1",
            WorkspaceCredentialProvider::Gitlab,
            "ghp_secret",
        )
        .await
        .unwrap();

        let listed = svc.list(&c, "ws_1").await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].provider, WorkspaceCredentialProvider::Gitlab);
        assert_eq!(listed[0].env_key, "GITLAB_TOKEN");
        let response = super::super::handlers::credential_response(listed[0].clone());
        let json = serde_json::to_string(&response).unwrap();
        assert!(!json.contains("ghp_secret"), "{json}");
    }

    #[tokio::test]
    async fn an_empty_value_is_rejected_before_anything_is_stored() {
        let svc = service(cipher());
        let err = svc
            .set(
                &ctx("owner"),
                "ws_1",
                WorkspaceCredentialProvider::Gitlab,
                "  ",
            )
            .await
            .expect_err("an empty token is a misconfiguration, not a credential");
        assert!(matches!(err, WorkspaceError::BadRequest(_)));
    }

    #[tokio::test]
    async fn a_workspace_with_no_credentials_resolves_empty_without_a_cipher() {
        // The common case — no workspace credentials at all — must not depend
        // on encryption being configured, or every sandbox on a deployment
        // without a key would fail to start.
        let svc = service(None);
        assert!(svc.resolve_env("ws_1").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn setting_a_credential_without_a_cipher_is_a_configuration_error() {
        let svc = service(None);
        let err = svc
            .set(
                &ctx("owner"),
                "ws_1",
                WorkspaceCredentialProvider::Gitlab,
                "glpat_x",
            )
            .await
            .expect_err("nothing may be stored unencrypted");
        assert!(matches!(err, WorkspaceError::EncryptionNotConfigured));
    }

    #[tokio::test]
    async fn resolution_fails_loudly_when_a_stored_value_cannot_be_decrypted() {
        // Falling back to the run-as user's token here would run the task as a
        // different account than the workspace was configured with.
        let credentials = Arc::new(FakeCredentials::default());
        let good = WorkspaceCredentialService::new(
            credentials.clone(),
            workspaces_holding(workspace("owner")),
            noop_audit(),
            cipher(),
        );
        good.set(
            &ctx("owner"),
            "ws_1",
            WorkspaceCredentialProvider::Gitlab,
            "glpat_x",
        )
        .await
        .unwrap();

        // A second deployment key cannot open the first one's ciphertext.
        let other_key = LocalEnvelopeSecretCipher::from_config(&SecretsConfig {
            encryption_key: Some("fedcba9876543210fedcba9876543210".into()),
            kms_key_id: None,
        });
        let stale = WorkspaceCredentialService::new(
            credentials,
            workspaces_holding(workspace("owner")),
            noop_audit(),
            other_key,
        );

        assert!(matches!(
            stale.resolve_env("ws_1").await,
            Err(WorkspaceError::Crypto)
        ));
    }
}
