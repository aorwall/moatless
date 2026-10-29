//! The generation every backend route checks a Sandbox's task token against,
//! and the user its tokens are minted for.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use moka::future::Cache;
use moka::ops::compute::Op;

use moatless_core::sandbox_jwt::TokenGenerations;
use moatless_types::store::StoreError;

use super::dao::LifecycleDao;

/// A Sandbox's driver after a switch, and the generation its tokens are now
/// minted at.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct DriverChange {
    pub generation: i64,
    /// The person the Sandbox acted for before: its driver, else its owner.
    pub previous_user_id: Option<String>,
}

/// How long a read generation is reused. This bounds revocation latency: a
/// bump on another replica is seen here, and a token from before it refused,
/// at most this long after the bump.
const GENERATION_CACHE_TTL: Duration = Duration::from_secs(5);

/// Max Sandboxes whose generation is cached at once.
const MAX_CACHED_SANDBOXES: u64 = 10_000;

/// A Sandbox's generation and the user its tokens at that generation are
/// minted for. Cached as a pair: every driver switch bumps the generation, so
/// the newer generation always carries the newer user.
#[derive(Debug, Clone, PartialEq, Eq)]
struct TokenState {
    generation: i64,
    user_id: Option<String>,
}

/// Cached reads of `sandboxes.token_generation` and the driver, and the bump
/// that revokes every task token minted before it.
pub struct TokenGenerationStore {
    store: Arc<dyn LifecycleDao>,
    /// `None` caches a Sandbox with no row.
    cache: Cache<String, Option<TokenState>>,
}

impl TokenGenerationStore {
    pub(crate) fn new(store: Arc<dyn LifecycleDao>) -> Self {
        Self {
            store,
            cache: Cache::builder()
                .time_to_live(GENERATION_CACHE_TTL)
                .max_capacity(MAX_CACHED_SANDBOXES)
                .build(),
        }
    }

    /// Advance `sandbox_id`'s generation and return the new value. Mint the
    /// replacement token at that value: every token minted before is refused.
    pub async fn bump(&self, sandbox_id: &str) -> Result<i64, StoreError> {
        let (generation, user_id) = self
            .store
            .bump_token_generation(sandbox_id)
            .await?
            .ok_or_else(|| StoreError::new(format!("sandbox {sandbox_id} has no row to bump")))?;
        self.remember(
            sandbox_id,
            Some(TokenState {
                generation,
                user_id,
            }),
        )
        .await;
        Ok(generation)
    }

    /// Make `user_id` the Sandbox's driver, revoking every token minted for
    /// the previous one. `None` when it already acts for `user_id`.
    ///
    /// Mint the driver's token at the returned generation.
    pub async fn switch_driver(
        &self,
        sandbox_id: &str,
        user_id: &str,
    ) -> Result<Option<DriverChange>, StoreError> {
        let change = self.store.switch_driver(sandbox_id, user_id).await?;
        if let Some(change) = &change {
            let state = TokenState {
                generation: change.generation,
                user_id: Some(user_id.to_string()),
            };
            self.remember(sandbox_id, Some(state)).await;
        }
        Ok(change)
    }

    async fn state(&self, sandbox_id: &str) -> Result<Option<TokenState>, StoreError> {
        if let Some(state) = self.cache.get(sandbox_id).await {
            return Ok(state);
        }
        self.read(sandbox_id).await
    }

    async fn read(&self, sandbox_id: &str) -> Result<Option<TokenState>, StoreError> {
        let read = self
            .store
            .sandbox_shape(sandbox_id)
            .await?
            .map(|shape| TokenState {
                generation: shape.token_generation,
                user_id: shape.acting_user_id().map(str::to_string),
            });
        self.remember(sandbox_id, read.clone()).await;
        Ok(read)
    }

    /// Cache `read` unless a newer value is cached: a read that started before
    /// a bump can land after it, and must not put the old generation back.
    async fn remember(&self, sandbox_id: &str, read: Option<TokenState>) {
        self.cache
            .entry(sandbox_id.to_string())
            .and_compute_with(|cached| {
                let newest = match cached.map(moka::Entry::into_value) {
                    Some(Some(cached))
                        if read
                            .as_ref()
                            .is_none_or(|read| read.generation < cached.generation) =>
                    {
                        Some(cached)
                    }
                    _ => read,
                };
                std::future::ready(Op::Put(newest))
            })
            .await;
    }
}

