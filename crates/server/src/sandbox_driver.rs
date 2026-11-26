//! Handing a Sandbox to the person whose message starts its turn.

use std::sync::Arc;

use tracing::{info, warn};

use moatless_core::sandbox_jwt as jwt;
use moatless_ports::identity::CommitIdentity;
use moatless_sandbox_control::lifecycle::{DriverChange, SandboxIngress, TokenGenerationStore};
use moatless_sandbox_control::notify::CredentialBroadcast;
use moatless_tasks::task_event::models::InsertTaskEvent;
use moatless_tasks::task_event::operations::TaskEvents;
use moatless_types::access_control::ScopeContext;
use moatless_types::credential_key::USER_AUTH_TOKEN;

/// Switches a Sandbox's driver, then pushes the new driver's task token to the
/// running pod.
pub struct SandboxDrivers {
    pub sandboxes: Arc<SandboxIngress>,
    pub generations: Arc<TokenGenerationStore>,
    pub users: Arc<dyn CommitIdentity>,
    pub broadcast: Arc<dyn CredentialBroadcast>,
    pub task_events: Arc<TaskEvents>,
    /// `None` only under an unvalidated config, which mints no tokens.
    pub task_token_secret: Option<String>,
    pub user_token_ttl_secs: u64,
}

impl SandboxDrivers {
    /// Over `ports`' Sandbox store, pushing through `connector`.
    pub fn compose(
        config: &moatless_core::config::Config,
        ports: &crate::compose::Ports,
        connector: Arc<dyn sandbox_proto::client::SandboxConnector>,
    ) -> Arc<Self> {
        Arc::new(Self {
            sandboxes: ports.sandbox_graph.sandbox_ingress(),
            generations: ports.sandbox_graph.token_generations(),
            users: ports.user_directory.clone(),
            broadcast: Arc::new(crate::sandbox::notify::CredentialBroadcastService::new(
                connector,
            )),
            task_events: ports.task_events.clone(),
            task_token_secret: config.sandbox.task_token_secret.clone(),
            user_token_ttl_secs: config.sandbox.user_token_ttl_secs,
        })
    }
}

/// Why [`SandboxDrivers::drive`] did not hand the Sandbox over.
#[derive(Debug, thiserror::Error)]
pub enum DriveError {
    /// `user_id` cannot write the task, so may not hold its credentials.
    #[error("{0}")]
    Forbidden(String),
    /// The Sandbox's row could not be read or switched.
    #[error("{0}")]
    Unavailable(String),
}

impl SandboxDrivers {
    /// Make `user_id` the driver of `task_id`'s Sandbox, revoking every token
    /// minted for the previous one, and return their new token when this call
    /// switched it, pushed or not. `None` when they already drive it, when the
    /// task has no Sandbox row yet (provisioning reads the driver), and where
    /// the deployment mints no tokens.
    pub async fn drive(&self, task_id: &str, user_id: &str) -> Result<Option<String>, DriveError> {
        let Some(intent) = self.sandboxes.intent(task_id).await.map_err(unavailable)? else {
            return Ok(None);
        };
        let admin = matches!(self.users.is_platform_admin(user_id).await, Ok(true));
        let ctx = ScopeContext::new(user_id.to_string(), admin);
        let resource = intent.scoped_resource();
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
            return Ok(None);
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
        Ok(self
            .hand_over(task_id, &intent.sandbox_id, user_id, change.generation)
            .await)
    }

    /// Mint the driver's token and push it. Nothing here fails the switch, which has already committed: the token is
    /// returned for the caller to hand over should the push miss.
    async fn hand_over(
        &self,
        task_id: &str,
        sandbox_id: &str,
        user_id: &str,
        generation: i64,
    ) -> Option<String> {
        let token = self.mint(sandbox_id, user_id, generation).await;
        if let Some(token) = &token {
            self.push(task_id, USER_AUTH_TOKEN, token, &[0, 200, 800])
                .await;
        }
        token
    }

