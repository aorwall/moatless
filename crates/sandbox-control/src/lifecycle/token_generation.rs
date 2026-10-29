//! The generation every backend route checks a Sandbox's task token against.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use moka::future::Cache;
use moka::ops::compute::Op;

use moatless_core::sandbox_jwt::TokenGenerations;
use moatless_types::store::StoreError;

use super::dao::LifecycleDao;

/// How long a read generation is reused. This bounds revocation latency: a
/// bump on another replica is seen here, and a token from before it refused,
/// at most this long after the bump.
const GENERATION_CACHE_TTL: Duration = Duration::from_secs(5);

/// Max Sandboxes whose generation is cached at once.
const MAX_CACHED_SANDBOXES: u64 = 10_000;

/// Cached reads of `sandboxes.token_generation`, and the bump that revokes
/// every task token minted before it.
pub struct TokenGenerationStore {
    store: Arc<dyn LifecycleDao>,
    /// `None` caches a Sandbox with no row.
    cache: Cache<String, Option<i64>>,
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
        let generation = self
            .store
            .bump_token_generation(sandbox_id)
            .await?
            .ok_or_else(|| StoreError::new(format!("sandbox {sandbox_id} has no row to bump")))?;
        self.remember(sandbox_id, Some(generation)).await;
        Ok(generation)
    }

    /// Cache `read` unless a newer value is cached: a read that started before
    /// a bump can land after it, and must not put the old generation back.
    async fn remember(&self, sandbox_id: &str, read: Option<i64>) {
        self.cache
            .entry(sandbox_id.to_string())
            .and_compute_with(|cached| {
                let newest = cached.map_or(read, |entry| (*entry.value()).max(read));
                std::future::ready(Op::Put(newest))
            })
            .await;
    }
}

#[async_trait]
impl TokenGenerations for TokenGenerationStore {
    async fn current(&self, sandbox_id: &str) -> Result<Option<i64>, StoreError> {
        if let Some(generation) = self.cache.get(sandbox_id).await {
            return Ok(generation);
        }
        let read = self
            .store
            .sandbox_shape(sandbox_id)
            .await?
            .map(|shape| shape.token_generation);
        self.remember(sandbox_id, read).await;
        Ok(read)
    }
}

#[cfg(all(test, feature = "db-tests"))]
mod db_tests {
    use super::*;
    use crate::lifecycle::dao::PgLifecycleDao;
    use crate::lifecycle::test_support::seed_task;

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
}
