//! Who a queue worker claims rows as, shared by every claim/lease store here.

/// The claim identity written onto the records this process holds.
///
/// The pod hostname rather than a random per-process id, so a killed process
/// returns under the same name and its store's `release_claims_by_owner`
/// recovers its rows at once. The `HOSTNAME`-unset fallback stays unique per
/// process: a shared constant would let replicas steal live records.
pub(crate) fn worker_owner_id() -> String {
    std::env::var("HOSTNAME").unwrap_or_else(|_| format!("unknown-host-{}", uuid::Uuid::new_v4()))
}