    /// `None` where the deployment mints no tokens, or the mint failed.
    async fn mint(&self, sandbox_id: &str, user_id: &str, generation: i64) -> Option<String> {
        let secret = self.task_token_secret.as_deref()?;
        // The login only labels the token, so an unreadable one is no reason
        // to leave the pod with a refused token.
        let login = match self.users.git_identity(user_id).await {
            Ok(identity) => identity.map_or_default(|identity| identity.login),
            Err(e) => {
                warn!(user_id, error = %e, "Failed to read the new driver's login");
                String::new()
            }
        };
        jwt::generate_user_auth_token(
            user_id,
            &login,
            sandbox_id,
            generation,
            secret,
            self.user_token_ttl_secs,
        )
        .inspect_err(|e| warn!(sandbox_id, error = %e, "Failed to mint the new driver's token"))
        .ok()
    }

    /// Push `key` to the pod, sleeping each of `backoffs_ms` before an
    /// attempt: a pod mid-restart answers once it is back.
    async fn push(&self, task_id: &str, key: &str, value: &str, backoffs_ms: &[u64]) {
        for (attempt, &backoff_ms) in backoffs_ms.iter().enumerate() {
            tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
            match self.broadcast.refresh_credential(task_id, key, value).await {
                Ok(()) => return,
                Err(e) => {
                    warn!(task_id, key, attempt, error = %e, "Failed to push a credential to the new driver's pod");
                }
            }
        }
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

    use async_trait::async_trait;
    use moatless_ports::identity::MockCommitIdentity;
    use moatless_types::sandbox::SandboxError;
    use sqlx::PgPool;

    use super::*;

    const SECRET: &str = "driver-test-secret";

    /// Every push the drivers made, as `(task_id, key, value)`; each one
    /// refused when the flag is set, as by a pod that cannot be reached.
    #[derive(Default)]
    struct RecordedPushes(Mutex<Vec<(String, String, String)>>, bool);

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
            if self.1 {
                return Err(SandboxError::NotReady("unreachable".to_string()));
            }
            Ok(())
        }
    }

    /// `owner`'s task `t1`, its own Sandbox, at `visibility`.
    async fn seed(pool: &PgPool, visibility: &str) {
        for statement in [
            "INSERT INTO workspaces (id, name, scope, owner_user_id, created_by)
             VALUES ('ws-x', 'X', 'user', 'owner', 'owner')",
            "INSERT INTO sandboxes (sandbox_id, workspace_id, owner_user_id,
                 run_as_user_id, agent_profile)
             VALUES ('t1', 'ws-x', 'owner', 'owner', 'claude-code')",
        ] {
            sqlx::query(statement).execute(pool).await.unwrap();
        }
        sqlx::query(
            "INSERT INTO tasks (task_id, created_at, last_message_at, agent_profile,
                 repository_id, status, total_cost_usd, visibility,
                 created_by, owner_user_id, workspace_id, sandbox_id)
             VALUES ('t1', NOW()::text, NOW()::text, 'claude-code', 'repo-x', 'open', 0,
                 $1, 'owner', 'owner', 'ws-x', 't1')",
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

        let handed = drivers.drive("t1", "visitor").await.unwrap();

        let pushed = pushes.0.lock().unwrap().clone();
        let [(task_id, key, token)] = pushed.as_slice() else {
            panic!("one token push, got {pushed:?}");
        };
        assert_eq!((task_id.as_str(), key.as_str()), ("t1", USER_AUTH_TOKEN));
        let claims = jwt::verify_user_auth_token(token, SECRET).unwrap();
        assert_eq!(claims.user_id, "visitor");
        assert_eq!(claims.login, "visitor-login");
        assert_eq!(claims.generation, 1);
        assert_eq!(handed.as_ref(), Some(token));

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

        assert_eq!(drivers.drive("t1", "owner").await.unwrap(), None);
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

    /// The switch stands when the pod cannot be reached; the token comes back
    /// for the caller to hand over rather than being lost.
    #[sqlx::test]
    async fn an_undelivered_token_is_returned_for_the_caller(pool: PgPool) {
        seed(&pool, "public").await;
        let pushes = Arc::new(RecordedPushes(Mutex::default(), true));
        let drivers = drivers(&pool, pushes.clone());

        let token = drivers
            .drive("t1", "visitor")
            .await
            .unwrap()
            .expect("the token is minted regardless");
        let claims = jwt::verify_user_auth_token(&token, SECRET).unwrap();
        assert_eq!((claims.user_id.as_str(), claims.generation), ("visitor", 1));
        assert_eq!(pushes.0.lock().unwrap().len(), 3, "the push is retried");
    }
}
