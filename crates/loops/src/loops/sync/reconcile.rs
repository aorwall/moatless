//! Fetch and reconcile Git-owned Loop declarations.

use super::error::LoopError;
use super::fetcher::{GitHubLoopFetcher, LoopFetcher};
use super::models::ParsedLoop;
use super::parser::parse_loop;
use super::resolve::{ResolveOutcome, declared_agent_type, managed_loop_input};
use crate::loops::dao::LoopDao;
use moatless_adapter_model::connections::ConnectionRouting;
use moatless_api_schemas::loops::{LoopSyncSkip, LoopSyncSummary, LoopSyncWarning};
use moatless_ports::identity::GitHostTokenIssuer;
use moatless_ports::task::TagResolver;
use moatless_tasks::agent::catalog;
use moatless_types::access_control::ScopeContext;
use moatless_workspace_ports::WorkspaceResolver;
use moatless_workspace_types::repository::{Repository, RepositoryProvider};
use std::collections::HashSet;
use std::sync::Arc;
use tracing::{debug, warn};

/// Reconciles a repository's committed loop declarations into the loop tables.
///
/// Holds the five ports the reconcile tree reads: adapter connections, loop
/// store, tags, workspaces, and the git-host token seam.
pub struct LoopSyncService {
    pub(super) fetcher: GitHubLoopFetcher,
    pub(super) adapter_connections: Arc<ConnectionRouting>,
    pub(super) loops: Arc<LoopDao>,
    pub(super) tags: Arc<dyn TagResolver>,
    pub(super) workspaces: Arc<dyn WorkspaceResolver>,
    pub(super) git_host_tokens: Arc<dyn GitHostTokenIssuer>,
}

impl LoopSyncService {
    /// Fetch, parse, and reconcile the loops declared in a repository.
    ///
    /// GitHub-only for v1 (non-GitHub repos return an empty summary with a
    /// warning); the fetch step can still fail hard, which callers surface or
    /// swallow as appropriate.
    pub async fn sync_repo_loops_for(
        &self,
        repo: &Repository,
        ctx: &ScopeContext,
    ) -> Result<LoopSyncSummary, LoopError> {
        let Some(remote) = repo.remote.as_ref() else {
            return Ok(LoopSyncSummary::default());
        };
        let provider = repo.provider;
        if provider != Some(RepositoryProvider::Github) {
            warn!(
                repository_id = %repo.id,
                provider = ?provider,
                "Loop sync only supports GitHub repos today — skipping"
            );
            return Ok(LoopSyncSummary::default());
        }

        let token = self.resolve_github_token(ctx, remote).await;
        let files = self
            .fetcher
            .fetch_loop_files(remote, token.as_deref(), repo.default_branch.as_deref())
            .await?;

        let present = files
            .iter()
            .map(|file| file.dir_name.clone())
            .collect::<HashSet<_>>();
        let mut parsed = Vec::new();
        let mut parse_skips = Vec::new();
        for file in files {
            match parse_loop(&file.content, &file.dir_name) {
                Ok(loop_) => parsed.push(loop_),
                Err(reason) => parse_skips.push(LoopSyncSkip {
                    loop_name: file.dir_name,
                    reason: format!("{reason}; existing Loop preserved"),
                }),
            }
        }

        self.sync_loops_snapshot(repo, ctx, parsed, present, parse_skips)
            .await
    }

    /// Reset one overridden or deleted Loop to its current Git declaration.
    ///
    /// Unlike repository reconciliation this never considers sibling removals:
    /// the row keeps its identity and its execution approval untouched — so a
    /// deleted Loop awaits approval and an overridden one keeps its run-as
    /// user.
    pub async fn reset_repo_loop_for(
        &self,
        repo: &Repository,
        ctx: &ScopeContext,
        source_key: &str,
    ) -> Result<bool, LoopError> {
        let remote = repo.remote.as_ref().ok_or_else(|| LoopError::Fetch {
            git_url: repo.id.clone(),
            message: "repository has no remote URL".to_string(),
        })?;
        let provider = repo.provider;
        if provider != Some(RepositoryProvider::Github) {
            return Err(LoopError::Fetch {
                git_url: remote.url.clone(),
                message: "Loop sync only supports GitHub repositories".to_string(),
            });
        }
        let token = self.resolve_github_token(ctx, remote).await;
        let files = self
            .fetcher
            .fetch_loop_files(remote, token.as_deref(), repo.default_branch.as_deref())
            .await?;
        let file = files
            .into_iter()
            .find(|file| file.dir_name == source_key)
            .ok_or_else(|| LoopError::Fetch {
                git_url: remote.url.clone(),
                message: format!("Loop directory `{source_key}` no longer exists"),
            })?;
        let parsed = parse_loop(&file.content, source_key)
            .map_err(|reason| LoopError::Internal(format!("Loop `{source_key}`: {reason}")))?;
        let resolved = match self.resolve_loop(repo, &parsed).await? {
            ResolveOutcome::Resolved(resolved) => resolved,
            ResolveOutcome::Skipped(reason) => return Err(LoopError::Internal(reason)),
        };
        let input = managed_loop_input(repo, ctx, &parsed, &resolved);
        self.loops
            .reset_managed_loop(&input)
            .await
            .map_err(LoopError::from)
    }

