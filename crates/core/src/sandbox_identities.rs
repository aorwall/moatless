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

/// A service account and the network profile that lets it reach the API server.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SandboxIdentity {
    pub service_account: String,
    pub network_profile: Option<String>,
}

/// The identity a sandbox gets from its people's role when its Workspace names
/// neither a `sandboxServiceAccount` nor a `networkProfile`.
///
/// Env `SANDBOX_ROLE_IDENTITIES_JSON`, `{"admin": {...}, "user": {...}}`. A
/// missing role leaves that role's sandboxes on the deployment default.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleIdentities {
    pub admin: Option<SandboxIdentity>,
    pub user: Option<SandboxIdentity>,
}

impl RoleIdentities {
    /// Parses the env value; blank is empty. Malformed JSON is an error rather
    /// than empty: read as empty, it would silently drop every sandbox's access.
    pub fn parse(raw: Option<&str>) -> Result<Self, String> {
        match raw.map(str::trim).filter(|r| !r.is_empty()) {
            None => Ok(Self::default()),
            Some(raw) => serde_json::from_str(raw).map_err(|e| e.to_string()),
        }
    }

    /// The identity for a sandbox whose people are all admins (`admin`), else
    /// the `user` one.
    pub fn for_role(&self, admin: bool) -> Option<&SandboxIdentity> {
        if admin {
            self.admin.as_ref()
        } else {
            self.user.as_ref()
        }
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
    fn role_identities_parse_and_pick_by_role() {
        let roles = RoleIdentities::parse(Some(
            r#"{"admin":{"serviceAccount":"op","networkProfile":"op"},
                "user":{"serviceAccount":"reader"}}"#,
        ))
        .unwrap();
        assert_eq!(roles.for_role(true).unwrap().service_account, "op");
        assert_eq!(roles.for_role(false).unwrap().service_account, "reader");
        assert_eq!(roles.for_role(false).unwrap().network_profile, None);
        assert_eq!(
            RoleIdentities::parse(Some("  ")).unwrap(),
            RoleIdentities::default()
        );
        assert!(RoleIdentities::parse(Some("{\"admins\":{}}")).is_err());
    }

    #[test]
    fn blank_value_clears_rather_than_names() {
        let identities = SandboxIdentities::new(names(&["admin"]), names(&["moatless-admin"]));
        assert!(identities.check_network_profile("  ").is_ok());
        assert!(identities.check_service_account("").is_ok());
    }
}
