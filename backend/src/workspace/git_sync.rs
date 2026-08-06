//! Reconcile a repo's git-declared workspaces into `workspaces` rows.
//!
//! A repository can declare workspaces in its git tree: the default form is
//! `.moatless/workspaces.json` (a list of workspaces, each with its own
//! run-config and explicit repository URL(s)); the legacy form is
//! `.moatless/config.json` (a single workspace's run-config). Both reconcile
//! into `workspaces` rows flagged `source = 'git'`, keyed by
//! `(source_repository_id, source_key)`, so reconciliation never touches
//! manually-created workspaces. This mirrors `loops/sync.rs`.
//!
//! Git-synced workspaces are read-only in the UI and write API — only a re-sync
//! from git changes them. The sync writes through the DAO directly, bypassing
//! the read-only guard by design. The `.moatless/` file schemas + provider
//! fetchers live here too — the repository module owns only the git connection.
//!
//! A workspace can be taken out of git's hands from the UI/CLI: `override` flips
//! it to `source = 'manual'` while keeping its provenance columns, so the
//! automatic sync on task start leaves it alone. [`SyncForce`] decides whether a
//! given sync respects that — only the explicit, user-initiated syncs
//! (`sync-config`, per-workspace `reset-git`) reclaim an active override,
//! replacing the local edits with the committed config. A deleted override is a
//! tombstone and is only restored by resetting that specific workspace.

use std::collections::HashSet;
use std::sync::Arc;

use chrono::Utc;
use serde::Deserialize;
use tracing::{debug, info, warn};

use crate::auth::provider_token_dao::ProviderTokenDao;
use moatless_core::db::DbPool;
use crate::extractors::AuthUser;
use crate::repository::access::{access_error_for, RepositoryTokenResolver};
use crate::repository::config_reader::{
    fetch_provider_file, verify_repository_access, AccessStatus,
};
use crate::repository::dao::{RepositoryDao, RepositoryPatch};
use crate::repository::error::{is_remote_url_conflict, RepositoryError};
use crate::repository::handlers::{normalize_remote_url, validate_non_secret_env_vars};
use crate::repository::models::{Repository, RepositoryKind, RepositoryProvider, ResourceConfig};
use crate::repository::models::ServerConfig;
use crate::workspace::dao::{ensure_shadow_workspace, WorkspaceDao, WorkspacePatch};
use crate::workspace::models::{Workspace, WorkspaceRepo};
use moatless_api_schemas::workspace::{WorkspaceSyncSkip, WorkspaceSyncSummary};
use moatless_backend_core::scope::ScopeContext;

// =============================================================================
// `.moatless/` file schemas
// =============================================================================

/// Top-level `.moatless/config.json` schema.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoatlessRepoConfig {
    pub docker_image: Option<String>,
    pub code_execution: Option<CodeExecution>,
    pub servers: Option<Vec<ServerConfig>>,
    pub system_prompt: Option<String>,
    pub resources: Option<ResourceConfig>,
    /// Per-repo Kubernetes ServiceAccount for this repo's sandboxes. Overrides the
    /// global default. Security: the named SA must exist and be RBAC-bound in the
    /// sandbox namespace — this field only selects it, it grants nothing on its own.
    pub sandbox_service_account: Option<String>,
    /// Per-repo network profile — pods carry `moatless.io/network-profile=<value>`
    /// for a static NetworkPolicy to select. Selects a policy; grants nothing.
    pub network_profile: Option<String>,
}

/// Code execution settings.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeExecution {
    pub setup_commands: Option<Vec<String>>,
}

/// Top-level `.moatless/workspaces.json` schema — the default (non-legacy) form,
/// a list of workspaces the repo declares, each with its own run-config and
/// explicit repository URL(s).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoatlessWorkspacesFile {
    #[allow(dead_code)] // reserved for forward-compat; not yet branched on
    pub version: Option<u32>,
    pub workspaces: Vec<MoatlessWorkspaceEntry>,
}

/// One workspace entry in `.moatless/workspaces.json`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoatlessWorkspaceEntry {
    /// Stable natural key within the repo. When present, renaming `name` keeps
    /// the workspace identity; when absent, the slugified `name` is the key.
    pub key: Option<String>,
    pub name: String,
    pub description: Option<String>,
    pub repos: Vec<MoatlessWorkspaceRepoRef>,
    // --- Run configuration (mirrors `.moatless/config.json`) ---
    pub docker_image: Option<String>,
    pub docker_access: Option<bool>,
    pub code_execution: Option<CodeExecution>,
    pub servers: Option<Vec<ServerConfig>>,
    pub system_prompt: Option<String>,
    pub resources: Option<ResourceConfig>,
    pub env_vars: Option<std::collections::HashMap<String, String>>,
    pub sandbox_service_account: Option<String>,
    pub network_profile: Option<String>,
}

/// A repository reference inside a `workspaces.json` entry.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoatlessWorkspaceRepoRef {
    pub url: String,
    pub branch: Option<String>,
    pub mount_name: Option<String>,
    #[serde(default)]
    pub primary: bool,
}

