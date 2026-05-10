//! Kubernetes Lease-based leader election.
//!
//! Uses the `coordination.k8s.io/v1` Lease API for leader election among
//! multiple replicas. Only the leader runs the pod lifecycle watcher to
//! avoid duplicate event processing.

use k8s_openapi::api::coordination::v1::Lease;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::MicroTime;
use k8s_openapi::jiff;
use kube::api::{Api, PostParams};
use tracing::{debug, info};

use super::client::K8sClient;

/// Leader election via K8s Lease objects.
pub struct LeaderElection {
    leases: Api<Lease>,
    lease_name: String,
    identity: String,
    /// How long a lease is valid before it can be taken over (seconds).
    lease_duration_secs: i32,
}

impl LeaderElection {
    /// Create a new leader election instance with the default lease name.
    ///
    /// `identity` should be unique per replica (e.g., HOSTNAME).
    pub fn new(k8s: &K8sClient, identity: String) -> Self {
        Self::new_with_name(k8s, identity, "moatless-pod-watcher".to_string())
    }

    /// Create a new leader election instance with a custom lease name.
    ///
    /// Use distinct lease names for independent leader-elected subsystems
    /// (e.g., `"moatless-pod-watcher"` vs `"moatless-task-reaper"`).
    pub fn new_with_name(k8s: &K8sClient, identity: String, lease_name: String) -> Self {
        Self {
            leases: Api::namespaced(k8s.client.clone(), k8s.namespace()),
            lease_name,
            identity,
            lease_duration_secs: 15,
        }
    }

    /// Set a custom lease duration (in seconds).
    ///
    /// The lease duration must be **longer** than the interval between
    /// `try_acquire_or_renew` calls, otherwise the lease will always
    /// appear expired and leadership will be re-acquired every cycle.
    pub fn with_lease_duration(mut self, secs: i32) -> Self {
        self.lease_duration_secs = secs;
        self
    }

    /// Try to acquire or renew the lease.
    ///
    /// Returns `true` if this instance is the leader after the call.
    ///
    /// Algorithm:
    /// 1. GET the Lease. If 404, CREATE it (we become leader).
    /// 2. If we hold it, UPDATE renewTime.
    /// 3. If someone else holds it and it's expired, take over.
    /// 4. Otherwise, we're not the leader.
    pub async fn try_acquire_or_renew(&self) -> Result<bool, kube::Error> {
        let now = jiff::Timestamp::now();

        match self.leases.get(&self.lease_name).await {
            Ok(lease) => {
                let holder = lease
                    .spec
                    .as_ref()
                    .and_then(|s| s.holder_identity.as_deref());
                let renew_time = lease
                    .spec
                    .as_ref()
                    .and_then(|s| s.renew_time.as_ref())
                    .map(|t| t.0);
                let duration = lease
                    .spec
                    .as_ref()
                    .and_then(|s| s.lease_duration_seconds)
                    .unwrap_or(self.lease_duration_secs);

                let is_expired = renew_time.is_none_or(|t| {
                    let elapsed = now.duration_since(t);
                    elapsed.as_secs() > duration as i64
                });

                if holder == Some(self.identity.as_str()) {
                    // We hold it — renew
                    self.update_lease(&lease).await?;
                    debug!(identity = %self.identity, lease = %self.lease_name, "Renewed leadership");
                    Ok(true)
                } else if is_expired {
                    // Expired — take over
                    info!(
                        identity = %self.identity,
                        lease = %self.lease_name,
                        prev_holder = ?holder,
                        "Taking over expired lease"
                    );
                    self.update_lease(&lease).await?;
                    Ok(true)
                } else {
                    debug!(
                        identity = %self.identity,
                        lease = %self.lease_name,
                        holder = ?holder,
                        "Not leader, lease held by another"
                    );
                    Ok(false)
                }
            }
            Err(kube::Error::Api(ref e)) if e.code == 404 => {
                // Lease doesn't exist — create it
                self.create_lease().await?;
                info!(identity = %self.identity, lease = %self.lease_name, "Acquired leadership (created lease)");
                Ok(true)
            }
            Err(e) => Err(e),
        }
    }

    /// Create a new Lease (first-time acquisition).
    async fn create_lease(&self) -> Result<(), kube::Error> {
        let now = jiff::Timestamp::now();
        let now_str = now.to_string();
        #[allow(clippy::expect_used)] // static JSON shape, deserialization cannot fail at runtime
        let lease: Lease = serde_json::from_value(serde_json::json!({
            "apiVersion": "coordination.k8s.io/v1",
            "kind": "Lease",
            "metadata": {
                "name": self.lease_name,
            },
            "spec": {
                "holderIdentity": self.identity,
                "leaseDurationSeconds": self.lease_duration_secs,
                "acquireTime": now_str,
                "renewTime": now_str,
            }
        }))
        .expect("valid lease JSON");

        match self.leases.create(&PostParams::default(), &lease).await {
            Ok(_) => Ok(()),
            Err(kube::Error::Api(ref e)) if e.code == 409 => {
                // Another replica created it first — not fatal
                debug!(identity = %self.identity, lease = %self.lease_name, "Lease already exists (race)");
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    /// Update an existing Lease (renew or take over).
    ///
    /// Uses `replace` (PUT) with the existing `resourceVersion` for
    /// optimistic concurrency. This is more reliable than merge-patch
    /// because the full object is written back atomically.
    async fn update_lease(&self, existing: &Lease) -> Result<(), kube::Error> {
        let now = jiff::Timestamp::now();
        let mut lease = existing.clone();
        let spec = lease.spec.get_or_insert_with(Default::default);
        spec.holder_identity = Some(self.identity.clone());
        spec.lease_duration_seconds = Some(self.lease_duration_secs);
        spec.renew_time = Some(MicroTime(now));

        self.leases
            .replace(&self.lease_name, &PostParams::default(), &lease)
            .await?;

        Ok(())
    }
}
