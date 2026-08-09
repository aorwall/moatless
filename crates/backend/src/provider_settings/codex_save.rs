//! Scope-aware Codex `auth.json` save path.
//!
//! Persists the blob addressed by `scope` and broadcasts the rotation to
//! running sandboxes whose effective user resolves through this scope. Parsing
//! and metadata construction live in `auth::codex_auth_json`; centralising the
//! save here keeps the scope-specific HTTP handlers thin and guarantees they
//! apply identical metadata, error mapping, and broadcast behaviour.

use std::sync::Arc;

use moatless_api_schemas::provider_settings::{CodexConfigResponse, SaveCodexAuthRequest};

use crate::auth::codex_auth_json::{build_codex_metadata, parse_codex_auth_json};
use crate::auth::provider_token_dao::ProviderTokenDao;
use crate::auth::codex_scope::CodexScope;
use crate::credential_sync::CredentialSync;
use crate::provider_settings::error::ProviderSettingsError;

/// Shared codex-blob save path used by every scope endpoint (user / admin global).
/// Parses + validates, upserts the row addressed by `scope`, and broadcasts
/// the rotation to running sandboxes whose effective user resolves through
/// this scope.
pub async fn save_codex_auth_for_scope(
    provider_tokens: &Arc<dyn ProviderTokenDao>,
    credential_sync: &Arc<CredentialSync>,
    scope: CodexScope,
    body: SaveCodexAuthRequest,
) -> Result<CodexConfigResponse, ProviderSettingsError> {
    let parsed = parse_codex_auth_json(&body.auth_json)?;
    let metadata = build_codex_metadata(&parsed);
    let serialized = parsed.to_string();

    // Persist the row first. `upsert_codex_blob` carries `token_type` and the
    // Codex-account `metadata`; those columns are the reason this write cannot
    // be delegated to `store`, whose upsert passes `None`/`None`/`None` for
    // exactly them (the double write is a column mismatch, not redundancy).
    provider_tokens
        .upsert_codex_blob(&scope, &serialized, None, Some("auth_json"), Some(&metadata))
        .await?;

    // Fan the already-persisted rotation out to every running sandbox whose
    // effective user resolves through this scope. Broadcast directly rather
    // than through `store`: the row is already written above, so `store` would
    // read it back, dedup on the identical value, and suppress the very
    // broadcast this save exists to send. The echo loop terminates one hop in
    // — a sandbox that applies the push and echoes it back arrives as
    // `store(.., SandboxRotation)`, which dedups on the same value (and, on a
    // changed value, excludes the originating task) and does not re-broadcast.
    credential_sync
        .broadcast_codex_rotation(&scope, &serialized)
        .await;

    Ok(CodexConfigResponse {
        connected: true,
        token_type: Some("auth_json".to_string()),
        auth_method: Some("auth_json".to_string()),
        email: None,
        plan_type: None,
        needs_reconnect: Some(false),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::HashMap;

    use async_trait::async_trait;
    use moatless_backend_core::sandbox::{SandboxClient, SandboxConnector, SandboxError};
    use tokio::sync::mpsc;

    use crate::auth::codex_scope::CodexScope;
    use crate::auth::github_scope::GitHubScope;
    use crate::auth::models::{GithubTokenPresence, ProviderAccessToken, ProviderToken};
    use crate::credentials::CredentialService;
    use crate::task::dao::MockTaskDao;

    /// A `ProviderTokenDao` that actually *stores* what it is given and serves
    /// it back — the discriminating property the shared `StubProviderTokens`
    /// (writes accepted-and-dropped, reads canned) deliberately lacks. Only
    /// this write-then-read-back behaviour reproduces the suppression the fix
    /// removes: `store`'s dedup reads the row `upsert_codex_blob` just wrote,
    /// sees an identical value, and returns before the broadcast.
    ///
    /// Keys mirror `PgProviderTokenDao`: user-scope `upsert_codex_blob`
    /// delegates to `upsert_token(user_id, "codex", …)`, and `store` reads
    /// `find_provider_token(user_id, "codex")` — the same slot.
    #[derive(Default)]
    struct WriteHonouringTokens {
        rows: tokio::sync::Mutex<HashMap<(String, String), String>>,
    }

    impl WriteHonouringTokens {
        fn codex_key(scope: &CodexScope) -> (String, String) {
            let user = match scope {
                CodexScope::User(id) => id.clone(),
                CodexScope::Global => String::new(),
            };
            (user, "codex".to_string())
        }
    }

    #[async_trait]
    impl ProviderTokenDao for WriteHonouringTokens {
        async fn find_provider_token(
            &self,
            user_id: &str,
            provider: &str,
        ) -> Result<Option<ProviderAccessToken>, sqlx::Error> {
            Ok(self
                .rows
                .lock()
                .await
                .get(&(user_id.to_string(), provider.to_string()))
                .map(|v| ProviderAccessToken {
                    access_token: v.clone(),
                }))
        }

        async fn find_provider_token_full(
            &self,
            _user_id: &str,
            _provider: &str,
        ) -> Result<Option<ProviderToken>, sqlx::Error> {
            Ok(None)
        }

        async fn upsert_token(
            &self,
            user_id: &str,
            provider: &str,
            access_token: &str,
            _refresh_token: Option<&str>,
            _token_type: Option<&str>,
            _metadata: Option<&serde_json::Value>,
        ) -> Result<(), sqlx::Error> {
            self.rows.lock().await.insert(
                (user_id.to_string(), provider.to_string()),
                access_token.to_string(),
            );
            Ok(())
        }

        async fn upsert_token_with_auth_method(
            &self,
            _user_id: &str,
            _provider: &str,
            _auth_method: &str,
            _access_token: &str,
            _refresh_token: Option<&str>,
            _token_type: Option<&str>,
            _metadata: Option<&serde_json::Value>,
        ) -> Result<(), sqlx::Error> {
            Ok(())
        }

        async fn delete_token(&self, _user_id: &str, _provider: &str) -> Result<(), sqlx::Error> {
            Ok(())
        }

        async fn find_codex_blob(
            &self,
            _scope: &CodexScope,
        ) -> Result<Option<ProviderToken>, sqlx::Error> {
            Ok(None)
        }

        async fn upsert_codex_blob(
            &self,
            scope: &CodexScope,
            access_token: &str,
            _refresh_token: Option<&str>,
            _token_type: Option<&str>,
            _metadata: Option<&serde_json::Value>,
        ) -> Result<(), sqlx::Error> {
            self.rows
                .lock()
                .await
                .insert(Self::codex_key(scope), access_token.to_string());
            Ok(())
        }

        async fn delete_codex_blob(&self, _scope: &CodexScope) -> Result<(), sqlx::Error> {
            Ok(())
        }

        async fn find_github_row(
            &self,
            _scope: &GitHubScope,
        ) -> Result<Option<ProviderToken>, sqlx::Error> {
            Ok(None)
        }

        async fn upsert_github_row(
            &self,
            _scope: &GitHubScope,
            _auth_method: &str,
            _access_token: &str,
            _token_type: Option<&str>,
            _metadata: Option<&serde_json::Value>,
            _provider_user_id: Option<&str>,
            _provider_login: Option<&str>,
        ) -> Result<(), sqlx::Error> {
            Ok(())
        }

        async fn delete_github_row(&self, _scope: &GitHubScope) -> Result<(), sqlx::Error> {
            Ok(())
        }

        async fn find_github_scope_presence(
            &self,
            _user_id: &str,
        ) -> Result<Option<GithubTokenPresence>, sqlx::Error> {
            Ok(None)
        }
    }

    /// A `SandboxConnector` that records every `task_id` it is dialled for and
    /// then fails the connect. We only care that the fan-out reached the
    /// connector — not that a real gRPC push succeeded — so `NotReady` keeps
    /// the test free of any live sandbox plumbing.
    struct RecordingConnector {
        dialled: mpsc::UnboundedSender<String>,
    }

    #[async_trait]
    impl SandboxConnector for RecordingConnector {
        async fn sandbox_client(&self, task_id: &str) -> Result<SandboxClient, SandboxError> {
            let _ = self.dialled.send(task_id.to_string());
            Err(SandboxError::NotReady("test connector".to_string()))
        }
    }

    /// A user-scope Codex save with an open sandbox must fan the rotation out
    /// to that sandbox. Asserts the *behaviour* — the connector is dialled for
    /// the open task — so it stays true regardless of how the save orders its
    /// persist and broadcast steps.
    ///
    /// The fixture is write-honouring on purpose (see [`WriteHonouringTokens`]):
    /// the save's `upsert_codex_blob` lands in the map first, so if the
    /// user-scope path routed through `store` again — the regression this
    /// guards — `store`'s dedup would read that identical value back and
    /// suppress the broadcast, the connector would never be dialled, and this
    /// test would time out and fail. It has been run both ways to confirm it
    /// fails against the reverted call site, not merely passes against the fix.
    #[tokio::test]
    async fn user_scope_save_with_open_sandbox_fans_out() {
        let (tx, mut rx) = mpsc::unbounded_channel();

        let mut tasks = MockTaskDao::new();
        tasks
            .expect_list_open_codex_tasks_for_scope()
            .returning(|_, _, _| Ok(vec!["task-1".to_string()]));

        let provider_tokens: Arc<dyn ProviderTokenDao> =
            Arc::new(WriteHonouringTokens::default());
        let credentials = Arc::new(CredentialService::new(Arc::clone(&provider_tokens)));
        let credential_sync = Arc::new(CredentialSync::new(
            credentials,
            Arc::new(tasks),
            Arc::new(RecordingConnector { dialled: tx }),
        ));

        let body = SaveCodexAuthRequest {
            auth_json: r#"{"tokens":{"access_token":"a","refresh_token":"r"}}"#.to_string(),
        };

        save_codex_auth_for_scope(
            &provider_tokens,
            &credential_sync,
            CodexScope::user("u1"),
            body,
        )
        .await
        .expect("save should succeed");

        // The fan-out is spawned, so wait for the connector to be dialled.
        let dialled = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .expect("connector should be dialled within the timeout")
            .expect("connector sender should not be dropped");
        assert_eq!(dialled, "task-1");
    }
}