/// Fetch `.moatless/config.json` from a remote repository.
///
/// Returns `Ok(None)` when the file does not exist (HTTP 404).
/// Token is optional — GitHub public repos can be fetched without auth.
pub async fn fetch_moatless_config(
    remote_url: &str,
    branch: Option<&str>,
    token: Option<&str>,
    provider: RepositoryProvider,
) -> Result<Option<MoatlessRepoConfig>, RepositoryError> {
    let Some(body) =
        fetch_provider_file(remote_url, ".moatless/config.json", branch, token, provider).await?
    else {
        return Ok(None);
    };
    let config = serde_json::from_str(&body).map_err(|e| {
        RepositoryError::ProviderError(format!("Failed to parse .moatless/config.json: {e}"))
    })?;
    Ok(Some(config))
}

/// Fetch and parse `.moatless/workspaces.json` from a remote repository.
///
/// Returns `Ok(None)` when the file does not exist. This is the default
/// (non-legacy) workspace declaration form; `.moatless/config.json` remains the
/// legacy single-workspace form.
pub async fn fetch_moatless_workspaces(
    remote_url: &str,
    branch: Option<&str>,
    token: Option<&str>,
    provider: RepositoryProvider,
) -> Result<Option<MoatlessWorkspacesFile>, RepositoryError> {
    let Some(body) = fetch_provider_file(
        remote_url,
        ".moatless/workspaces.json",
        branch,
        token,
        provider,
    )
    .await?
    else {
        return Ok(None);
    };
    let file = serde_json::from_str(&body).map_err(|e| {
        RepositoryError::ProviderError(format!("Failed to parse .moatless/workspaces.json: {e}"))
    })?;
    Ok(Some(file))
}

// =============================================================================
// Sync service
// =============================================================================

/// The deterministic `source_key` for the legacy single `.moatless/config.json`
/// workspace, which flags a repo's existing shadow workspace in place.
const CONFIG_JSON_KEY: &str = "__config_json__";

/// Whether a sync reclaims workspaces that were manually overridden.
///
/// An override (`source = 'manual'`) exists to survive the automatic sync that
/// runs on task start; only an explicit, user-initiated sync discards it.
pub enum SyncForce {
    /// Leave overridden workspaces alone (task-start auto-sync, repo create).
    RespectOverrides,
    /// Reclaim every overridden workspace in the repo (`sync-config`).
    All,
    /// Reclaim just this one workspace (per-workspace `reset-git`).
    Only(String),
}

impl SyncForce {
    /// Whether an overridden workspace with this id is taken back under git.
    fn reclaims(&self, workspace_id: &str) -> bool {
        match self {
            Self::RespectOverrides => false,
            Self::All => true,
            Self::Only(id) => id == workspace_id,
        }
    }
}

/// What happened to one git-declared workspace during a sync.
enum UpsertOutcome {
    Created,
    Updated,
    /// Was manually overridden and taken back under git control.
    Reclaimed,
    /// Manually overridden and left untouched.
    Skipped,
}

/// Reconciles a repository's git-declared workspaces, holding the ports the
/// reconcile tree threads through every frame (the workspace, repository, and
/// provider-token DAOs, the pool, and the repository token resolver).
pub struct GitSyncService {
    db: DbPool,
    workspaces: Arc<dyn WorkspaceDao>,
    repositories: Arc<dyn RepositoryDao>,
    provider_tokens: Arc<dyn ProviderTokenDao>,
    repo_token_resolver: Arc<RepositoryTokenResolver>,
}

impl GitSyncService {
    pub fn new(
        db: DbPool,
        workspaces: Arc<dyn WorkspaceDao>,
        repositories: Arc<dyn RepositoryDao>,
        provider_tokens: Arc<dyn ProviderTokenDao>,
        repo_token_resolver: Arc<RepositoryTokenResolver>,
    ) -> Self {
        Self {
            db,
            workspaces,
            repositories,
            provider_tokens,
            repo_token_resolver,
        }
    }

