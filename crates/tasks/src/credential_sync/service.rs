use std::sync::Arc;

use tracing::{debug, info, warn};

use crate::task::dao::TaskDao;
use moatless_identity::credentials::codex::refresh::CodexRefreshOutcome;
use moatless_identity::credentials::{
    CredentialError, CredentialService, StoreOutcome, WriteSource, should_broadcast,
};
use moatless_ports::sandbox::CredentialBroadcast;
use moatless_types::agent::CodexScope;

/// Tasks-band orchestrator: wraps the pure [`CredentialService`] with the
/// in-flight guard and the sandbox fan-out.
pub struct CredentialSync {
    credentials: Arc<CredentialService>,
    tasks: Arc<dyn TaskDao>,
    broadcast: Arc<dyn CredentialBroadcast>,
}

impl CredentialSync {
    /// Build the orchestrator over a pure credential service.
    pub(crate) fn new(
        credentials: Arc<CredentialService>,
        tasks: Arc<dyn TaskDao>,
        broadcast: Arc<dyn CredentialBroadcast>,
    ) -> Self {
        Self {
            credentials,
            tasks,
            broadcast,
        }
    }

    /// Just-in-time Codex auth.json refresh, addressed by `scope`.
    ///
    /// 1. If another *open* codex task within this scope (other than
    ///    `except_task_id`) is in flight, return the stored blob unchanged —
    ///    the sandbox watcher will rotate it. Avoids racing the CLI for the
    ///    single-use `refresh_token`.
    /// 2. Otherwise refresh via [`CredentialService::refresh_codex_locked`] and,
    ///    on [`CodexRefreshOutcome::Rotated`], fan the new blob out.
    pub async fn ensure_fresh_codex(
        &self,
        scope: CodexScope,
        except_task_id: Option<&str>,
    ) -> Result<String, CredentialError> {
        // Lock ordering: the in-flight guard — a cheap best-effort `TaskDao`
        // read that never needs the lock — runs before the per-scope refresh
        // mutex in `refresh_codex_locked`, so it is consulted first. When a
        // sandbox is the active refresher we return its stored blob without
        // ever taking the refresh mutex.
        if self.any_other_open_codex_task(&scope, except_task_id).await {
            debug!(
                scope = %scope.display(),
                "ensure_fresh_codex: skipping backend refresh (open codex task in flight)"
            );
            return self.credentials.read_raw_codex(&scope).await;
        }

        let outcome = self.credentials.refresh_codex_locked(&scope).await?;
        if let CodexRefreshOutcome::Rotated { ref new_blob } = outcome {
            // Broadcast outside the SQL transaction — fan-out spawns gRPC
            // tasks per running sandbox.
            self.broadcast_to_scope(&scope, None, "CODEX_AUTH_JSON", new_blob)
                .await;
        }
        Ok(outcome.into_blob())
    }

    /// Resolve `CODEX_AUTH_JSON` with `user > global` precedence in one call.
    ///
    /// Single entry point for callers that don't want to think about scopes:
    /// the resolver in `sandbox::secrets`, the gRPC `GetCredentials` handler,
    /// and the background refresher all see one method. The walk lives here so
    /// the "most-specific wins" rule is enforced centrally — callers can't
    /// accidentally invert the precedence by reordering their calls.
    ///
    /// Each scope's blob is refreshed in place via [`Self::ensure_fresh_codex`].
    /// The first scope that yields a usable blob wins; scopes with no row or a
    /// permanent refresh failure (`CredentialError::NotFound`) silently fall
    /// through. Transient errors are logged and the walk continues so a single
    /// scope's upstream outage can't block the others.
    pub async fn ensure_fresh_codex_for(
        &self,
        user_id: &str,
        except_task_id: Option<&str>,
    ) -> Result<String, CredentialError> {
        let scopes = [CodexScope::user(user_id), CodexScope::Global];

        let mut last_err: Option<CredentialError> = None;
        for scope in &scopes {
            match self.ensure_fresh_codex(scope.clone(), except_task_id).await {
                Ok(blob) => return Ok(blob),
                Err(CredentialError::NotFound(_)) => continue,
                Err(e) => {
                    warn!(
                        error = %e,
                        scope = %scope.display(),
                        "ensure_fresh_codex_for: transient failure, trying next scope"
                    );
                    last_err = Some(e);
                }
            }
        }
        Err(last_err.unwrap_or_else(|| {
            CredentialError::NotFound(format!(
                "No codex provider_token found at any scope (user={user_id})"
            ))
        }))
    }

