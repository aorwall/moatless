//! Complete a workspace installation in connection-then-token order.
use super::state::Installation;
use moatless_identity::auth::{error::AuthError, oauth::provider::OAuthProvider};
use tracing::info;

impl Installation {
    pub(super) async fn complete_install(&self, code: &str) -> Result<String, AuthError> {
        let provider = self
            .provider
            .as_ref()
            .ok_or_else(|| AuthError::Config("Linear OAuth not configured".into()))?;
        let tokens = provider.exchange_code_for_tokens(code).await?;
        let identity = provider
            .fetch_install_identity(&tokens.access_token)
            .await?;

        persist_install(
            self.connections.as_ref(),
            self.credentials.as_ref(),
            tokens,
            identity,
        )
        .await
    }
}

async fn persist_install(
    connections: &dyn moatless_adapter_model::connections::ConnectionInstaller,
    credentials: &dyn moatless_ports::identity::AppCredentials,
    tokens: moatless_identity::auth::models::OAuthTokens,
    identity: super::client::LinearInstallIdentity,
) -> Result<String, AuthError> {
    use moatless_types::adapter::AdapterConnection;
    let conn_repo = connections;

    let now = chrono::Utc::now();
    let transport_config = serde_json::json!({
        "source": "oauth",
        "agent_viewer_id": identity.agent_viewer_id,
        "agent_viewer_name": identity.agent_viewer_name,
        "organization_name": identity.organization_name,
    });

    let existing = conn_repo
        .by_external_account("linear", &identity.organization_id)
        .await
        .map_err(AuthError::Store)?;

    let connection_row = AdapterConnection {
        id: existing
            .as_ref()
            .map_or_else(|| format!("pc_{}", uuid::Uuid::new_v4()), |c| c.id.clone()),
        adapter_kind: moatless_types::adapter::AdapterKind::Linear,
        connection_kind: "workspace".to_string(),
        scope_value: identity.organization_id.clone(),
        credential_ref: None,
        webhook_secret: existing.as_ref().and_then(|c| c.webhook_secret.clone()),
        transport_config,
        created_by: existing
            .as_ref()
            .map_or_else(|| "system".to_string(), |c| c.created_by.clone()),
        scope: existing
            .as_ref()
            .map_or(moatless_types::access_control::Scope::Global, |c| c.scope),
        owner_user_id: existing.as_ref().and_then(|c| c.owner_user_id.clone()),
        created_at: existing.as_ref().map_or(now, |c| c.created_at),
        updated_at: now,
    };
    conn_repo
        .install(&connection_row)
        .await
        .map_err(AuthError::Store)?;

    // Upsert the workspace token row (global scope: user_id=NULL, team_id=NULL).
    let expires_at = tokens
        .expires_in
        .map(|secs| now + chrono::Duration::seconds(secs as i64));
    let metadata = serde_json::json!({
        "organization_id": identity.organization_id,
        "organization_name": identity.organization_name,
        "agent_viewer_id": identity.agent_viewer_id,
        "agent_viewer_name": identity.agent_viewer_name,
        "agent_viewer_email": identity.agent_viewer_email,
        "scope": tokens.scope,
    });
    credentials
        .store_app_token(
            "linear",
            &moatless_ports::identity::AppToken {
                access_token: tokens.access_token,
                refresh_token: tokens.refresh_token,
                token_type: Some(tokens.token_type),
                metadata: Some(metadata),
                expires_at,
            },
        )
        .await
        .map_err(AuthError::Store)?;

    info!(
        org_id = %identity.organization_id,
        agent_id = %identity.agent_viewer_id,
        "Linear workspace installed"
    );

    Ok(identity.organization_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use moatless_adapter_model::connections::{ConnectionInstaller, ConnectionLookup};
    use moatless_identity::auth::models::OAuthTokens;
    use moatless_ports::identity::{AppCredentials, AppToken};
    use moatless_types::access_control::Scope;
    use moatless_types::adapter::{AdapterConnection, AdapterKind};
    use moatless_types::store::StoreError;
    use std::sync::Mutex;

    struct Store {
        connection: Mutex<Option<AdapterConnection>>,
        token: Mutex<Option<AppToken>>,
        calls: Mutex<Vec<&'static str>>,
        refuse_connection: bool,
    }

    #[async_trait::async_trait]
    impl ConnectionLookup for Store {
        async fn by_external_account(
            &self,
            kind: &str,
            id: &str,
        ) -> Result<Option<AdapterConnection>, StoreError> {
            assert_eq!((kind, id), ("linear", "org-1"));
            self.calls.lock().unwrap().push("lookup");
            Ok(self.connection.lock().unwrap().clone())
        }
        async fn by_scope_value(
            &self,
            _: &str,
            _: &str,
        ) -> Result<Option<AdapterConnection>, StoreError> {
            panic!("unexpected lookup")
        }
        async fn by_id_trusted(&self, _: &str) -> Result<Option<AdapterConnection>, StoreError> {
            panic!("unexpected lookup")
        }
        async fn by_provider(&self, _: &str) -> Result<Vec<AdapterConnection>, StoreError> {
            panic!("unexpected listing")
        }
    }

    #[async_trait::async_trait]
    impl ConnectionInstaller for Store {
        async fn install(&self, row: &AdapterConnection) -> Result<AdapterConnection, StoreError> {
            self.calls.lock().unwrap().push("connection");
            if self.refuse_connection {
                return Err(StoreError::new("connection refused"));
            }
            *self.connection.lock().unwrap() = Some(row.clone());
            Ok(row.clone())
        }
    }

    #[async_trait::async_trait]
    impl AppCredentials for Store {
        async fn app_token(&self, _: &str) -> Result<Option<AppToken>, StoreError> {
            panic!("unexpected token read")
        }
        async fn store_app_token(
            &self,
            provider: &str,
            token: &AppToken,
        ) -> Result<(), StoreError> {
            assert_eq!(provider, "linear");
            self.calls.lock().unwrap().push("token");
            *self.token.lock().unwrap() = Some(token.clone());
            Ok(())
        }
    }

    fn store(refuse_connection: bool) -> Store {
        let now = chrono::Utc::now();
        Store {
            connection: Mutex::new(Some(AdapterConnection {
                id: "pc-existing".into(),
                adapter_kind: AdapterKind::Linear,
                connection_kind: "workspace".into(),
                scope_value: "org-1".into(),
                credential_ref: None,
                webhook_secret: Some("existing-secret".into()),
                transport_config: serde_json::json!({}),
                created_by: "installer".into(),
                scope: Scope::User,
                owner_user_id: Some("owner".into()),
                created_at: now,
                updated_at: now,
            })),
            token: Mutex::new(None),
            calls: Mutex::new(Vec::new()),
            refuse_connection,
        }
    }

    fn tokens() -> OAuthTokens {
        OAuthTokens {
            access_token: "new-access".into(),
            token_type: "Bearer".into(),
            scope: Some("read,write".into()),
            id_token: None,
            refresh_token: Some("new-refresh".into()),
            expires_in: Some(3600),
            refresh_token_expires_in: None,
        }
    }

    fn identity() -> super::super::client::LinearInstallIdentity {
        super::super::client::LinearInstallIdentity {
            organization_id: "org-1".into(),
            organization_name: Some("Organization".into()),
            agent_viewer_id: "agent-1".into(),
            agent_viewer_name: Some("Agent".into()),
            agent_viewer_email: None,
        }
    }

    #[tokio::test]
    async fn reinstall_preserves_connection_ownership_before_replacing_global_token() {
        let store = store(false);
        let created_at = store
            .connection
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .created_at;
        assert_eq!(
            persist_install(&store, &store, tokens(), identity())
                .await
                .unwrap(),
            "org-1"
        );
        assert_eq!(
            *store.calls.lock().unwrap(),
            ["lookup", "connection", "token"]
        );
        let row = store.connection.lock().unwrap().clone().unwrap();
        assert_eq!(row.id, "pc-existing");
        assert_eq!(row.created_by, "installer");
        assert_eq!(row.owner_user_id.as_deref(), Some("owner"));
        assert_eq!(row.scope, Scope::User);
        assert_eq!(row.webhook_secret.as_deref(), Some("existing-secret"));
        assert_eq!(row.created_at, created_at);
        let token = store.token.lock().unwrap().clone().unwrap();
        assert_eq!(token.access_token, "new-access");
        assert_eq!(token.refresh_token.as_deref(), Some("new-refresh"));
        assert_eq!(token.metadata.unwrap()["organization_id"], "org-1");
    }

    #[tokio::test]
    async fn connection_failure_prevents_token_replacement() {
        let store = store(true);
        assert!(
            persist_install(&store, &store, tokens(), identity())
                .await
                .is_err()
        );
        assert_eq!(*store.calls.lock().unwrap(), ["lookup", "connection"]);
        assert!(store.token.lock().unwrap().is_none());
    }
}