    /// Fetch, resolve, and reconcile the workspaces declared in a repository.
    ///
    /// `workspaces.json` wins when present (its entries are the desired set, and any
    /// `config.json` git workspace is pruned by reconcile-delete). Otherwise a
    /// present `config.json` flags the repo's shadow workspace `ws_<repo_id>` in
    /// place. Best-effort/non-fatal: callers log the outcome.
    pub async fn sync_repo_workspaces(
        &self,
        repo: &Repository,
        auth: &AuthUser,
        force: &SyncForce,
    ) -> Result<WorkspaceSyncSummary, RepositoryError> {
        let mut summary = WorkspaceSyncSummary::default();

        let Some(remote_url) = repo.remote_url.as_deref() else {
            return Ok(summary);
        };
        let provider = repo.provider.unwrap_or(RepositoryProvider::Github);
        let token = self
            .resolve_sync_token(repo, auth, provider, remote_url)
            .await;
        let token_str = token.as_deref();
        let branch = repo.default_branch.as_deref();

        // source_keys present in git this round — reconcile-delete keeps only these.
        let mut present: HashSet<String> = HashSet::new();

        match fetch_moatless_workspaces(remote_url, branch, token_str, provider).await? {
            Some(file) => {
                // workspaces.json wins. Its entries are the desired set; the
                // config.json git workspace (if any) is pruned by reconcile-delete
                // because CONFIG_JSON_KEY is never inserted into `present`.
                let mut seen_keys: HashSet<String> = HashSet::new();
                for entry in &file.workspaces {
                    let source_key = source_key_for(entry);
                    if !seen_keys.insert(source_key.clone()) {
                        // A duplicate source_key would violate workspaces_git_source_unique.
                        warn!(repository_id = %repo.id, %source_key, "Duplicate workspace source_key; skipping later entry");
                        summary.skipped.push(WorkspaceSyncSkip {
                            source_key,
                            reason: "duplicate source_key".to_string(),
                        });
                        continue;
                    }
                    if let Err(e) = validate_non_secret_env_vars(entry.env_vars.as_ref()) {
                        warn!(repository_id = %repo.id, %source_key, error = %e, "Skipping workspace with invalid envVars");
                        summary.skipped.push(WorkspaceSyncSkip {
                            source_key,
                            reason: e.to_string(),
                        });
                        continue;
                    }

                    let run = match run_config_from_entry(entry) {
                        Ok(run) => run,
                        Err(e) => {
                            summary.skipped.push(WorkspaceSyncSkip {
                                source_key,
                                reason: format!("serialize run-config: {e}"),
                            });
                            continue;
                        }
                    };
                    let placements = self.resolve_placements(repo, &entry.repos, auth).await?;

                    present.insert(source_key.clone());
                    let desired = DesiredWorkspace {
                        fixed_id: None,
                        source_key,
                        source_config_path: ".moatless/workspaces.json".to_string(),
                        name: entry.name.clone(),
                        description: entry.description.clone(),
                        run,
                        placements,
                    };
                    record_outcome(
                        &mut summary,
                        self.upsert_git_workspace(repo, &desired, force).await?,
                        &desired.source_key,
                    );
                }
            }
            None => {
                // Fall back to config.json: flag the repo's shadow workspace in place.
                if let Some(config) =
                    fetch_moatless_config(remote_url, branch, token_str, provider).await?
                {
                    let run = match run_config_from_moatless_config(&config) {
                        Ok(run) => run,
                        Err(e) => {
                            summary.skipped.push(WorkspaceSyncSkip {
                                source_key: CONFIG_JSON_KEY.to_string(),
                                reason: format!("serialize run-config: {e}"),
                            });
                            return Ok(summary);
                        }
                    };
                    present.insert(CONFIG_JSON_KEY.to_string());
                    let desired = DesiredWorkspace {
                        fixed_id: Some(format!("ws_{}", repo.id)),
                        source_key: CONFIG_JSON_KEY.to_string(),
                        source_config_path: ".moatless/config.json".to_string(),
                        name: repo.name.clone(),
                        description: repo.description.clone(),
                        // The shadow mirrors the run-config declared in config.json.
                        run,
                        placements: vec![ResolvedPlacement {
                            repository_id: repo.id.clone(),
                            branch: None,
                            mount_name: None,
                            primary: true,
                        }],
                    };
                    record_outcome(
                        &mut summary,
                        self.upsert_git_workspace(repo, &desired, force).await?,
                        &desired.source_key,
                    );
                } else {
                    debug!(repository_id = %repo.id, "No .moatless/workspaces.json or config.json; nothing to sync");
                }
            }
        }

        // Reconcile-delete: soft-delete any git workspace for this repo whose
        // source_key is no longer declared. Soft so in-flight tasks pointing at it
        // keep working — provision lookups don't filter `deleted`.
        let existing: Vec<(String, Option<String>)> = sqlx::query_as(
            "SELECT id, source_key FROM workspaces \
         WHERE source_repository_id = $1 AND source = 'git' \
           AND (deleted IS NULL OR deleted = false)",
        )
        .bind(&repo.id)
        .fetch_all(&self.db)
        .await?;
        // reason: git reconcile mirrors repository content across the repo's own
        // workspaces; the caller's access to trigger the sync was already checked.
        #[allow(clippy::disallowed_methods)]
        let sys_ctx = ScopeContext::system();
        for (id, key) in existing {
            let keep = key.as_deref().is_some_and(|k| present.contains(k));
            if !keep {
                self.workspaces.soft_delete(&id, &sys_ctx).await?;
                summary.removed += 1;
            }
        }

        Ok(summary)
    }

    /// Best-effort auto-sync of a repo's workspace config when a task starts.
    /// Gated on the repo actually shipping a `.moatless/` directory so we don't
    /// hit the provider API for repos that don't use it. Every failure is logged
    /// and swallowed — this must never block or fail a task start. Loops are
    /// never synchronized as a side effect of task start.
    pub async fn sync_repo_config_on_start(&self, auth: &AuthUser, repo: &Repository) {
        let Some(remote_url) = repo.remote_url.as_deref() else {
            return;
        };
        let provider = repo.provider.unwrap_or(RepositoryProvider::Github);

        // Resolve a token for the existence probe; fall back to an unauthenticated
        // probe (works for public repos) if we can't load one.
        let token = if provider == RepositoryProvider::Github {
            self.repo_token_resolver
                .resolve(repo, remote_url)
                .await
                .unwrap_or_else(|e| {
                    warn!(repository_id = %repo.id, error = %e, "Config auto-sync: failed to resolve GitHub token; probing unauthenticated");
                    None
                })
        } else {
            match self
                .provider_tokens
                .find_provider_token(&auth.user_id.to_string(), provider.as_str())
                .await
            {
                Ok(token) => token.map(|t| t.access_token),
                Err(e) => {
                    warn!(repository_id = %repo.id, error = %e, "Config auto-sync: failed to load provider token; probing unauthenticated");
                    None
                }
            }
        };

        let token_str = token.as_deref();
        let branch = repo.default_branch.as_deref();

        if !crate::repository::config_reader::moatless_dir_exists(
            remote_url, branch, token_str, provider,
        )
        .await
        {
            debug!(repository_id = %repo.id, "Config auto-sync: no .moatless directory, skipping");
            return;
        }

        // Automatic sync: workspaces the user overrode from the UI/CLI stay
        // overridden — only an explicit sync-config or reset-git reclaims them.
        let force = SyncForce::RespectOverrides;
        match self.sync_repo_workspaces(repo, auth, &force).await {
            Ok(workspaces) => {
                info!(
                    repository_id = %repo.id,
                    workspaces_created = workspaces.created,
                    workspaces_updated = workspaces.updated,
                    workspaces_overridden = workspaces.skipped.len(),
                    "Workspace config auto-sync on task start complete"
                );
            }
            Err(e) => {
                warn!(repository_id = %repo.id, error = %e, "Workspace config auto-sync on task start failed");
            }
        }
    }
}