    /// Persist a credential and, when a new value actually landed and the key
    /// is one sandboxes can't pull on demand, fan it out to running sandboxes.
    ///
    /// Takes `source` (not just a bool) because the broadcast-exclusion is
    /// derived from it: a `SandboxRotation { task_id }` excludes the
    /// originating sandbox so a rotated value doesn't echo back as a fresh
    /// push; every other source excludes nothing.
    ///
    /// The dedup-before-broadcast order is preserved: [`CredentialService::store`]
    /// short-circuits an unchanged value to [`StoreOutcome::Unchanged`] and a
    /// GitHub token to [`StoreOutcome::SecretRef`], both *before* returning, so
    /// only [`StoreOutcome::Persisted`] can reach the fan-out below.
    pub async fn store(
        &self,
        user_id: &str,
        key: &str,
        value: &str,
        source: WriteSource,
    ) -> Result<(), CredentialError> {
        // Compute the exclusion before `source` is moved into `store`.
        let except_task_id = match &source {
            WriteSource::SandboxRotation { task_id } => Some(task_id.clone()),
            _ => None,
        };

        let outcome = self.credentials.store(user_id, key, value, source).await?;

        // `SecretRef` (GitHub → encrypted store; `should_broadcast("GITHUB_TOKEN")`
        // is false anyway) and `Unchanged` (dedup hit) never fan out; only a
        // fresh `Persisted` value with a file-resident key does.
        if matches!(outcome, StoreOutcome::Persisted) && should_broadcast(key) {
            // `store` is user-scoped (per-user provider_tokens upsert), so fan
            // out to that user's running sandboxes. Team / global rotations go
            // through `broadcast_codex_rotation` instead.
            self.broadcast_to_scope(
                &CodexScope::user(user_id),
                except_task_id.as_deref(),
                key,
                value,
            )
            .await;
        }

        Ok(())
    }

    /// Public-facing entry point for callers that have already persisted a
    /// codex blob via the DAO and want to fan the rotation out to live
    /// sandboxes — team / global saves from HTTP handlers, and user-scope
    /// saves that wrote the row themselves. Those user-scope callers must come
    /// here rather than [`Self::store`]: `store`'s dedup reads the just-written
    /// value back, sees no change, and would suppress the broadcast. Call
    /// `store` instead only when the write has not happened yet and you want
    /// its dedup + persistence path to run.
    pub async fn broadcast_codex_rotation(&self, scope: &CodexScope, value: &str) {
        self.broadcast_to_scope(scope, None, "CODEX_AUTH_JSON", value)
            .await;
    }

    /// Returns true when any *other* codex-typed task within this scope is
    /// currently `Open`. Best-effort: any DB error is logged and treated as
    /// "no other tasks" so a transient failure can't block every refresh.
    async fn any_other_open_codex_task(
        &self,
        scope: &CodexScope,
        except_task_id: Option<&str>,
    ) -> bool {
        let scope_kind = scope.scope_kind().to_string();
        let scope_id = match scope {
            CodexScope::User(id) => id.clone(),
            CodexScope::Global => String::new(),
        };
        let except = except_task_id.unwrap_or("").to_string();
        match self
            .tasks
            .list_open_codex_tasks_for_scope(scope_kind, scope_id, except)
            .await
        {
            Ok(tasks) => !tasks.is_empty(),
            Err(e) => {
                warn!(
                    error = %e,
                    scope = %scope.display(),
                    "ensure_fresh_codex: failed to list codex tasks, skipping in-flight guard"
                );
                false
            }
        }
    }

