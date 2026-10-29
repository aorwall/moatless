//! Handing a Sandbox to the person whose message or Script now drives it.

use std::sync::Arc;

use async_trait::async_trait;
use tracing::{info, warn};

use moatless_core::sandbox_jwt as jwt;
use moatless_ports::identity::CommitIdentity;
use moatless_ports::sandbox::{CodexCredentials, CredentialBroadcast, DriveError, SandboxDriver};
use moatless_sandbox_control::lifecycle::{DriverChange, SandboxIngress, TokenGenerationStore};
use moatless_tasks::task_event::models::InsertTaskEvent;
use moatless_tasks::task_event::operations::TaskEvents;
use moatless_types::access_control::{ScopeContext, ScopedResource};
use moatless_types::credential_key::USER_AUTH_TOKEN;

/// The [`SandboxDriver`] that switches the row, then pushes the new driver's
/// task token to the running pod.
pub struct SandboxDrivers {
    pub sandboxes: Arc<SandboxIngress>,
    pub generations: Arc<TokenGenerationStore>,
    pub users: Arc<dyn CommitIdentity>,
    pub broadcast: Arc<dyn CredentialBroadcast>,
    pub codex: Arc<dyn CodexCredentials>,
    pub task_events: Arc<TaskEvents>,
    /// `None` in the dev bypass, which mints no tokens.
    pub task_token_secret: Option<String>,
    pub user_token_ttl_secs: u64,
}

impl SandboxDrivers {
    /// Over `ports`' Sandbox store, pushing through `connector`.
    pub fn compose(
        config: &moatless_core::config::Config,
        ports: &crate::compose::Ports,
        connector: Arc<dyn sandbox_proto::client::SandboxConnector>,
        codex: Arc<dyn CodexCredentials>,
    ) -> Arc<Self> {
        Arc::new(Self {
            sandboxes: ports.sandbox_graph.sandbox_ingress(),
            generations: ports.sandbox_graph.token_generations(),
            users: ports.user_directory.clone(),
            broadcast: Arc::new(crate::sandbox::notify::CredentialBroadcastService::new(
                connector,
            )),
            codex,
            task_events: ports.task_events.clone(),
            task_token_secret: config.sandbox.task_token_secret.clone(),
            user_token_ttl_secs: config.sandbox.user_token_ttl_secs,
        })
    }
}

#[async_trait]
impl SandboxDriver for SandboxDrivers {
    async fn drive(&self, task_id: &str, user_id: &str) -> Result<(), DriveError> {
        let Some(intent) = self.sandboxes.intent(task_id).await.map_err(unavailable)? else {
            return Ok(());
        };
        let admin = matches!(self.users.is_platform_admin(user_id).await, Ok(true));
        let ctx = ScopeContext::new(user_id.to_string(), admin);
        let resource = ScopedResource::borrowed(intent.scope, intent.owner_user_id.as_deref());
        if !ctx.can_write(&resource, Some(intent.visibility)) {
            return Err(DriveError::Forbidden(format!(
                "{user_id} cannot write task {task_id}, so cannot drive its sandbox"
            )));
        }
        let Some(change) = self
            .generations
            .switch_driver(&intent.sandbox_id, user_id)
            .await
            .map_err(unavailable)?
        else {
            return Ok(());
        };
        info!(
            task_id,
            sandbox_id = %intent.sandbox_id,
            driver = user_id,
            previous = ?change.previous_user_id,
            generation = change.generation,
            "Sandbox driver changed"
        );
        self.record(task_id, user_id, &change).await;
        self.push(task_id, &intent.sandbox_id, user_id, change.generation)
            .await
    }
}