/// Fold one upsert outcome into the running summary.
fn record_outcome(summary: &mut WorkspaceSyncSummary, outcome: UpsertOutcome, source_key: &str) {
    match outcome {
        UpsertOutcome::Created => summary.created += 1,
        UpsertOutcome::Updated => summary.updated += 1,
        UpsertOutcome::Reclaimed => summary.reclaimed += 1,
        UpsertOutcome::Skipped => summary.skipped.push(WorkspaceSyncSkip {
            source_key: source_key.to_string(),
            reason: "manually overridden".to_string(),
        }),
    }
}

/// Run-config fields shared by the entry and config.json paths.
struct RunConfig {
    docker_image: Option<String>,
    docker_access: Option<bool>,
    sandbox_service_account: Option<String>,
    network_profile: Option<String>,
    system_prompt: Option<String>,
    setup_commands: Option<serde_json::Value>,
    env_vars: Option<serde_json::Value>,
    servers: Option<serde_json::Value>,
    resources: Option<serde_json::Value>,
}

/// A repository placement whose URL has already been resolved to a repo id.
struct ResolvedPlacement {
    repository_id: String,
    branch: Option<String>,
    mount_name: Option<String>,
    primary: bool,
}

/// A git-declared workspace resolved to concrete values, ready to upsert.
struct DesiredWorkspace {
    /// Deterministic id for the config.json shadow (`ws_<repo_id>`); `None` for
    /// workspaces.json entries (found by natural key, else a fresh id).
    fixed_id: Option<String>,
    source_key: String,
    /// Path to the declaring file within the repository, e.g.
    /// `.moatless/workspaces.json` or `.moatless/config.json`.
    source_config_path: String,
    name: String,
    description: Option<String>,
    run: RunConfig,
    placements: Vec<ResolvedPlacement>,
}

/// The stable natural key for a workspaces.json entry: its `key`, else the
/// slugified `name`. Documenting `key` as the stability contract — renaming a
/// keyed entry updates in place; a name-keyed entry churns (prune + create).
fn source_key_for(entry: &MoatlessWorkspaceEntry) -> String {
    entry
        .key
        .as_deref()
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .map_or_else(|| slugify(&entry.name), str::to_string)
}

fn run_config_from_entry(entry: &MoatlessWorkspaceEntry) -> Result<RunConfig, serde_json::Error> {
    let setup_commands = entry
        .code_execution
        .as_ref()
        .and_then(|c| c.setup_commands.as_ref())
        .map(serde_json::to_value)
        .transpose()?;
    Ok(RunConfig {
        docker_image: entry.docker_image.clone(),
        docker_access: entry.docker_access,
        sandbox_service_account: entry.sandbox_service_account.clone(),
        network_profile: entry.network_profile.clone(),
        system_prompt: entry.system_prompt.clone(),
        setup_commands,
        env_vars: entry
            .env_vars
            .as_ref()
            .map(serde_json::to_value)
            .transpose()?,
        servers: entry
            .servers
            .as_ref()
            .map(serde_json::to_value)
            .transpose()?,
        resources: entry
            .resources
            .as_ref()
            .map(serde_json::to_value)
            .transpose()?,
    })
}

/// Build the shadow workspace's run-config from a parsed legacy
/// `.moatless/config.json`. The file has no `dockerAccess`/`envVars`,
/// so those are `None`.
fn run_config_from_moatless_config(
    config: &MoatlessRepoConfig,
) -> Result<RunConfig, serde_json::Error> {
    let setup_commands = config
        .code_execution
        .as_ref()
        .and_then(|c| c.setup_commands.as_ref())
        .map(serde_json::to_value)
        .transpose()?;
    Ok(RunConfig {
        docker_image: config.docker_image.clone(),
        docker_access: None,
        sandbox_service_account: config.sandbox_service_account.clone(),
        network_profile: config.network_profile.clone(),
        system_prompt: config.system_prompt.clone(),
        setup_commands,
        env_vars: None,
        servers: config
            .servers
            .as_ref()
            .map(serde_json::to_value)
            .transpose()?,
        resources: config
            .resources
            .as_ref()
            .map(serde_json::to_value)
            .transpose()?,
    })
}

/// What to do with an existing `source = 'manual'` row that a git-declared
/// workspace maps onto: `None` leaves the override/tombstone in place,
/// `Some(outcome)` writes git's config over it.
fn reclaim_override(
    workspace_id: &str,
    deleted: Option<bool>,
    force: &SyncForce,
) -> Option<UpsertOutcome> {
    if deleted == Some(true) {
        return matches!(force, SyncForce::Only(id) if id == workspace_id)
            .then_some(UpsertOutcome::Reclaimed);
    }
    force
        .reclaims(workspace_id)
        .then_some(UpsertOutcome::Reclaimed)
}