    /// Push a freshly stored credential to every running sandbox whose
    /// effective user resolves through `scope`. Fire-and-forget per sandbox
    /// — failures are logged.
    ///
    /// v1 fan-out is coarse: for team / global scopes we push to every open
    /// codex task within the scope, including users who have their own row
    /// and don't need the push. Harmless because the sandbox watcher applies
    /// the value only when it differs from the in-memory blob.
    async fn broadcast_to_scope(
        &self,
        scope: &CodexScope,
        except_task_id: Option<&str>,
        key: &str,
        value: &str,
    ) {
        let scope_kind = scope.scope_kind().to_string();
        let scope_id = match scope {
            CodexScope::User(id) => id.clone(),
            CodexScope::Global => String::new(),
        };
        let except = except_task_id.unwrap_or("").to_string();
        let task_ids = match self
            .tasks
            .list_open_codex_tasks_for_scope(scope_kind, scope_id, except)
            .await
        {
            Ok(ids) => ids,
            Err(e) => {
                warn!(
                    error = %e,
                    scope = %scope.display(),
                    "broadcast_to_scope: failed to list codex tasks"
                );
                return;
            }
        };

        for task_id in task_ids {
            let broadcast = Arc::clone(&self.broadcast);
            let key = key.to_string();
            let value = value.to_string();
            tokio::spawn(async move {
                if let Err(e) = broadcast.refresh_credential(&task_id, &key, &value).await {
                    debug!(task_id, error = %e, "broadcast: refresh_credential failed");
                } else {
                    info!(task_id, key, "broadcast: refresh_credential delivered");
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::dao::MockTaskDao;
    use async_trait::async_trait;
    use moatless_identity::credentials::test_support::{InMemoryTokens, credentials_over};
    use moatless_ports::sandbox::{CredentialBroadcast, SandboxError};

    /// A `CredentialBroadcast` that must never be reached. Every test below
    /// either keeps `list_open_codex_tasks_for_scope` returning empty (nothing
    /// to fan out to) or never enters the broadcast path, so a delivery is a
    /// bug — fail loudly rather than making a network call.
    struct NeverBroadcast;

    #[async_trait]
    impl CredentialBroadcast for NeverBroadcast {
        async fn refresh_credential(
            &self,
            task_id: &str,
            _key: &str,
            _value: &str,
        ) -> Result<(), SandboxError> {
            panic!("NeverBroadcast::refresh_credential called for {task_id}");
        }
    }

    fn make_sync(repo: Arc<InMemoryTokens>, tasks: MockTaskDao) -> CredentialSync {
        CredentialSync::new(
            credentials_over(repo),
            Arc::new(tasks),
            Arc::new(NeverBroadcast),
        )
    }

    // == Test A: the guard predicate ==

    #[tokio::test]
    async fn guard_true_when_an_open_task_exists() {
        let mut tasks = MockTaskDao::new();
        tasks
            .expect_list_open_codex_tasks_for_scope()
            .returning(|_, _, _| Ok(vec!["task-1".to_string()]));
        let sync = make_sync(InMemoryTokens::empty(), tasks);

        assert!(
            sync.any_other_open_codex_task(&CodexScope::user("u1"), None)
                .await
        );
    }

    #[tokio::test]
    async fn guard_false_and_threads_the_except_task_id() {
        let mut tasks = MockTaskDao::new();
        // The excluded task is the only open one, so the DAO (which filters on
        // `except`) sees an empty list. Assert the except arg is threaded verbatim.
        tasks
            .expect_list_open_codex_tasks_for_scope()
            .withf(|kind, id, except| kind == "user" && id == "u1" && except == "task-1")
            .returning(|_, _, _| Ok(vec![]));
        let sync = make_sync(InMemoryTokens::empty(), tasks);

        assert!(
            !sync
                .any_other_open_codex_task(&CodexScope::user("u1"), Some("task-1"))
                .await
        );
    }

    // == Test B': the guard is consulted exactly once on the refresh path ==

    #[tokio::test]
    async fn ensure_fresh_codex_consults_the_guard_exactly_once() {
        let mut tasks = MockTaskDao::new();
        // `.times(1)` is verified when the mock drops with `sync`: if the guard
        // stops firing, this expectation fails. No open task → guard passes →
        // refresh path. The service has no db/http, so `refresh_codex_locked`
        // falls back to `read_raw_codex` (Unchanged, no Rotated) → no broadcast,
        // so the connector is never reached and this is the only DAO call.
        tasks
            .expect_list_open_codex_tasks_for_scope()
            .times(1)
            .returning(|_, _, _| Ok(vec![]));
        let sync = make_sync(InMemoryTokens::with_codex_blob("seeded-blob"), tasks);

        let blob = sync
            .ensure_fresh_codex(CodexScope::user("u1"), None)
            .await
            .unwrap();
        assert_eq!(blob, "seeded-blob");
    }

    // == Fan-out: store persisted broadcasts with the source-derived except;
    //    unchanged does not ==

    #[tokio::test]
    async fn store_persisted_fans_out_with_the_source_except() {
        let mut tasks = MockTaskDao::new();
        // Persisted + file-resident key → broadcast. Assert the resolution query
        // fires once with the originating task excluded. Empty result → the
        // NeverBroadcast is never dialled.
        tasks
            .expect_list_open_codex_tasks_for_scope()
            .withf(|kind, id, except| kind == "user" && id == "u1" && except == "origin-task")
            .times(1)
            .returning(|_, _, _| Ok(vec![]));
        // Empty repo → find_provider_token None → a genuine upsert → Persisted.
        let sync = make_sync(InMemoryTokens::empty(), tasks);

        sync.store(
            "u1",
            "CODEX_AUTH_JSON",
            "new-blob",
            WriteSource::SandboxRotation {
                task_id: "origin-task".to_string(),
            },
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn store_unchanged_does_not_fan_out() {
        let mut tasks = MockTaskDao::new();
        // Dedup hit → Unchanged → the resolution query must not fire at all.
        tasks.expect_list_open_codex_tasks_for_scope().times(0);
        // Seed the same value the caller stores → dedup short-circuit.
        let sync = make_sync(InMemoryTokens::with_token("u1", "codex", "same"), tasks);

        sync.store("u1", "CODEX_AUTH_JSON", "same", WriteSource::UserUiSave)
            .await
            .unwrap();
    }
}