impl SandboxDrivers {
    /// Push the driver's token, then their Codex blob. A failed token push is
    /// returned: the pod's own token is already refused, so only a
    /// re-provision, which mints for the driver, brings it back.
    async fn push(
        &self,
        task_id: &str,
        sandbox_id: &str,
        user_id: &str,
        generation: i64,
    ) -> Result<(), DriveError> {
        if let Some(secret) = self.task_token_secret.as_deref() {
            let login = match self.users.git_identity(user_id).await {
                Ok(Some(identity)) => identity.login,
                Ok(None) => String::new(),
                Err(e) => return Err(unavailable(e)),
            };
            let token = jwt::generate_user_auth_token(
                user_id,
                &login,
                sandbox_id,
                generation,
                secret,
                self.user_token_ttl_secs,
            )
            .map_err(unavailable)?;
            self.broadcast
                .refresh_credential(task_id, USER_AUTH_TOKEN, &token)
                .await
                .map_err(unavailable)?;
        }
        // Best-effort: a driver with no Codex credential keeps the previous
        // blob until their own exists.
        match self.codex.ensure_fresh_codex_for(user_id, None).await {
            Ok(Some(blob)) => {
                if let Err(e) = self
                    .broadcast
                    .refresh_credential(task_id, moatless_core::secret_keys::CODEX_AUTH_JSON, &blob)
                    .await
                {
                    warn!(task_id, error = %e, "Failed to push the new driver's Codex credential");
                }
            }
            Ok(None) => {}
            Err(e) => {
                warn!(task_id, error = %e, "Failed to read the new driver's Codex credential");
            }
        }
        Ok(())
    }

    async fn record(&self, task_id: &str, user_id: &str, change: &DriverChange) {
        let event = InsertTaskEvent {
            event_id: uuid::Uuid::new_v4().to_string(),
            task_id: task_id.to_string(),
            event_type: "sandbox".to_string(),
            sub_type: "driver_changed".to_string(),
            is_error: false,
            agent_profile: None,
            turn_number: None,
            payload: serde_json::json!({
                "driverUserId": user_id,
                "previousDriverUserId": change.previous_user_id,
                "tokenGeneration": change.generation,
            }),
            timestamp: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        };
        if let Err(e) = self.task_events.insert_event(&event).await {
            warn!(task_id, error = %e, "Failed to record the sandbox driver change");
        }
    }
}

fn unavailable(error: impl std::fmt::Display) -> DriveError {
    DriveError::Unavailable(error.to_string())
}

#[cfg(all(test, feature = "db-tests"))]
mod db_tests {
    use std::sync::Mutex;

    use moatless_ports::identity::MockCommitIdentity;
    use moatless_types::sandbox::SandboxError;
    use sqlx::PgPool;

    use super::*;

    const SECRET: &str = "driver-test-secret";

    /// Every push the drivers made, as `(task_id, key, value)`.
    #[derive(Default)]
    struct RecordedPushes(Mutex<Vec<(String, String, String)>>);

    #[async_trait]
    impl CredentialBroadcast for RecordedPushes {
        async fn refresh_credential(
            &self,
            task_id: &str,
            key: &str,
            value: &str,
        ) -> Result<(), SandboxError> {
            self.0
                .lock()
                .unwrap()
                .push((task_id.to_string(), key.to_string(), value.to_string()));
            Ok(())
        }
    }

    struct NoCodex;

    #[async_trait]
    impl CodexCredentials for NoCodex {
        async fn ensure_fresh_codex_for(
            &self,
            _user_id: &str,
            _except_task_id: Option<&str>,
        ) -> Result<Option<String>, String> {
            Ok(None)
        }
    }

