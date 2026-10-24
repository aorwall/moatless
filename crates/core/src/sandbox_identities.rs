//! The network profiles and service accounts a Workspace may name, as the
//! Helm chart that creates them reports them.

/// The sandbox identities this deployment defines.
///
/// An empty list leaves its field unchecked, so a deployment that sets neither
/// env accepts every name, as it did before the chart passed them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SandboxIdentities {
    network_profiles: Vec<String>,
    service_accounts: Vec<String>,
}

impl SandboxIdentities {
    /// Do not add the deployment's default service account implicitly: a pod
    /// that names it mounts its token (see `build_pod`), and on a default
    /// install that account is the backend's own.
    pub fn new(network_profiles: Vec<String>, service_accounts: Vec<String>) -> Self {
        Self {
            network_profiles,
            service_accounts,
        }
    }

    /// Refuses a `networkProfile` no chart template selects.
    ///
    /// A blank value clears the field, so it always passes.
    pub fn check_network_profile(&self, profile: &str) -> Result<(), String> {
        check(&self.network_profiles, profile).map_err(|known| {
            format!("networkProfile '{profile}' is not defined by this deployment; known: {known}")
        })
    }

    /// Refuses a `sandboxServiceAccount` outside the deployment's allowlist.
    ///
    /// A blank value clears the field, so it always passes.
    pub fn check_service_account(&self, account: &str) -> Result<(), String> {
        check(&self.service_accounts, account).map_err(|permitted| {
            format!(
                "sandboxServiceAccount '{account}' is not permitted by this deployment; \
                 permitted: {permitted}"
            )
        })
    }
}

/// `Err` carries the comma-joined `allowed` list for the message.
///
/// Compares untrimmed: the value is stored as sent, and Kubernetes rejects a
/// padded `serviceAccountName` or label value.
fn check(allowed: &[String], name: &str) -> Result<(), String> {
    if allowed.is_empty() || name.trim().is_empty() || allowed.iter().any(|a| a == name) {
        return Ok(());
    }
    Err(allowed.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn empty_lists_accept_every_name() {
        let identities = SandboxIdentities::new(vec![], vec![]);
        assert!(identities.check_network_profile("anything").is_ok());
        assert!(identities.check_service_account("moatless-backend").is_ok());
    }

    #[test]
    fn unknown_network_profile_is_refused() {
        let identities = SandboxIdentities::new(names(&["admin", "sealed-egress"]), vec![]);
        assert!(identities.check_network_profile("admin").is_ok());
        let err = identities.check_network_profile("admn").unwrap_err();
        assert!(err.contains("'admn'") && err.contains("admin, sealed-egress"));
    }

    #[test]
    fn service_account_outside_allowlist_is_refused() {
        let identities = SandboxIdentities::new(vec![], names(&["moatless-cluster-reader"]));
        assert!(
            identities
                .check_service_account("moatless-cluster-reader")
                .is_ok()
        );
        assert!(
            identities
                .check_service_account("moatless-backend")
                .is_err(),
            "the deployment's default SA is refused unless listed"
        );
        assert!(
            identities
                .check_service_account(" moatless-cluster-reader ")
                .is_err(),
            "a padded name is not the listed account"
        );
    }

    #[test]
    fn blank_value_clears_rather_than_names() {
        let identities = SandboxIdentities::new(names(&["admin"]), names(&["moatless-admin"]));
        assert!(identities.check_network_profile("  ").is_ok());
        assert!(identities.check_service_account("").is_ok());
    }
}