impl GitSyncService {
    /// Insert or update the workspace row + placements for one desired workspace.
    ///
    /// Returns [`UpsertOutcome::Skipped`] when the workspace has been manually
    /// overridden (`source = 'manual'`) and `force` does not reclaim it.
    async fn upsert_git_workspace(
        &self,
        repo: &Repository,
        desired: &DesiredWorkspace,
        force: &SyncForce,
    ) -> Result<UpsertOutcome, RepositoryError> {
        // reason: git sync writes workspaces derived from repository content, not
        // caller input; scope/owner are inherited from the repository row.
        #[allow(clippy::disallowed_methods)]
        let sys_ctx = ScopeContext::system();
        let (workspace_id, outcome) = if let Some(id) = &desired.fixed_id {
            // Shadow workspace — look up by id to check if it has been overridden.
            let existing: Option<(String, Option<bool>)> =
                sqlx::query_as("SELECT source, deleted FROM workspaces WHERE id = $1")
                    .bind(id)
                    .fetch_optional(&self.db)
                    .await?;
            match existing {
                Some((source, deleted)) if source == "manual" => {
                    let Some(outcome) = reclaim_override(id, deleted, force) else {
                        debug!(workspace_id = %id, "skipping git sync — workspace has been manually overridden");
                        return Ok(UpsertOutcome::Skipped);
                    };
                    (id.clone(), outcome)
                }
                Some(_) => (id.clone(), UpsertOutcome::Updated),
                None => {
                    // Doesn't exist yet — create it (idempotent, race-safe).
                    ensure_shadow_workspace(&self.db, &repo.id).await?;
                    (id.clone(), UpsertOutcome::Created)
                }
            }
        } else {
            // Look up by natural key — include 'manual' so an overridden workspace
            // is found and skipped (or reclaimed) rather than re-created under the
            // same key. Soft-deleted rows are reused too, so a delete never orphans
            // the key behind a row that no lookup would find again.
            let existing: Option<(String, String, Option<bool>)> = sqlx::query_as(
                "SELECT id, source, deleted FROM workspaces \
             WHERE source_repository_id = $1 AND source_key = $2 \
               AND source IN ('git', 'manual') \
             LIMIT 1",
            )
            .bind(&repo.id)
            .bind(&desired.source_key)
            .fetch_optional(&self.db)
            .await?;

            match existing {
                Some((id, source, deleted)) if source == "manual" => {
                    let Some(outcome) = reclaim_override(&id, deleted, force) else {
                        debug!(workspace_id = %id, source_key = %desired.source_key,
                        "skipping git sync — workspace has been manually overridden");
                        return Ok(UpsertOutcome::Skipped);
                    };
                    (id, outcome)
                }
                Some((id, _, _)) => (id, UpsertOutcome::Updated),
                None => {
                    let id = format!("ws_{}", uuid::Uuid::new_v4());
                    let row = Workspace {
                        id: id.clone(),
                        name: desired.name.clone(),
                        description: desired.description.clone(),
                        kind: "single".to_string(), // refresh_kind fixes below
                        docker_image: desired.run.docker_image.clone(),
                        docker_access: desired.run.docker_access,
                        sandbox_service_account: desired.run.sandbox_service_account.clone(),
                        system_prompt: desired.run.system_prompt.clone(),
                        setup_commands: desired.run.setup_commands.clone(),
                        env_vars: desired.run.env_vars.clone(),
                        servers: desired.run.servers.clone(),
                        resources: desired.run.resources.clone(),
                        network_profile: desired.run.network_profile.clone(),
                        source: "git".to_string(),
                        source_repository_id: Some(repo.id.clone()),
                        source_key: Some(desired.source_key.clone()),
                        source_config_path: Some(desired.source_config_path.clone()),
                        scope: repo.scope,
                        owner_user_id: repo.owner_user_id.clone(),
                        created_by: repo.created_by.clone(),
                        deleted: Some(false),
                        created_at: None,
                        updated_at: None,
                    };
                    self.workspaces.insert_workspace(&row).await?;
                    (id, UpsertOutcome::Created)
                }
            }
        };

        // Un-delete if a prior reconcile soft-deleted this workspace (key re-added).
        //
        // This puts the project back into every listing that dropped it, so it
        // records here — in its own transaction, guarded on the un-delete, and last.
        // The patch below would record too, but the two are separate transactions,
        // and a failure between them would leave the workspace visible again with
        // nothing recorded.
        let mut tx = self.db.begin().await?;
        let reclaimed =
            sqlx::query("UPDATE workspaces SET deleted = false WHERE id = $1 AND deleted = true")
                .bind(&workspace_id)
                .execute(&mut *tx)
                .await?;
        if reclaimed.rows_affected() > 0 {
            moatless_core::task_listing::record_workspace_change(&mut tx, &workspace_id).await?;
        }
        tx.commit().await?;

        // Apply run-config + provenance + name/description + scope, idempotently.
        let patch = WorkspacePatch {
            name: Some(desired.name.clone()),
            description: Some(desired.description.clone()),
            kind: None,
            scope: Some(repo.scope),
            owner_user_id: Some(repo.owner_user_id.clone()),
            docker_image: Some(desired.run.docker_image.clone()),
            docker_access: Some(desired.run.docker_access),
            sandbox_service_account: Some(desired.run.sandbox_service_account.clone()),
            system_prompt: Some(desired.run.system_prompt.clone()),
            setup_commands: Some(desired.run.setup_commands.clone()),
            env_vars: Some(desired.run.env_vars.clone()),
            servers: Some(desired.run.servers.clone()),
            resources: Some(desired.run.resources.clone()),
            network_profile: Some(desired.run.network_profile.clone()),
            source: Some("git".to_string()),
            source_repository_id: Some(Some(repo.id.clone())),
            source_key: Some(Some(desired.source_key.clone())),
            source_config_path: Some(Some(desired.source_config_path.clone())),
        };
        self.workspaces
            .update(&workspace_id, &patch, &sys_ctx)
            .await?;

        // Replace placements (delete-all + re-insert non-primary, then elect one
        // primary) so the partial unique index never sees two primaries mid-write.
        sqlx::query("DELETE FROM workspace_repos WHERE workspace_id = $1")
            .bind(&workspace_id)
            .execute(&self.db)
            .await?;

        let mut placement_ids: Vec<String> = Vec::with_capacity(desired.placements.len());
        let mut primary_choice: Option<String> = None;
        for (i, p) in desired.placements.iter().enumerate() {
            let placement_id = format!("wsr_{}", uuid::Uuid::new_v4());
            self.workspaces
                .add_repo(
                    &WorkspaceRepo {
                        id: placement_id.clone(),
                        workspace_id: workspace_id.clone(),
                        repository_id: p.repository_id.clone(),
                        branch: p.branch.clone(),
                        mount_name: p.mount_name.clone(),
                        is_primary: false,
                        position: i as i32,
                        created_at: None,
                    },
                    &sys_ctx,
                )
                .await?;
            if primary_choice.is_none() && p.primary {
                primary_choice = Some(placement_id.clone());
            }
            placement_ids.push(placement_id);
        }
        if let Some(target) = primary_choice.or_else(|| placement_ids.first().cloned()) {
            self.workspaces
                .set_primary(&workspace_id, &target, &sys_ctx)
                .await?;
        }
        self.workspaces
            .refresh_kind(&workspace_id, &sys_ctx)
            .await?;

        if matches!(outcome, UpsertOutcome::Reclaimed) {
            info!(
                %workspace_id, source_key = %desired.source_key,
                "Reclaimed manually overridden workspace from git; local edits replaced"
            );
        }
        Ok(outcome)
    }

