//! Which service proxies a Workspace may use, and what its pods are handed
//! for each.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use moatless_core::service_proxies::{
    Availability, BROKERED_PLACEHOLDER, ServiceProxies, ServiceProxy,
};
use moatless_core::workspace_identity_grants::WorkspaceIdentityGrant;
use moatless_types::credential_key::SERVICE_RELAY_ORIGIN;
use moatless_workspace_ports::WorkspaceResolver;
use moka::future::Cache;

pub use crate::proxy::grants::GrantReader;

/// How long a Workspace's enabled services are reused, which bounds how late a
/// withdrawn grant or opt-in takes effect.
const ACCESS_CACHE_TTL: Duration = Duration::from_secs(30);

/// Answers which services a Workspace has on: every `availability: all` one,
/// and each `granted` one both its `services` opt-in and its grant list.
pub struct ServiceAccess {
    services: Arc<ServiceProxies>,
    broker_secrets: bool,
    workspaces: Arc<dyn WorkspaceResolver>,
    grants: Arc<dyn GrantReader>,
    cache: Cache<String, Arc<Vec<String>>>,
}

impl ServiceAccess {
    pub fn new(
        services: Arc<ServiceProxies>,
        broker_secrets: bool,
        workspaces: Arc<dyn WorkspaceResolver>,
        grants: Arc<dyn GrantReader>,
    ) -> Self {
        Self {
            services,
            broker_secrets,
            workspaces,
            grants,
            cache: Cache::builder()
                .time_to_live(ACCESS_CACHE_TTL)
                .max_capacity(10_000)
                .build(),
        }
    }

    pub fn services(&self) -> &ServiceProxies {
        &self.services
    }

    /// The names of the services `workspace_id` has on, in name order.
    pub async fn enabled(&self, workspace_id: &str) -> Result<Arc<Vec<String>>, String> {
        if self.services.is_empty() {
            return Ok(Arc::default());
        }
        if let Some(cached) = self.cache.get(workspace_id).await {
            return Ok(cached);
        }
        let opted_in = self
            .workspaces
            .resolve(workspace_id)
            .await
            .map_err(|e| e.to_string())?
            .map(|workspace| workspace.services)
            .unwrap_or_default();
        let grant = if opted_in.is_empty() {
            None
        } else {
            self.grants.grant_for(workspace_id).await?
        };
        let enabled = Arc::new(
            self.services
                .iter()
                .filter(|service| available(service, &opted_in, grant.as_ref()))
                .map(|service| service.name.clone())
                .collect::<Vec<_>>(),
        );
        self.cache
            .insert(workspace_id.to_string(), enabled.clone())
            .await;
        Ok(enabled)
    }

    /// Rewrite the credentials a pod of `workspace_id` is handed: drop every
    /// brokered secret when brokering is on, and add each enabled service's
    /// relay URL and placeholder token.
    pub async fn shape_pod_credentials(
        &self,
        workspace_id: Option<&str>,
        credentials: &mut HashMap<String, String>,
    ) -> Result<(), String> {
        if self.broker_secrets {
            credentials.retain(|key, _| self.services.brokering(key).is_none());
        }
        let Some(workspace_id) = workspace_id else {
            return Ok(());
        };
        for name in self.enabled(workspace_id).await?.iter() {
            let Some(expose) = self.services.get(name).and_then(|s| s.expose.as_ref()) else {
                continue;
            };
            credentials.insert(
                expose.url_env.clone(),
                format!("{SERVICE_RELAY_ORIGIN}/{name}"),
            );
            if let Some(token_env) = &expose.token_env {
                // Left alone while brokering is off: a tool still calling the
                // upstream directly needs the real secret until it moves over.
                credentials
                    .entry(token_env.clone())
                    .or_insert_with(|| BROKERED_PLACEHOLDER.to_string());
            }
        }
        Ok(())
    }
}

