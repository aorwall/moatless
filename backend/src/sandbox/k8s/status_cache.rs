//! Short-TTL cache for pod status queries.
//!
//! Avoids hammering the K8s API for frequently-checked pod statuses.

use moka::future::Cache;
use std::sync::Arc;
use std::time::Duration;

use crate::sandbox::types::PodPhase;

/// Cached pod status
#[derive(Debug, Clone)]
pub struct CachedPodStatus {
    pub phase: PodPhase,
    pub pod_name: Option<String>,
    pub last_error: Option<String>,
}

/// Cache for pod status lookups
#[derive(Clone)]
pub struct StatusCache {
    cache: Arc<Cache<String, CachedPodStatus>>,
}

impl StatusCache {
    /// Create a new status cache with a short TTL (5-10 seconds)
    pub fn new(ttl_secs: u64) -> Self {
        let cache = Cache::builder()
            .max_capacity(500)
            .time_to_live(Duration::from_secs(ttl_secs))
            .build();

        Self {
            cache: Arc::new(cache),
        }
    }

    /// Get cached status for a task
    pub async fn get(&self, task_id: &str) -> Option<CachedPodStatus> {
        self.cache.get(task_id).await
    }

    /// Cache a pod status
    pub async fn insert(&self, task_id: &str, status: CachedPodStatus) {
        self.cache.insert(task_id.to_string(), status).await;
    }

    /// Invalidate cached status for a task
    pub async fn invalidate(&self, task_id: &str) {
        self.cache.invalidate(task_id).await;
    }
}