    /// Resolve each `repos[].url` to a repository id, creating (and verifying) rows
    /// for URLs that don't resolve yet.
    async fn resolve_placements(
        &self,
        source_repo: &Repository,
        refs: &[MoatlessWorkspaceRepoRef],
        auth: &AuthUser,
    ) -> Result<Vec<ResolvedPlacement>, RepositoryError> {
        let mut out = Vec::with_capacity(refs.len());
        for r in refs {
            let repository_id = self
                .resolve_or_create_repo(source_repo, &r.url, auth)
                .await?;
            out.push(ResolvedPlacement {
                repository_id,
                branch: r.branch.clone(),
                mount_name: r.mount_name.clone(),
                primary: r.primary,
            });
        }
        Ok(out)
    }

    /// Find a repository by normalized URL (scoped to the source repo's tenant), or
    /// create one inheriting provider/scope/owner. Auto-created repos — and existing
    /// `unchecked` ones — are verified against the provider and stamped.
    async fn resolve_or_create_repo(
        &self,
        source_repo: &Repository,
        url: &str,
        auth: &AuthUser,
    ) -> Result<String, RepositoryError> {
        let provider = source_repo.provider.unwrap_or(RepositoryProvider::Github);
        let key = normalize_remote_url(url);
        let token = self
            .resolve_sync_token(source_repo, auth, provider, url)
            .await;

        if let Some(existing) = self
            .repositories
            .find_by_normalized_remote_url(
                &key,
                source_repo.scope,
                source_repo.owner_user_id.clone(),
            )
            .await?
        {
            if existing.access_status == "unchecked" {
                if let Some(existing_url) = existing.remote_url.clone() {
                    let status =
                        verify_repository_access(&existing_url, token.as_deref(), provider).await;
                    self.stamp_access(&existing.id, status, &existing_url).await;
                }
            }
            return Ok(existing.id);
        }

        // Create the row, then verify reachability and stamp — a missing or
        // inaccessible repo is still created + placed, just flagged.
        let status = verify_repository_access(url, token.as_deref(), provider).await;
        let now = Utc::now();
        let id = uuid::Uuid::new_v4().to_string();
        let row = Repository {
            id: id.clone(),
            name: repo_name_from_url(url),
            kind: RepositoryKind::Remote,
            remote_url: Some(url.to_string()),
            default_branch: None,
            description: None,
            provider: Some(provider),
            scope: source_repo.scope,
            owner_user_id: source_repo.owner_user_id.clone(),
            created_by: source_repo.created_by.clone(),
            access_status: status.as_str().to_string(),
            access_checked_at: Some(now),
            access_error: access_error_for(status, url),
            deleted: Some(false),
            created_at: Some(now),
            updated_at: Some(now),
        };
        if status == AccessStatus::Ok {
            info!(repository_id = %id, %url, "Auto-created repository for git-synced workspace");
        } else {
            warn!(repository_id = %id, %url, status = status.as_str(), "Auto-created repository is unreachable");
        }
        match self.repositories.insert(&row).await {
            Ok(()) => Ok(id),
            Err(error) if is_remote_url_conflict(&error) => self
                .repositories
                .find_by_normalized_remote_url(
                    &key,
                    source_repo.scope,
                    source_repo.owner_user_id.clone(),
                )
                .await?
                .map(|existing| existing.id)
                .ok_or_else(|| RepositoryError::Db(error)),
            Err(error) => Err(RepositoryError::Db(error)),
        }
    }

