//! The privileged sandbox settings a git-declared Workspace may carry, as an
//! administrator grants them to one repository's one Workspace.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::sandbox_identities::SandboxIdentities;

/// One Workspace's grant: the values its `.moatless/workspaces.json` entry may
/// declare for each admin-only field, and the user it is private to.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceIdentityGrant {
    /// The declaring repository as `owner/name`, compared case-insensitively.
    pub repository: String,
    /// The entry's `source_key`, `default` for the repository's own Workspace.
    pub workspace: String,
    pub sandbox_service_account: Option<String>,
    pub network_profile: Option<String>,
    pub node_selector: Option<BTreeMap<String, String>>,
    /// Makes the Workspace user-scoped to this user rather than inheriting the
    /// repository's scope. Never read from git: a privileged identity would
    /// otherwise reach every user of a global repository.
    pub owner_user_id: Option<String>,
    /// Lets a sandbox token from this Workspace keep its administrator
    /// driver's role and reach every route (`moatless_identity`'s
    /// `sandbox_scope`). Never read from git, like every field here.
    #[serde(default)]
    pub agent_admin: bool,
    /// The `availability: granted` service proxies the Workspace may use once
    /// its `workspaces.json` entry lists them too.
    #[serde(default)]
    pub services: Vec<String>,
}

/// Every grant this deployment makes, read from `WORKSPACE_IDENTITY_GRANTS`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkspaceIdentityGrants(Vec<WorkspaceIdentityGrant>);

impl WorkspaceIdentityGrants {
    pub fn new(grants: Vec<WorkspaceIdentityGrant>) -> Self {
        Self(grants)
    }

    /// Parses the JSON array `WORKSPACE_IDENTITY_GRANTS` holds.
    pub fn parse(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json).map(Self)
    }

    /// Refuses a grant naming a network profile or service account
    /// `identities` does not define, so a typo fails startup rather than a
    /// sandbox start.
    pub fn check(&self, identities: &SandboxIdentities) -> Result<(), String> {
        for grant in &self.0 {
            let at = |e: String| format!("{}/{}: {e}", grant.repository, grant.workspace);
            if let Some(profile) = &grant.network_profile {
                identities.check_network_profile(profile).map_err(at)?;
            }
            if let Some(account) = &grant.sandbox_service_account {
                identities.check_service_account(account).map_err(at)?;
            }
        }
        Ok(())
    }

    /// Refuses a grant naming a service `defined` does not hold.
    pub fn check_services(&self, defined: impl Fn(&str) -> bool) -> Result<(), String> {
        for grant in &self.0 {
            if let Some(service) = grant.services.iter().find(|s| !defined(s)) {
                return Err(format!(
                    "{}/{}: services names `{service}`, which SERVICE_PROXIES_JSON does not define",
                    grant.repository, grant.workspace
                ));
            }
        }
        Ok(())
    }

    /// True iff some grant names a service or sets
    /// [`WorkspaceIdentityGrant::agent_admin`], so a Workspace's grant is
    /// worth reading.
    pub fn grants_proxy_access(&self) -> bool {
        self.0
            .iter()
            .any(|g| g.agent_admin || !g.services.is_empty())
    }

    /// True iff some grant sets [`WorkspaceIdentityGrant::agent_admin`].
    pub fn grants_agent_admin(&self) -> bool {
        self.0.iter().any(|g| g.agent_admin)
    }

    /// The grant for `workspace` in the repository `owner/name`, if any.
    pub fn find(&self, repository: &str, workspace: &str) -> Option<&WorkspaceIdentityGrant> {
        self.0
            .iter()
            .find(|g| g.repository.eq_ignore_ascii_case(repository) && g.workspace == workspace)
    }
}

/// The org sync and identity grant settings.
#[derive(Debug, Clone, Default)]
pub struct WorkspaceSyncConfig {
    /// `GITHUB_ORG_SYNC_ORGS`: the GitHub organizations (or users) whose
    /// repositories are registered as global repositories. Empty disables the
    /// org sync.
    pub github_orgs: Vec<String>,
    /// `GITHUB_ORG_SYNC_USER_ID`: the user whose GitHub credential lists the
    /// organizations, recorded as the creator of each repository it registers.
    pub github_org_sync_user_id: Option<String>,
    /// `WORKSPACE_IDENTITY_GRANTS`, a JSON array.
    pub identity_grants: WorkspaceIdentityGrants,
}

