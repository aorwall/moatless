use super::{InMemoryPreviewTabDao, PreviewTabDao, PreviewTabTtl, RedisPreviewTabDao};
use std::sync::Arc;
use tokio::task::JoinHandle;
use tracing::{info, warn};
/// Build the store this deployment's configuration calls for.
///
/// Returns the subscriber task to supervise when there is one; the in-memory
/// store publishes onto its own broadcast and has none.
pub async fn build(
    redis_url: Option<&str>,
    ttl: PreviewTabTtl,
) -> (Arc<dyn PreviewTabDao>, Option<JoinHandle<()>>) {
    let Some(redis_url) = redis_url else {
        info!("REDIS_URL not set — preview tabs are held in this process only");
        return (Arc::new(InMemoryPreviewTabDao::new()), None);
    };
    match RedisPreviewTabDao::connect(redis_url, ttl).await {
        Ok((store, forwarder)) => (Arc::new(store), Some(forwarder)),
        Err(error) => {
            // Degrading to the in-process store beats refusing to start: every
            // other T3 surface works without tabs, and a single-replica
            // deployment is indistinguishable from the Redis-backed one.
            warn!(%error, "Could not reach Redis for preview tabs — holding them in this process");
            (Arc::new(InMemoryPreviewTabDao::new()), None)
        }
    }
}