    /// Best-effort stamp of a repository's `access_status`.
    async fn stamp_access(&self, repository_id: &str, status: AccessStatus, url: &str) {
        let patch = RepositoryPatch {
            access_status: Some(status.as_str().to_string()),
            access_checked_at: Some(Some(Utc::now())),
            access_error: Some(access_error_for(status, url)),
            ..Default::default()
        };
        // reason: access-status stamping is sync bookkeeping on rows the reconcile
        // itself created or resolved.
        #[allow(clippy::disallowed_methods)]
        let sys_ctx = ScopeContext::system();
        if let Err(e) = self
            .repositories
            .update(repository_id, &patch, &sys_ctx)
            .await
        {
            warn!(%repository_id, error = %e, "Failed to stamp repository access_status");
        }
    }

    /// Resolve a GitHub/provider token for the sync, best-effort (unauthenticated
    /// works for public GitHub repos).
    async fn resolve_sync_token(
        &self,
        repository: &Repository,
        auth: &AuthUser,
        provider: RepositoryProvider,
        remote_url: &str,
    ) -> Option<String> {
        if provider == RepositoryProvider::Github {
            self.repo_token_resolver
                .resolve(repository, remote_url)
                .await
                .unwrap_or_else(|e| {
                    warn!(error = %e, "Workspace sync: failed to resolve GitHub token");
                    None
                })
        } else {
            match self
                .provider_tokens
                .find_provider_token(&auth.user_id.to_string(), provider.as_str())
                .await
            {
                Ok(token) => token.map(|t| t.access_token),
                Err(e) => {
                    warn!(error = %e, "Workspace sync: failed to load provider token");
                    None
                }
            }
        }
    }
}

/// Slugify a workspace name into a stable-ish source_key (used when no explicit
/// `key` is set): lowercase, non-alphanumerics collapsed to single `-`.
fn slugify(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut prev_dash = false;
    for ch in name.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    let slug = out.trim_matches('-').to_string();
    if slug.is_empty() {
        "workspace".to_string()
    } else {
        slug
    }
}

