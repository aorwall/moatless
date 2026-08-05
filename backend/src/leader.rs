//! Generic leader election trait.
//!
//! Abstracts leader election so modules like the cron scheduler can guard
//! against duplicate execution without depending on K8s directly. The trait is
//! dependency-free; the K8s Lease implementation lives beside the K8s client
//! (`sandbox::k8s::leader_election::K8sLeaderGuard`).

use async_trait::async_trait;

/// Trait for leader election guards.
///
/// Implementations decide whether this replica should run
/// singleton background tasks (cron scheduler, reapers, etc.).
#[async_trait]
pub trait LeaderGuard: Send + Sync {
    /// Try to acquire or renew leadership.
    /// Returns `true` if this instance is the current leader.
    async fn try_acquire_or_renew(&self) -> Result<bool, Box<dyn std::error::Error + Send + Sync>>;
}