    /// `owner`'s task `t1`, its own Sandbox, at `visibility`.
    async fn seed(pool: &PgPool, visibility: &str) {
        for statement in [
            "INSERT INTO teams (id, name, slug) VALUES ('team-x', 'Team X', 'team-x')",
            "INSERT INTO workspaces (id, name, scope, team_id, owner_user_id, created_by)
             VALUES ('ws-x', 'X', 'user', 'team-x', 'owner', 'owner')",
            "INSERT INTO sandboxes (sandbox_id, workspace_id, scope, owner_user_id,
                 run_as_user_id, agent_profile)
             VALUES ('t1', 'ws-x', 'user', 'owner', 'owner', 'claude-code')",
        ] {
            sqlx::query(statement).execute(pool).await.unwrap();
        }
        sqlx::query(
            "INSERT INTO tasks (task_id, created_at, last_message_at, agent_profile,
                 repository_id, status, total_cost_usd, visibility, scope, team_id,
                 created_by, owner_user_id, workspace_id, sandbox_id)
             VALUES ('t1', NOW()::text, NOW()::text, 'claude-code', 'repo-x', 'open', 0,
                 $1, 'user', 'team-x', 'owner', 'owner', 'ws-x', 't1')",
        )
        .bind(visibility)
        .execute(pool)
        .await
        .unwrap();
    }

    fn drivers(pool: &PgPool, pushes: Arc<RecordedPushes>) -> SandboxDrivers {
        let graph = moatless_sandbox_control::compose::sandbox_graph(pool.clone());
        let mut users = MockCommitIdentity::new();
        users.expect_is_platform_admin().returning(|_| Ok(false));
        users.expect_git_identity().returning(|user_id| {
            Ok(Some(moatless_ports::identity::GitIdentity {
                is_bot: false,
                login: format!("{user_id}-login"),
                name: None,
                email: None,
            }))
        });
        SandboxDrivers {
            sandboxes: graph.sandbox_ingress(),
            generations: graph.token_generations(),
            users: Arc::new(users),
            broadcast: pushes,
            codex: Arc::new(NoCodex),
            task_events: moatless_tasks::compose::task_graph(pool.clone()).task_events(),
            task_token_secret: Some(SECRET.to_string()),
            user_token_ttl_secs: 600,
        }
    }

    /// The done-when of drivers: a visitor's turn on the owner's public task
    /// hands the pod a token for the visitor, and the owner's is refused.
    #[sqlx::test]
    async fn a_visitor_driving_a_public_task_revokes_the_owners_token(pool: PgPool) {
        seed(&pool, "public").await;
        let pushes = Arc::new(RecordedPushes::default());
        let drivers = drivers(&pool, pushes.clone());

        drivers.drive("t1", "visitor").await.unwrap();

        let pushed = pushes.0.lock().unwrap().clone();
        let [(task_id, key, token)] = pushed.as_slice() else {
            panic!("one token push, got {pushed:?}");
        };
        assert_eq!((task_id.as_str(), key.as_str()), ("t1", USER_AUTH_TOKEN));
        let claims = jwt::verify_user_auth_token(token, SECRET).unwrap();
        assert_eq!(claims.user_id, "visitor");
        assert_eq!(claims.login, "visitor-login");
        assert_eq!(claims.generation, 1);

        let generations = drivers.generations.as_ref();
        assert!(jwt::check_generation(generations, "t1", 1).await.is_ok());
        assert!(matches!(
            jwt::check_generation(generations, "t1", 0).await,
            Err(jwt::GenerationError::Stale { .. })
        ));
        assert_eq!(
            jwt::TokenGenerations::token_user(generations, "t1")
                .await
                .unwrap()
                .as_deref(),
            Some("visitor")
        );

        let events = drivers
            .task_events
            .list_recent_events("t1", &None, 10, 0)
            .await
            .unwrap();
        let event = events
            .iter()
            .find(|event| event.sub_type == "driver_changed")
            .expect("the change is recorded");
        assert_eq!(event.payload["driverUserId"], "visitor");
        assert_eq!(event.payload["previousDriverUserId"], "owner");
    }

    #[sqlx::test]
    async fn the_current_driver_driving_again_pushes_nothing(pool: PgPool) {
        seed(&pool, "public").await;
        let pushes = Arc::new(RecordedPushes::default());
        let drivers = drivers(&pool, pushes.clone());

        drivers.drive("t1", "owner").await.unwrap();

        assert!(pushes.0.lock().unwrap().is_empty());
    }

    #[sqlx::test]
    async fn a_reader_without_write_cannot_drive(pool: PgPool) {
        seed(&pool, "private").await;
        let pushes = Arc::new(RecordedPushes::default());
        let drivers = drivers(&pool, pushes.clone());

        let refused = drivers.drive("t1", "stranger").await;

        assert!(matches!(refused, Err(DriveError::Forbidden(_))));
        assert!(pushes.0.lock().unwrap().is_empty());
    }
}