/// Last path segment of a repo URL (trailing `/` and `.git` stripped) as a name.
fn repo_name_from_url(url: &str) -> String {
    url.trim()
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("repository")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn force_only_reclaims_its_own_workspace() {
        assert!(!SyncForce::RespectOverrides.reclaims("ws_1"));
        assert!(SyncForce::All.reclaims("ws_1"));

        let only = SyncForce::Only("ws_1".to_string());
        assert!(only.reclaims("ws_1"));
        // A repo-wide sync triggered by one workspace's reset must not drag
        // every other overridden workspace back under git.
        assert!(!only.reclaims("ws_2"));
    }

    #[test]
    fn overrides_and_deleted_tombstones_block_automatic_sync() {
        // Live override: kept by the automatic sync, reclaimed by an explicit one.
        assert!(reclaim_override("ws_1", Some(false), &SyncForce::RespectOverrides).is_none());
        assert!(matches!(
            reclaim_override("ws_1", Some(false), &SyncForce::All),
            Some(UpsertOutcome::Reclaimed)
        ));

        // Deleted override: a tombstone blocks automatic and repo-wide sync, so
        // a git entry is not recreated unless this exact workspace is reset.
        assert!(reclaim_override("ws_1", Some(true), &SyncForce::RespectOverrides).is_none());
        assert!(reclaim_override("ws_1", Some(true), &SyncForce::All).is_none());

        let only_this = SyncForce::Only("ws_1".to_string());
        assert!(matches!(
            reclaim_override("ws_1", Some(true), &only_this),
            Some(UpsertOutcome::Reclaimed)
        ));

        let only_other = SyncForce::Only("ws_2".to_string());
        assert!(reclaim_override("ws_1", Some(true), &only_other).is_none());
    }

    #[test]
    fn slugify_produces_stable_keys() {
        assert_eq!(slugify("API Server"), "api-server");
        assert_eq!(slugify("  Web / Frontend  "), "web-frontend");
        assert_eq!(slugify("a__b--c"), "a-b-c");
        assert_eq!(slugify("!!!"), "workspace");
        assert_eq!(slugify(""), "workspace");
    }

    #[test]
    fn repo_name_from_url_takes_last_segment() {
        assert_eq!(repo_name_from_url("https://github.com/acme/api.git"), "api");
        assert_eq!(repo_name_from_url("https://github.com/acme/web/"), "web");
        assert_eq!(repo_name_from_url("https://gitlab.com/g/sub/proj"), "proj");
    }

    #[test]
    fn access_error_only_for_failures() {
        assert!(access_error_for(AccessStatus::Ok, "u").is_none());
        assert!(access_error_for(AccessStatus::Unchecked, "u").is_none());
        assert!(access_error_for(AccessStatus::NotFound, "u").is_some());
        assert!(access_error_for(AccessStatus::Forbidden, "u").is_some());
        assert!(access_error_for(AccessStatus::Error, "u").is_some());
    }

    #[test]
    fn parse_config_json() {
        let json = r#"{
            "codeExecution": {
                "setupCommands": ["bun install"]
            },
            "servers": [
                {
                    "name": "dev",
                    "label": "Dev Server",
                    "port": 3000,
                    "startCommands": ["bun run dev"],
                    "startupTimeoutSeconds": 30,
                    "autoReload": true,
                    "enableInspector": true,
                    "default": true
                }
            ]
        }"#;

        let config: MoatlessRepoConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.servers.as_ref().unwrap().len(), 1);
        assert_eq!(config.servers.as_ref().unwrap()[0].name, "dev");
        assert_eq!(
            config
                .code_execution
                .as_ref()
                .unwrap()
                .setup_commands
                .as_ref()
                .unwrap(),
            &vec!["bun install".to_string()]
        );
    }

    #[test]
    fn run_config_from_moatless_config_maps_fields() {
        let config = MoatlessRepoConfig {
            docker_image: Some("myregistry/custom-sandbox:latest".to_string()),
            code_execution: Some(CodeExecution {
                setup_commands: Some(vec!["npm install".to_string()]),
            }),
            servers: Some(vec![ServerConfig {
                name: "dev".to_string(),
                port: 3000,
                default: true,
                ..Default::default()
            }]),
            system_prompt: Some("Be helpful".to_string()),
            resources: None,
            sandbox_service_account: Some("moatless-dev".to_string()),
            network_profile: None,
        };

        let run = run_config_from_moatless_config(&config).unwrap();
        assert_eq!(
            run.docker_image,
            Some("myregistry/custom-sandbox:latest".to_string())
        );
        assert_eq!(
            run.sandbox_service_account,
            Some("moatless-dev".to_string())
        );
        assert_eq!(run.system_prompt, Some("Be helpful".to_string()));
        assert!(run.servers.is_some());
        assert!(run.setup_commands.is_some());
        // config.json has no dockerAccess/envVars.
        assert!(run.docker_access.is_none());
        assert!(run.env_vars.is_none());
    }

    #[test]
    fn config_carries_storage_mode_from_git_repo() {
        // A repo selecting the ephemeral storage tier via its committed
        // `.moatless/config.json`. The nested `ResourceConfig` must read the
        // camelCase keys and round-trip back to camelCase in the stored JSONB so
        // the provision-time deserialize reads it correctly.
        let json = r#"{
            "resources": { "storageMode": "ephemeral", "storage": "8Gi" }
        }"#;
        let config: MoatlessRepoConfig = serde_json::from_str(json).unwrap();
        let resources = config.resources.as_ref().expect("resources present");
        assert_eq!(resources.storage_mode.as_deref(), Some("ephemeral"));
        assert_eq!(resources.storage.as_deref(), Some("8Gi"));

        let run = run_config_from_moatless_config(&config).unwrap();
        let value = run.resources.expect("resources value present");
        assert_eq!(
            value.get("storageMode"),
            Some(&serde_json::json!("ephemeral"))
        );
        assert_eq!(value.get("storage"), Some(&serde_json::json!("8Gi")));
        assert!(
            value.get("storage_mode").is_none(),
            "snake_case key must not leak into stored JSONB"
        );
    }

    #[test]
    fn parse_workspaces_json() {
        let json = r#"{
            "version": 1,
            "workspaces": [
                {
                    "key": "api",
                    "name": "API",
                    "description": "backend",
                    "repos": [
                        { "url": "https://github.com/acme/api", "primary": true },
                        { "url": "https://github.com/acme/shared", "mountName": "shared" }
                    ],
                    "dockerImage": "acme/sandbox:latest",
                    "codeExecution": { "setupCommands": ["bun install"] },
                    "servers": [{ "name": "dev", "port": 3000, "default": true }],
                    "envVars": { "LOG_LEVEL": "debug" },
                    "networkProfile": "sealed"
                },
                { "name": "Web Only", "repos": [{ "url": "https://github.com/acme/web" }] }
            ]
        }"#;

        let file: MoatlessWorkspacesFile = serde_json::from_str(json).unwrap();
        assert_eq!(file.version, Some(1));
        assert_eq!(file.workspaces.len(), 2);

        let api = &file.workspaces[0];
        assert_eq!(api.key.as_deref(), Some("api"));
        assert_eq!(api.name, "API");
        assert_eq!(api.repos.len(), 2);
        assert!(api.repos[0].primary);
        assert_eq!(api.repos[0].url, "https://github.com/acme/api");
        assert_eq!(api.repos[1].mount_name.as_deref(), Some("shared"));
        assert!(!api.repos[1].primary);
        assert_eq!(api.docker_image.as_deref(), Some("acme/sandbox:latest"));
        assert_eq!(
            api.code_execution
                .as_ref()
                .unwrap()
                .setup_commands
                .as_ref()
                .unwrap(),
            &vec!["bun install".to_string()]
        );
        assert_eq!(api.network_profile.as_deref(), Some("sealed"));

        // Missing key → resolved later from the name; primary defaults to false.
        let web = &file.workspaces[1];
        assert!(web.key.is_none());
        assert_eq!(web.name, "Web Only");
        assert!(!web.repos[0].primary);
    }
}