/// Whether `service` is on for a Workspace listing `opted_in`, given its grant.
fn available(
    service: &ServiceProxy,
    opted_in: &[String],
    grant: Option<&WorkspaceIdentityGrant>,
) -> bool {
    match service.availability {
        Availability::All => true,
        Availability::Granted => {
            opted_in.contains(&service.name)
                && grant.is_some_and(|g| g.services.contains(&service.name))
        }
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use moatless_workspace_ports::{MockWorkspaceResolver, ResolvedWorkspace};

    use super::*;

    struct FixedGrant(Option<WorkspaceIdentityGrant>);

    #[async_trait]
    impl GrantReader for FixedGrant {
        async fn grant_for(&self, _: &str) -> Result<Option<WorkspaceIdentityGrant>, String> {
            Ok(self.0.clone())
        }
    }

    pub(crate) fn services() -> Arc<ServiceProxies> {
        let json = serde_json::json!({
            "grafana": {
                "upstream": "https://grafana.example.com",
                "credential": { "secret": "GRAFANA_TOKEN", "inject": { "style": "bearer" } },
                "expose": { "urlEnv": "GRAFANA_URL", "tokenEnv": "GRAFANA_TOKEN" },
                "availability": "granted",
                "rules": [{ "methods": ["GET"], "path": "/api/**" }]
            },
            "npm": {
                "upstream": "https://npm.pkg.github.com",
                "credential": { "secret": "NPM_TOKEN", "inject": { "style": "bearer" } },
                "expose": { "urlEnv": "NPM_URL" },
                "availability": "all",
                "rules": [{ "methods": ["GET"], "path": "/**" }]
            }
        });
        Arc::new(ServiceProxies::parse(Some(&json.to_string())).unwrap())
    }

    fn workspace(services: &[&str]) -> Arc<dyn WorkspaceResolver> {
        let services: Vec<String> = services.iter().map(|s| (*s).to_string()).collect();
        let mut resolver = MockWorkspaceResolver::new();
        resolver.expect_resolve().returning(move |id| {
            Ok(Some(ResolvedWorkspace {
                id: id.to_string(),
                name: "ws".to_string(),
                docker_image: None,
                docker_access: None,
                sandbox_service_account: None,
                system_prompt: None,
                setup_commands: None,
                env_vars: None,
                resources: None,
                language_servers: None,
                network_profile: None,
                node_selector: None,
                services: services.clone(),
                run_as_user_id: None,
                deleted: false,
            }))
        });
        Arc::new(resolver)
    }

    fn grant(services: &[&str]) -> Arc<dyn GrantReader> {
        Arc::new(FixedGrant(Some(WorkspaceIdentityGrant {
            repository: "a/b".to_string(),
            workspace: "default".to_string(),
            services: services.iter().map(|s| (*s).to_string()).collect(),
            ..WorkspaceIdentityGrant::default()
        })))
    }

    #[tokio::test]
    async fn a_granted_service_needs_both_the_opt_in_and_the_grant() {
        let opted_in = ServiceAccess::new(
            services(),
            false,
            workspace(&["grafana", "npm", "jira"]),
            grant(&[]),
        );
        assert_eq!(*opted_in.enabled("ws").await.unwrap(), vec!["npm"]);

        let granted = ServiceAccess::new(
            services(),
            false,
            workspace(&["grafana"]),
            grant(&["grafana"]),
        );
        assert_eq!(
            *granted.enabled("ws").await.unwrap(),
            vec!["grafana", "npm"]
        );

        let not_opted_in =
            ServiceAccess::new(services(), false, workspace(&[]), grant(&["grafana"]));
        assert_eq!(*not_opted_in.enabled("ws").await.unwrap(), vec!["npm"]);
    }

    #[tokio::test]
    async fn an_all_service_reaches_a_workspace_that_lists_nothing() {
        let mut credentials = HashMap::new();
        ServiceAccess::new(services(), true, workspace(&[]), grant(&[]))
            .shape_pod_credentials(Some("ws"), &mut credentials)
            .await
            .unwrap();
        assert_eq!(credentials["NPM_URL"], "http://127.0.0.1:7781/npm");
        assert!(!credentials.contains_key("GRAFANA_URL"));
    }

    #[tokio::test]
    async fn a_pod_gets_the_relay_url_and_a_placeholder_and_loses_the_secret_once_brokered() {
        let base = HashMap::from([
            ("GRAFANA_TOKEN".to_string(), "glsa-real".to_string()),
            ("OTHER".to_string(), "kept".to_string()),
        ]);

        let mut transition = base.clone();
        ServiceAccess::new(
            services(),
            false,
            workspace(&["grafana"]),
            grant(&["grafana"]),
        )
        .shape_pod_credentials(Some("ws"), &mut transition)
        .await
        .unwrap();
        assert_eq!(transition["GRAFANA_URL"], "http://127.0.0.1:7781/grafana");
        assert_eq!(transition["GRAFANA_TOKEN"], "glsa-real");

        let mut brokered = base.clone();
        ServiceAccess::new(
            services(),
            true,
            workspace(&["grafana"]),
            grant(&["grafana"]),
        )
        .shape_pod_credentials(Some("ws"), &mut brokered)
        .await
        .unwrap();
        assert_eq!(brokered["GRAFANA_TOKEN"], BROKERED_PLACEHOLDER);
        assert_eq!(brokered["OTHER"], "kept");

        let mut elsewhere = base;
        ServiceAccess::new(services(), true, workspace(&[]), grant(&[]))
            .shape_pod_credentials(Some("ws"), &mut elsewhere)
            .await
            .unwrap();
        assert!(!elsewhere.contains_key("GRAFANA_TOKEN"));
        assert!(!elsewhere.contains_key("GRAFANA_URL"));
    }
}