    /// Reconcile a set of already-parsed loops into the loop tables.
    ///
    /// Reconciles by `(source_repository_id, source_key)` and removes Git-owned
    /// rows whose Loop directory is confirmed absent.
    pub async fn sync_loops_for(
        &self,
        repo: &Repository,
        ctx: &ScopeContext,
        loops: Vec<ParsedLoop>,
    ) -> Result<LoopSyncSummary, LoopError> {
        let present = loops
            .iter()
            .map(|loop_| loop_.dir_name.clone())
            .collect::<HashSet<_>>();
        self.sync_loops_snapshot(repo, ctx, loops, present, Vec::new())
            .await
    }

    async fn sync_loops_snapshot(
        &self,
        repo: &Repository,
        ctx: &ScopeContext,
        loops: Vec<ParsedLoop>,
        present: HashSet<String>,
        parse_skips: Vec<LoopSyncSkip>,
    ) -> Result<LoopSyncSummary, LoopError> {
        let mut summary = LoopSyncSummary {
            skipped: parse_skips,
            ..LoopSyncSummary::default()
        };
        let mut desired = Vec::new();

        for parsed in &loops {
            if parsed.frontmatter.active.is_some() {
                summary.warnings.push(LoopSyncWarning {
                    loop_name: parsed.dir_name.clone(),
                    field: "active".to_string(),
                    message: "ignored: Git cannot approve Loop execution".to_string(),
                });
            }
            if parsed.frontmatter.run_as_user_id.is_some() {
                summary.warnings.push(LoopSyncWarning {
                    loop_name: parsed.dir_name.clone(),
                    field: "runAsUserId".to_string(),
                    message: "ignored: select the run-as user during administrator activation"
                        .to_string(),
                });
            }
            // A model no catalog agent offers still runs, because `model` is a
            // pass-through to the agent CLI, so it warns here; one another agent
            // offers is a `Skipped` from `resolve_loop`.
            if let Some(model) = parsed.frontmatter.model.as_deref() {
                let agent_type = declared_agent_type(&parsed.frontmatter);
                if !catalog::offers_model(&agent_type, model)
                    && !catalog::model_conflicts(&agent_type, model)
                {
                    summary.warnings.push(LoopSyncWarning {
                        loop_name: parsed.dir_name.clone(),
                        field: "model".to_string(),
                        message: format!(
                            "`{model}` is not one of the models `{agent_type}` offers; passed to the agent as written"
                        ),
                    });
                }
            }
            match self.resolve_loop(repo, parsed).await? {
                ResolveOutcome::Skipped(reason) => {
                    debug!(repository_id = %repo.id, loop = %parsed.dir_name, %reason, "Skipping loop");
                    summary.skipped.push(LoopSyncSkip {
                        loop_name: parsed.dir_name.clone(),
                        reason,
                    });
                }
                ResolveOutcome::Resolved(resolved) => {
                    desired.push(managed_loop_input(repo, ctx, parsed, &resolved));
                }
            }
        }

        let reconciled = self
            .loops
            .reconcile_managed_loops(&repo.id, &desired, &present)
            .await?;
        summary.created = reconciled.created;
        summary.updated = reconciled.updated;
        summary.removed = reconciled.removed;
        summary.overrides_skipped = reconciled.overrides_skipped;
        summary.skipped.extend(
        reconciled
            .skipped_overrides
            .into_iter()
            .map(|skip| LoopSyncSkip {
                loop_name: skip.source_key,
                reason: if skip.deleted {
                    "Git-origin Loop has a local deletion tombstone; reset it explicitly to restore"
                        .to_string()
                } else {
                    "Git-origin Loop has a local override; reset it explicitly to resume Git sync"
                        .to_string()
                },
            }),
    );

        Ok(summary)
    }
}