impl WorkspaceSyncConfig {
    /// Builds the settings from the three env values; blank reads as unset.
    pub fn parse(
        github_orgs: Vec<String>,
        github_org_sync_user_id: Option<String>,
        identity_grants: Option<String>,
    ) -> Result<Self, String> {
        let identity_grants = match identity_grants.filter(|json| !json.trim().is_empty()) {
            Some(json) => WorkspaceIdentityGrants::parse(&json)
                .map_err(|e| format!("WORKSPACE_IDENTITY_GRANTS: {e}"))?,
            None => WorkspaceIdentityGrants::default(),
        };
        let github_org_sync_user_id = github_org_sync_user_id.filter(|id| !id.trim().is_empty());
        if !github_orgs.is_empty() && github_org_sync_user_id.is_none() {
            return Err(
                "GITHUB_ORG_SYNC_ORGS is set but GITHUB_ORG_SYNC_USER_ID is not".to_string(),
            );
        }
        Ok(Self {
            github_orgs,
            github_org_sync_user_id,
            identity_grants,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_grant_is_found_by_repository_case_insensitively_and_by_exact_key() {
        let grants = WorkspaceIdentityGrants::parse(
            r#"[{"repository":"Soaplabs/soap-agent-ops","workspace":"admin",
                 "sandboxServiceAccount":"moatless-soap-nonprod-admin",
                 "nodeSelector":{"workload":"moatless-od"},"ownerUserId":"u1"}]"#,
        )
        .unwrap();

        let grant = grants.find("soaplabs/soap-agent-ops", "admin").unwrap();
        assert_eq!(
            grant.sandbox_service_account.as_deref(),
            Some("moatless-soap-nonprod-admin")
        );
        assert_eq!(grant.owner_user_id.as_deref(), Some("u1"));
        assert!(!grant.agent_admin);
        assert!(!grants.grants_agent_admin());
        assert!(grants.find("soaplabs/soap-agent-ops", "Admin").is_none());
        assert!(grants.find("soaplabs/soap-specs", "admin").is_none());
    }

    #[test]
    fn agent_admin_is_read_from_the_grant() {
        let grants = WorkspaceIdentityGrants::parse(
            r#"[{"repository":"a/b","workspace":"admin","agentAdmin":true}]"#,
        )
        .unwrap();

        assert!(grants.find("a/b", "admin").unwrap().agent_admin);
        assert!(grants.grants_agent_admin());
    }

    #[test]
    fn a_grant_naming_an_undefined_service_is_refused() {
        let grants = WorkspaceIdentityGrants::parse(
            r#"[{"repository":"a/b","workspace":"default","services":["grafana","jira"]}]"#,
        )
        .unwrap();
        assert!(grants.grants_proxy_access());
        let err = grants.check_services(|s| s == "grafana").unwrap_err();
        assert!(err.contains("`jira`"), "{err}");
        assert!(grants.check_services(|_| true).is_ok());
    }

    #[test]
    fn a_grant_naming_an_undefined_profile_is_refused() {
        let grants = WorkspaceIdentityGrants::parse(
            r#"[{"repository":"a/b","workspace":"default","networkProfile":"cluster-raeder"}]"#,
        )
        .unwrap();

        let err = grants
            .check(&SandboxIdentities::new(
                vec!["cluster-reader".into()],
                vec![],
            ))
            .unwrap_err();
        assert!(
            err.starts_with("a/b/default: networkProfile 'cluster-raeder'"),
            "{err}"
        );
    }

    #[test]
    fn org_sync_needs_its_acting_user() {
        let err =
            WorkspaceSyncConfig::parse(vec!["acme".into()], Some(" ".into()), None).unwrap_err();
        assert!(err.contains("GITHUB_ORG_SYNC_USER_ID"), "{err}");
        assert!(WorkspaceSyncConfig::parse(vec![], None, Some(String::new())).is_ok());
    }

    #[test]
    fn a_misspelled_field_fails_the_parse() {
        assert!(
            WorkspaceIdentityGrants::parse(
                r#"[{"repository":"a/b","workspace":"default","serviceAccount":"x"}]"#
            )
            .is_err()
        );
    }
}
