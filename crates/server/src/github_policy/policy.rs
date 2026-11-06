//! The policy's entry point: the cached scope per Workspace and the log line.

use std::sync::Arc;
use std::time::Duration;

use moatless_core::proxy_rules::{NormalPath, ProxyPolicyMode};
use moatless_workspace_ports::WorkspaceResolver;
use moka::future::Cache;
use tracing::warn;

use super::refusal::Refusal;
use super::scope::PolicyScope;
use super::{graphql, push, rest};
use crate::proxy::grants::GrantReader;

/// Where a request came from and what it asked, for the would-deny line.
pub(crate) struct Request<'a> {
    pub proxy: &'static str,
    pub sandbox_id: &'a str,
    pub workspace_id: &'a str,
    pub method: &'a str,
    pub path: String,
}

/// How long a Workspace's placements and grant are reused, which bounds how
/// late a withdrawn grant takes effect.
const SCOPE_CACHE_TTL: Duration = Duration::from_secs(30);

/// Decides the GitHub and git proxies' writes for a Workspace.
pub struct GitHubPolicy {
    mode: ProxyPolicyMode,
    workspaces: Arc<dyn WorkspaceResolver>,
    grants: Arc<dyn GrantReader>,
    cache: Cache<String, Arc<PolicyScope>>,
}

impl GitHubPolicy {
    pub fn new(
        mode: ProxyPolicyMode,
        workspaces: Arc<dyn WorkspaceResolver>,
        grants: Arc<dyn GrantReader>,
    ) -> Self {
        Self {
            mode,
            workspaces,
            grants,
            cache: Cache::builder()
                .time_to_live(SCOPE_CACHE_TTL)
                .max_capacity(10_000)
                .build(),
        }
    }

    /// The repositories and capabilities `workspace_id` writes with.
    async fn scope(&self, workspace_id: &str) -> Result<Arc<PolicyScope>, Refusal> {
        if let Some(cached) = self.cache.get(workspace_id).await {
            return Ok(cached);
        }
        let unreadable = |e: String| {
            warn!(error = %e, workspace_id, "Failed to read a workspace's GitHub policy");
            Refusal::new(
                "scope_unreadable",
                "Failed to read this workspace's repositories; retry",
            )
        };
        let placements = self
            .workspaces
            .placements(workspace_id)
            .await
            .map_err(|e| unreadable(e.to_string()))?;
        let grant = self
            .grants
            .grant_for(workspace_id)
            .await
            .map_err(unreadable)?;
        let scope = Arc::new(PolicyScope::new(
            placements.iter().filter_map(|p| p.repository.as_ref()),
            grant.as_ref().and_then(|g| g.github_policy.as_ref()),
        ));
        self.cache
            .insert(workspace_id.to_string(), scope.clone())
            .await;
        Ok(scope)
    }

    /// A REST request's refusal, if any. `body` is read only for a review.
    pub(crate) async fn check_rest(
        &self,
        workspace_id: &str,
        method: &str,
        path: &NormalPath,
        body: Option<&[u8]>,
    ) -> Result<(), Refusal> {
        if rest::is_read(method) {
            return Ok(());
        }
        let scope = self.scope(workspace_id).await?;
        rest::check(&scope, method, path, body)
    }

    /// Whether `method` writes, and so is checked at all.
    pub(crate) fn is_write(method: &str) -> bool {
        !rest::is_read(method)
    }

    /// Whether [`Self::check_rest`] reads the body of this request.
    pub(crate) fn rest_needs_body(method: &str, path: &NormalPath) -> bool {
        rest::needs_body(method, path)
    }

    /// Whether a GraphQL request runs a mutation and every one it runs is in
    /// `mutations`; an unreadable one runs none.
    pub(crate) fn graphql_runs_only(body: &[u8], mutations: &[&str]) -> bool {
        graphql::Document::parse(body).is_ok_and(|document| {
            let mut names = document.mutation_names().peekable();
            names.peek().is_some() && names.all(|m| mutations.contains(&m))
        })
    }

    /// A GraphQL request's refusal, if any.
    pub(crate) async fn check_graphql(
        &self,
        workspace_id: &str,
        body: &[u8],
    ) -> Result<(), Refusal> {
        let document = graphql::Document::parse(body)?;
        if !document.writes() {
            return graphql::check(&document, None);
        }
        let scope = self.scope(workspace_id).await?;
        graphql::check(&document, Some(&scope))
    }

    /// Each pushed ref's refusal, in `commands` order; empty when every ref
    /// may be pushed.
    pub(crate) async fn check_push(
        &self,
        workspace_id: &str,
        repository: &str,
        commands: &[push::RefUpdate],
    ) -> Vec<(String, Refusal)> {
        match self.scope(workspace_id).await {
            Ok(scope) => push::check(&scope, repository, commands),
            Err(refusal) => commands
                .iter()
                .map(|c| (c.name.clone(), refusal.clone()))
                .collect(),
        }
    }

    /// Log `refusal` and say whether to act on it.
    pub(crate) fn refuses(&self, request: &Request<'_>, refusal: &Refusal) -> bool {
        let enforced = self.mode == ProxyPolicyMode::Enforce;
        warn!(
            proxy = request.proxy,
            sandbox_id = request.sandbox_id,
            workspace_id = request.workspace_id,
            method = request.method,
            path = %request.path,
            rule = refusal.rule,
            reason = %refusal.message,
            enforced,
            "Proxy request outside its policy"
        );
        enforced
    }
}