#[async_trait]
impl TokenGenerations for TokenGenerationStore {
    async fn current(&self, sandbox_id: &str) -> Result<Option<i64>, StoreError> {
        Ok(self.state(sandbox_id).await?.map(|state| state.generation))
    }

    async fn token_user(&self, sandbox_id: &str) -> Result<Option<String>, StoreError> {
        Ok(self
            .state(sandbox_id)
            .await?
            .and_then(|state| state.user_id))
    }

    async fn reread(&self, sandbox_id: &str) -> Result<Option<i64>, StoreError> {
        Ok(self.read(sandbox_id).await?.map(|state| state.generation))
    }
}

#[cfg(all(test, feature = "db-tests"))]
mod db_tests {
    use super::*;
    use crate::lifecycle::dao::PgLifecycleDao;
    use crate::lifecycle::test_support::seed_task;
    use moatless_core::sandbox_jwt::check_generation;

    #[sqlx::test]
    async fn bump_advances_the_generation_a_read_returns(pool: sqlx::PgPool) {
        seed_task(&pool, "t1").await;
        let generations = TokenGenerationStore::new(Arc::new(PgLifecycleDao::new(pool)));

        assert_eq!(generations.current("t1").await.unwrap(), Some(0));
        assert_eq!(generations.bump("t1").await.unwrap(), 1);
        assert_eq!(generations.current("t1").await.unwrap(), Some(1));
        assert_eq!(generations.current("no-such-sandbox").await.unwrap(), None);
        assert!(generations.bump("no-such-sandbox").await.is_err());
    }

    #[sqlx::test]
    async fn a_token_minted_on_another_replica_is_accepted_at_once(pool: sqlx::PgPool) {
        seed_task(&pool, "t1").await;
        let dao = Arc::new(PgLifecycleDao::new(pool));
        let switching = TokenGenerationStore::new(dao.clone());
        let checking = TokenGenerationStore::new(dao);
        assert_eq!(checking.current("t1").await.unwrap(), Some(0));

        switching.switch_driver("t1", "user-2").await.unwrap();

        check_generation(&checking, "t1", 1).await.unwrap();
        assert_eq!(
            checking.token_user("t1").await.unwrap().as_deref(),
            Some("user-2")
        );
        assert!(check_generation(&checking, "t1", 0).await.is_err());
    }

    #[sqlx::test]
    async fn a_new_driver_revokes_the_previous_ones_tokens(pool: sqlx::PgPool) {
        seed_task(&pool, "t1").await;
        let dao = Arc::new(PgLifecycleDao::new(pool));
        let generations = TokenGenerationStore::new(dao.clone());

        assert_eq!(
            generations.token_user("t1").await.unwrap().as_deref(),
            Some("user-1")
        );
        assert_eq!(
            generations.switch_driver("t1", "user-1").await.unwrap(),
            None
        );

        let change = generations.switch_driver("t1", "user-2").await.unwrap();
        assert_eq!(
            change,
            Some(DriverChange {
                generation: 1,
                previous_user_id: Some("user-1".to_string()),
            })
        );
        assert_eq!(generations.current("t1").await.unwrap(), Some(1));
        assert_eq!(
            generations.token_user("t1").await.unwrap().as_deref(),
            Some("user-2")
        );
        assert_eq!(
            generations.switch_driver("t1", "user-2").await.unwrap(),
            None
        );

        // Uncached, so the row itself carries the pair.
        let fresh = TokenGenerationStore::new(dao);
        assert_eq!(
            fresh.token_user("t1").await.unwrap().as_deref(),
            Some("user-2")
        );
        assert_eq!(
            fresh
                .switch_driver("t1", "user-1")
                .await
                .unwrap()
                .map(|c| c.generation),
            Some(2)
        );
        assert_eq!(
            fresh
                .switch_driver("no-such-sandbox", "user-1")
                .await
                .unwrap(),
            None
        );
    }
}
