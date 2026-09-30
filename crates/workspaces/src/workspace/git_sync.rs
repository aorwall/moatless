//! Reconcile a repo's git-declared workspaces into `workspaces` rows.
//!
//! Rows from `.moatless/workspaces.json` are flagged `source = 'git'`, keyed by
//! `(source_repository_id, source_key)` so reconciliation never touches manual
//! workspaces, bypassing the write API's read-only guard via the DAO directly.
//!
//! `override` flips a workspace to `source = 'manual'` so auto-sync leaves it
//! alone; [`SyncForce`] decides which syncs reclaim an active override.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use chrono::Utc;
use serde::Deserialize;
use tracing::{debug, info, warn};

use crate::repository::config_reader::fetch_provider_file;
use crate::repository::dao::RepositoryDao;
use crate::repository::error::{RepositoryError, is_remote_url_conflict};
use crate::repository::models::Repository;
use crate::repository::remote::normalize_remote_url;
use crate::repository::remote::require_host_repo;
use crate::workspace::config_validation::validate_non_secret_env_vars;
use crate::workspace::dao::WorkspaceDao;
use crate::workspace::models::WorkspacePatch;
use crate::workspace::models::{Workspace, WorkspaceKind, WorkspaceRepo, WorkspaceSource};
use moatless_core::db::DbPool;
use moatless_core::http_auth::AuthUser;
use moatless_ports::identity::GitHostTokenIssuer;
use moatless_types::access_control::Actor;
use moatless_types::access_control::ScopeContext;
use moatless_types::http::HttpTimeouts;
use moatless_workspace_types::repository::RepositoryKind;
use moatless_workspace_types::repository::{GitHostRepo, RepositoryRemote};
use moatless_workspace_types::schemas::workspace::{
    WorkspaceSyncSkip, WorkspaceSyncSummary, WorkspaceSyncWarning,
};
use moatless_workspace_types::workspace::ResourceConfig;
use moatless_workspace_types::workspace::ScriptConfig;

// =============================================================================
// `.moatless/` file schemas
// =============================================================================

/// The keys of one object in `.moatless/workspaces.json` that no field of its
/// schema claims.
///
/// Serde routes what the named fields did not match into it, so a field added
/// above drops out of here with no second list to keep in step — which is what
/// lets [`unknown_key_warnings`] name a misspelled key rather than drop it.
type UnknownKeys = BTreeMap<String, serde_json::Value>;

/// Code execution settings.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeExecution {
    pub setup_commands: Option<Vec<String>>,
    #[serde(flatten)]
    pub unknown: UnknownKeys,
}

/// Top-level `.moatless/workspaces.json` schema — the default (non-legacy)
/// form, a list of workspaces the repo declares, each with its own run-config
/// and explicit repository URL(s).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoatlessWorkspacesFile {
    #[allow(dead_code)] // reserved for forward-compat; not yet branched on
    pub version: Option<u32>,
    pub workspaces: Vec<MoatlessWorkspaceEntry>,
    #[serde(flatten)]
    pub unknown: UnknownKeys,
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
    // --- Run configuration ---
    pub docker_image: Option<String>,
    pub docker_access: Option<bool>,
    pub code_execution: Option<CodeExecution>,
    pub scripts: Option<Vec<ScriptConfig>>,
    pub system_prompt: Option<String>,
    pub resources: Option<ResourceConfig>,
    pub env_vars: Option<std::collections::HashMap<String, String>>,
    pub sandbox_service_account: Option<String>,
    pub network_profile: Option<String>,
    #[serde(flatten)]
    pub unknown: UnknownKeys,
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
    #[serde(flatten)]
    pub unknown: UnknownKeys,
}

/// What the sync says about a key it read but did not apply.
const UNKNOWN_KEY: &str = "unknown key, ignored: nothing in the sandbox reads it";

/// The retired second git-declared form, kept only so a repo still carrying one
/// is told the file is dead rather than left reading it as live config.
const RETIRED_CONFIG_PATH: &str = ".moatless/config.json";

/// Whether a repo with no `.moatless/workspaces.json` still ships the retired
/// `.moatless/config.json`.
///
/// Probed only on that path, so an up-to-date repo pays nothing. A fetch
/// failure answers `None`: the warning exists to explain an empty sync, and
/// inventing one from an unreachable provider would explain the wrong thing.
async fn retired_config_warning(
    http: &reqwest::Client,
    remote: &RepositoryRemote,
    branch: Option<&str>,
    token: Option<&str>,
) -> Option<WorkspaceSyncWarning> {
    fetch_provider_file(http, remote, RETIRED_CONFIG_PATH, branch, token)
        .await
        .ok()
        .flatten()
        .map(|_| WorkspaceSyncWarning {
            path: RETIRED_CONFIG_PATH.to_string(),
            message: "not read: declare workspaces in .moatless/workspaces.json instead"
                .to_string(),
        })
}

/// Every key in the file that no field of the schema claims, as JSON paths into
/// the file.
///
/// A key serde does not recognise parses without error and is dropped, so a
/// misspelled or obsolete one leaves a repo whose committed config reads as
/// authoritative and is not.
fn unknown_key_warnings(file: &MoatlessWorkspacesFile) -> Vec<WorkspaceSyncWarning> {
    let mut warnings = Vec::new();
    let mut ignored = |path: String| {
        warnings.push(WorkspaceSyncWarning {
            path,
            message: UNKNOWN_KEY.to_string(),
        });
    };

    for key in commentless(&file.unknown) {
        ignored(key.clone());
    }
    for (i, entry) in file.workspaces.iter().enumerate() {
        for key in commentless(&entry.unknown) {
            ignored(format!("workspaces[{i}].{key}"));
        }
        if let Some(code_execution) = entry.code_execution.as_ref() {
            for key in commentless(&code_execution.unknown) {
                ignored(format!("workspaces[{i}].codeExecution.{key}"));
            }
        }
        for (j, repo) in entry.repos.iter().enumerate() {
            for key in commentless(&repo.unknown) {
                ignored(format!("workspaces[{i}].repos[{j}].{key}"));
            }
        }
    }
    warnings
}

/// The unclaimed keys that are not a comment.
///
/// JSON has no comment syntax, so an author annotating a config uses a `"//"`
/// key. Reporting those as mistakes would put a warning on every file that
/// explains itself.
fn commentless(unknown: &UnknownKeys) -> impl Iterator<Item = &String> {
    unknown.keys().filter(|key| !key.starts_with("//"))
}

/// Fetch and parse `.moatless/workspaces.json` from a remote repository.
///
/// Returns `Ok(None)` when the file does not exist.
pub async fn fetch_moatless_workspaces(
    http: &reqwest::Client,
    remote: &RepositoryRemote,
    branch: Option<&str>,
    token: Option<&str>,
) -> Result<Option<MoatlessWorkspacesFile>, RepositoryError> {
    let Some(body) =
        fetch_provider_file(http, remote, ".moatless/workspaces.json", branch, token).await?
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

/// Whether a sync reclaims workspaces that were manually overridden.
///
/// An override (`source = 'manual'`) exists to survive the automatic sync on
/// task start; only an explicit, user-initiated sync discards it.
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
/// reconcile tree threads through every frame (the workspace and repository
/// DAOs, the pool, and the git-host token seam).
pub struct GitSyncService {
    db: DbPool,
    workspaces: Arc<dyn WorkspaceDao>,
    repositories: Arc<dyn RepositoryDao>,
    git_host_tokens: Arc<dyn GitHostTokenIssuer>,
    http: reqwest::Client,
}

impl GitSyncService {
    pub(crate) fn new(
        db: DbPool,
        workspaces: Arc<dyn WorkspaceDao>,
        repositories: Arc<dyn RepositoryDao>,
        git_host_tokens: Arc<dyn GitHostTokenIssuer>,
        git_content: HttpTimeouts,
    ) -> Self {
        Self {
            db,
            workspaces,
            repositories,
            git_host_tokens,
            http: crate::repository::config_reader::client(git_content),
        }
    }

    /// Fetch, resolve, and reconcile the workspaces declared in a repository.
    ///
    /// A repo with no `.moatless/workspaces.json` reconciles and deletes
    /// nothing — an empty desired set would otherwise tombstone every workspace
    /// and Task in it. Best-effort/non-fatal: callers log the outcome.
    pub async fn sync_repo_workspaces_for(
        &self,
        repo: &Repository,
        scope: &ScopeContext,
        force: &SyncForce,
    ) -> Result<WorkspaceSyncSummary, RepositoryError> {
        let mut summary = WorkspaceSyncSummary::default();

        let Some(remote) = repo.remote.as_ref() else {
            return Ok(summary);
        };
        let host_repo = require_host_repo(remote)?;
        let token = self.resolve_sync_token(scope, host_repo).await;
        let token_str = token.as_deref();
        let branch = repo.default_branch.as_deref();

        let Some(file) = fetch_moatless_workspaces(&self.http, remote, branch, token_str).await?
        else {
            debug!(repository_id = %repo.id, "No .moatless/workspaces.json; nothing to sync");
            if let Some(warning) =
                retired_config_warning(&self.http, remote, branch, token_str).await
            {
                warn!(
                    repository_id = %repo.id,
                    path = %warning.path,
                    "Repository declares its config in a file the sync no longer reads"
                );
                summary.warnings.push(warning);
            }
            return Ok(summary);
        };

        // Reported before anything is applied, so a repo whose committed config
        // was parsed and partly dropped learns which keys went nowhere.
        summary.warnings = unknown_key_warnings(&file);
        for warning in &summary.warnings {
            warn!(
                repository_id = %repo.id,
                source_config_path = ".moatless/workspaces.json",
                path = %warning.path,
                "Ignoring unknown key in git-declared workspace config"
            );
        }

        // source_keys present in git this round — reconcile-delete keeps only these.
        let mut present: HashSet<String> = HashSet::new();
        let mut seen_keys: HashSet<String> = HashSet::new();
        for (i, entry) in file.workspaces.iter().enumerate() {
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

            let mut run = run_config_from_entry(entry);
            for field in
                strip_git_declared_identity(&mut run, &repo.id, ".moatless/workspaces.json")
            {
                summary.warnings.push(WorkspaceSyncWarning {
                    path: format!("workspaces[{i}].{field}"),
                    message: ADMIN_ONLY_IDENTITY.to_string(),
                });
            }
            let placements = self.resolve_placements(repo, &entry.repos).await?;

            present.insert(source_key.clone());
            let desired = DesiredWorkspace {
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

        // Reconcile-delete: soft-delete any git workspace for this repo whose
        // source_key is no longer declared. Soft so in-flight tasks pointing at it
        // keep working — provision lookups don't filter `deleted`.
        let existing = super::dao::git_source_keys(&self.db, &repo.id).await?;
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
    ///
    /// Gated on the repo shipping a `.moatless/` directory. Every failure is
    /// logged and swallowed — this must never block or fail a task start. Loops
    /// are never synchronized as a side effect of task start.
    pub async fn sync_repo_config_on_start_for(&self, scope: &ScopeContext, repo: &Repository) {
        let Some(remote) = repo.remote.as_ref() else {
            return;
        };
        let Some(host_repo) = remote.host_repo.as_ref() else {
            return;
        };

        // Resolve a token for the existence probe; fall back to an unauthenticated
        // probe (works for public repos) if we can't load one.
        let token = self.resolve_sync_token(scope, host_repo).await;

        let token_str = token.as_deref();
        let branch = repo.default_branch.as_deref();

        if !crate::repository::config_reader::moatless_dir_exists(
            &self.http, remote, branch, token_str,
        )
        .await
        {
            debug!(repository_id = %repo.id, "Config auto-sync: no .moatless directory, skipping");
            return;
        }

        // Automatic sync: workspaces the user overrode from the UI/CLI stay
        // overridden — only an explicit sync-config or reset-git reclaims them.
        let force = SyncForce::RespectOverrides;
        match self.sync_repo_workspaces_for(repo, scope, &force).await {
            Ok(workspaces) => {
                info!(
                    repository_id = %repo.id,
                    workspaces_created = workspaces.created,
                    workspaces_updated = workspaces.updated,
                    workspaces_overridden = workspaces.skipped.len(),
                    workspaces_warnings = workspaces.warnings.len(),
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

/// What the sync says about an identity field it dropped.
const ADMIN_ONLY_IDENTITY: &str = "ignored: a privileged sandbox identity is admin-only";

/// Drop a privileged sandbox identity from repo config, naming what it took.
///
/// Honouring `sandbox_service_account`/`network_profile` from a file anyone
/// with commit access can edit would escalate them to admin-gated Secrets.
/// Dropped rather than rejected, so one stray field cannot fail the whole sync.
fn strip_git_declared_identity(
    run: &mut RunConfig,
    repository_id: &str,
    source_config_path: &str,
) -> Vec<&'static str> {
    let mut stripped = Vec::new();
    if let Some(account) = run.sandbox_service_account.take() {
        warn!(
            %repository_id,
            source_config_path,
            declared = %account,
            "Ignoring repo-declared sandboxServiceAccount: a privileged sandbox identity is admin-only"
        );
        stripped.push("sandboxServiceAccount");
    }
    if let Some(profile) = run.network_profile.take() {
        warn!(
            %repository_id,
            source_config_path,
            declared = %profile,
            "Ignoring repo-declared networkProfile: a privileged sandbox identity is admin-only"
        );
        stripped.push("networkProfile");
    }
    stripped
}

/// The run-config fields a git-declared workspace carries.
struct RunConfig {
    docker_image: Option<String>,
    docker_access: Option<bool>,
    sandbox_service_account: Option<String>,
    network_profile: Option<String>,
    system_prompt: Option<String>,
    setup_commands: Option<Vec<String>>,
    env_vars: Option<HashMap<String, String>>,
    scripts: Option<Vec<ScriptConfig>>,
    resources: Option<ResourceConfig>,
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
    source_key: String,
    /// Path to the declaring file within the repository.
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

fn run_config_from_entry(entry: &MoatlessWorkspaceEntry) -> RunConfig {
    RunConfig {
        docker_image: entry.docker_image.clone(),
        docker_access: entry.docker_access,
        sandbox_service_account: entry.sandbox_service_account.clone(),
        network_profile: entry.network_profile.clone(),
        system_prompt: entry.system_prompt.clone(),
        setup_commands: entry
            .code_execution
            .as_ref()
            .and_then(|c| c.setup_commands.clone()),
        env_vars: entry.env_vars.clone(),
        scripts: entry.scripts.clone(),
        resources: entry.resources.clone(),
    }
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
    /// Insert or update the workspace row + placements for one desired
    /// workspace.
    ///
    /// Returns [`UpsertOutcome::Skipped`] when the workspace is manually
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
        // Look up by natural key — include 'manual' so an overridden workspace
        // is found and skipped (or reclaimed) rather than re-created under the
        // same key. Soft-deleted rows are reused too, so a delete never orphans
        // the key behind a row that no lookup would find again.
        let existing =
            super::dao::workspace_for_git_source(&self.db, &repo.id, &desired.source_key).await?;

        let (workspace_id, outcome) = match existing {
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
                    // Presentation and Task defaults are not declared in git.
                    icon: None,
                    default_model: None,
                    default_effort: None,
                    kind: WorkspaceKind::Single, // refresh_kind fixes below
                    docker_image: desired.run.docker_image.clone(),
                    docker_access: desired.run.docker_access,
                    sandbox_service_account: desired.run.sandbox_service_account.clone(),
                    system_prompt: desired.run.system_prompt.clone(),
                    setup_commands: desired.run.setup_commands.clone(),
                    env_vars: desired.run.env_vars.clone(),
                    scripts: desired.run.scripts.clone(),
                    resources: desired.run.resources.clone(),
                    network_profile: desired.run.network_profile.clone(),
                    source: WorkspaceSource::Git,
                    source_repository_id: Some(repo.id.clone()),
                    source_key: Some(desired.source_key.clone()),
                    source_config_path: Some(desired.source_config_path.clone()),
                    scope: repo.scope,
                    owner_user_id: repo.owner_user_id.clone(),
                    // Never declared in git: commit access to a repository
                    // would otherwise name the account its Tasks act as, or
                    // move its sandboxes onto dedicated capacity.
                    run_as_user_id: None,
                    node_selector: None,
                    created_by: repo.created_by.clone(),
                    deleted: false,
                    created_at: None,
                    updated_at: None,
                };
                self.workspaces.insert_workspace(&row).await?;
                (id, UpsertOutcome::Created)
            }
        };

        // Un-delete if a prior reconcile soft-deleted this workspace (key re-added).
        super::dao::restore_git_workspace(&self.db, &workspace_id).await?;

        // Apply run-config + provenance + name/description + scope, idempotently.
        let patch = WorkspacePatch {
            name: Some(desired.name.clone()),
            description: Some(desired.description.clone()),
            kind: None,
            scope: Some(repo.scope),
            owner_user_id: Some(repo.owner_user_id.clone()),
            // Absent from the patch, so a reconcile leaves whatever an admin set.
            run_as_user_id: None,
            node_selector: None,
            // Likewise: never declared in git, so a reconcile must not clear
            // what a user picked in the settings UI.
            icon: None,
            default_model: None,
            default_effort: None,
            docker_image: Some(desired.run.docker_image.clone()),
            docker_access: Some(desired.run.docker_access),
            sandbox_service_account: Some(desired.run.sandbox_service_account.clone()),
            system_prompt: Some(desired.run.system_prompt.clone()),
            setup_commands: Some(desired.run.setup_commands.clone()),
            env_vars: Some(desired.run.env_vars.clone()),
            scripts: Some(desired.run.scripts.clone()),
            resources: Some(desired.run.resources.clone()),
            network_profile: Some(desired.run.network_profile.clone()),
            source: Some(WorkspaceSource::Git),
            source_repository_id: Some(Some(repo.id.clone())),
            source_key: Some(Some(desired.source_key.clone())),
            source_config_path: Some(Some(desired.source_config_path.clone())),
        };
        self.workspaces
            .update(&workspace_id, &patch, &sys_ctx)
            .await?;

        let mut placements = Vec::with_capacity(desired.placements.len());
        let mut primary_choice = None;
        for (i, p) in desired.placements.iter().enumerate() {
            let id = format!("wsr_{}", uuid::Uuid::new_v4());
            if primary_choice.is_none() && p.primary {
                primary_choice = Some(id.clone());
            }
            placements.push(WorkspaceRepo {
                id,
                workspace_id: workspace_id.clone(),
                repository_id: p.repository_id.clone(),
                branch: p.branch.clone(),
                mount_name: p.mount_name.clone(),
                is_primary: false,
                position: i as i32,
                created_at: None,
            });
        }
        let primary = primary_choice.or_else(|| placements.first().map(|p| p.id.clone()));
        super::dao::replace_git_placements(
            &self.db,
            &workspace_id,
            &placements,
            primary.as_deref(),
        )
        .await?;

        if matches!(outcome, UpsertOutcome::Reclaimed) {
            info!(
                %workspace_id, source_key = %desired.source_key,
                "Reclaimed manually overridden workspace from git; local edits replaced"
            );
        }
        Ok(outcome)
    }

    /// Resolve each `repos[].url` to a repository id, creating rows for URLs
    /// that don't resolve yet.
    async fn resolve_placements(
        &self,
        source_repo: &Repository,
        refs: &[MoatlessWorkspaceRepoRef],
    ) -> Result<Vec<ResolvedPlacement>, RepositoryError> {
        let mut out = Vec::with_capacity(refs.len());
        for r in refs {
            let repository_id = self.resolve_or_create_repo(source_repo, &r.url).await?;
            out.push(ResolvedPlacement {
                repository_id,
                branch: r.branch.clone(),
                mount_name: r.mount_name.clone(),
                primary: r.primary,
            });
        }
        Ok(out)
    }

    /// Find a repository by normalized URL (scoped to the source repo's
    /// tenant), or create one inheriting provider/scope/owner.
    async fn resolve_or_create_repo(
        &self,
        source_repo: &Repository,
        url: &str,
    ) -> Result<String, RepositoryError> {
        let provider = source_repo.provider.ok_or_else(|| {
            RepositoryError::BadRequest("Source repository has no declared git host".to_string())
        })?;
        let key = normalize_remote_url(url);

        if let Some(existing) = self
            .repositories
            .find_by_normalized_remote_url(
                &key,
                source_repo.scope,
                source_repo.owner_user_id.clone(),
            )
            .await?
        {
            return Ok(existing.id);
        }

        let now = Utc::now();
        let id = uuid::Uuid::new_v4().to_string();
        let row = Repository {
            id: id.clone(),
            name: repo_name_from_url(url),
            kind: RepositoryKind::Remote,
            remote: Some(url.to_string())
                .map(|url| crate::repository::remote::resolve_remote(Some(provider), url)),
            default_branch: None,
            description: None,
            provider: Some(provider),
            scope: source_repo.scope,
            owner_user_id: source_repo.owner_user_id.clone(),
            created_by: source_repo.created_by.clone(),
            deleted: false,
            created_at: Some(now),
            updated_at: Some(now),
        };
        info!(repository_id = %id, %url, "Auto-created repository for git-synced workspace");
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

    /// The caller's own token for the sync, best-effort — an unauthenticated
    /// probe still works for a public repository.
    ///
    /// One rule on every host: the person who asked for the sync is the person
    /// it runs as.
    async fn resolve_sync_token(&self, scope: &ScopeContext, repo: &GitHostRepo) -> Option<String> {
        match self
            .git_host_tokens
            .token(&Actor::caller(scope), &repo.credential_account())
            .await
        {
            Ok(token) => Some(token),
            Err(e) => {
                warn!(error = %e, "Workspace sync: probing unauthenticated");
                None
            }
        }
    }
}

/// Slugify a workspace name into a stable-ish `source_key` (used when no
/// explicit `key` is set): lowercase, non-alphanumerics collapsed to single
/// `-`.
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

/// Last path segment of a repo URL (trailing `/` and `.git` stripped) as a
/// name.
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

impl GitSyncService {
    /// Synchronize using an HTTP caller's scope.
    pub async fn sync_repo_workspaces(
        &self,
        repo: &Repository,
        auth: &AuthUser,
        force: &SyncForce,
    ) -> Result<WorkspaceSyncSummary, RepositoryError> {
        self.sync_repo_workspaces_for(repo, &auth.scope_context(), force)
            .await
    }

    /// Synchronize before starting a task using an HTTP caller's scope.
    pub async fn sync_repo_config_on_start(&self, auth: &AuthUser, repo: &Repository) {
        self.sync_repo_config_on_start_for(&auth.scope_context(), repo)
            .await;
    }
}

pub async fn log_workspace_sync_for(
    git_sync: &GitSyncService,
    repo: &Repository,
    scope: &ScopeContext,
    phase: &str,
    force: &SyncForce,
) -> WorkspaceSyncSummary {
    match git_sync.sync_repo_workspaces_for(repo, scope, force).await {
        Ok(summary) => {
            if summary.created + summary.updated + summary.reclaimed + summary.removed > 0
                || !summary.skipped.is_empty()
            {
                info!(
                    repository_id = %repo.id,
                    created = summary.created,
                    updated = summary.updated,
                    reclaimed = summary.reclaimed,
                    removed = summary.removed,
                    skipped = summary.skipped.len(),
                    phase,
                    "Synced git workspaces"
                );
            }
            summary
        }
        Err(e) => {
            warn!(repository_id = %repo.id, error = %e, phase, "Failed to sync git workspaces");
            WorkspaceSyncSummary::default()
        }
    }
}
/// Synchronize with an HTTP caller's scope and log the outcome.
pub async fn log_workspace_sync(
    git_sync: &Arc<GitSyncService>,
    repo: &Repository,
    auth: &AuthUser,
    phase: &str,
    force: &SyncForce,
) -> WorkspaceSyncSummary {
    log_workspace_sync_for(git_sync, repo, &auth.scope_context(), phase, force).await
}

#[cfg(test)]
mod tests {
    use moatless_workspace_types::workspace::Memory;

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

    /// One entry of a `.moatless/workspaces.json`, parsed.
    fn parse_entry(json: &str) -> MoatlessWorkspaceEntry {
        serde_json::from_str(json).expect("parse workspace entry")
    }

    /// Repo config may not hand a sandbox an elevated Kubernetes identity —
    /// otherwise anyone with commit access could grant themselves the same
    /// access the admin-only API gate withholds.
    #[test]
    fn git_declared_privileged_identity_is_stripped() {
        let entry = parse_entry(
            r#"{
                "name": "API",
                "repos": [],
                "dockerImage": "myregistry/custom-sandbox:latest",
                "systemPrompt": "Be helpful",
                "sandboxServiceAccount": "moatless-admin",
                "networkProfile": "moatless-admin"
            }"#,
        );

        let mut run = run_config_from_entry(&entry);
        let stripped = strip_git_declared_identity(&mut run, "repo-1", ".moatless/workspaces.json");

        assert_eq!(stripped, ["sandboxServiceAccount", "networkProfile"]);
        assert!(run.sandbox_service_account.is_none());
        assert!(run.network_profile.is_none());
        // Everything else the repo declares still syncs.
        assert_eq!(
            run.docker_image,
            Some("myregistry/custom-sandbox:latest".to_string())
        );
        assert_eq!(run.system_prompt, Some("Be helpful".to_string()));
    }

    /// Stripping is unconditional, so a repo declaring neither is unaffected
    /// and reports nothing.
    #[test]
    fn strip_is_a_noop_without_declared_identity() {
        let entry = parse_entry(r#"{ "name": "API", "repos": [] }"#);

        let mut run = run_config_from_entry(&entry);
        let stripped = strip_git_declared_identity(&mut run, "repo-1", ".moatless/workspaces.json");

        assert!(stripped.is_empty());
        assert!(run.sandbox_service_account.is_none());
        assert!(run.network_profile.is_none());
    }

    #[test]
    fn config_carries_storage_switches_from_git_repo() {
        // A repo turning the PVC off and keeping the S3 snapshot. The nested
        // `ResourceConfig` must read the camelCase keys and round-trip back to
        // camelCase in the stored JSONB so the provision-time deserialize reads
        // it correctly.
        let entry = parse_entry(
            r#"{
                "name": "API",
                "repos": [],
                "resources": { "pvcEnabled": false, "s3Enabled": true, "storage": "8Gi" }
            }"#,
        );
        let resources = entry.resources.as_ref().expect("resources present");
        assert_eq!(resources.pvc_enabled, Some(false));
        assert_eq!(resources.s3_enabled, Some(true));
        assert_eq!(resources.storage.as_ref().map(Memory::as_str), Some("8Gi"));

        let run = run_config_from_entry(&entry);
        let stored = run.resources.expect("resources value present");
        let value = serde_json::to_value(&stored).expect("serialize");
        assert_eq!(value.get("pvcEnabled"), Some(&serde_json::json!(false)));
        assert_eq!(value.get("s3Enabled"), Some(&serde_json::json!(true)));
        assert_eq!(value.get("storage"), Some(&serde_json::json!("8Gi")));
        assert!(
            value.get("pvc_enabled").is_none(),
            "snake_case key must not leak into stored JSONB"
        );
    }

    /// The removed `storageMode` is an unknown field, not an error: a repo that
    /// still carries one loads and simply falls back to the deployment
    /// defaults.
    #[test]
    fn a_stale_storage_mode_is_ignored() {
        let entry = parse_entry(
            r#"{
                "name": "API",
                "repos": [],
                "resources": { "storageMode": "ephemeral", "storage": "8Gi" }
            }"#,
        );
        let resources = entry.resources.as_ref().expect("resources present");
        assert_eq!(resources.pvc_enabled, None);
        assert_eq!(resources.s3_enabled, None);
        assert_eq!(resources.storage.as_ref().map(Memory::as_str), Some("8Gi"));
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
                    "scripts": [{ "name": "dev", "command": "bun dev", "port": 3000 }],
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
        assert!(unknown_key_warnings(&file).is_empty());
    }

    /// The shape that cost two agents a session: `previewConfig` and
    /// `codeExecution.envVars` parse without error, nothing reads them, and the
    /// committed file reads as authoritative. Each one has to be named.
    #[test]
    fn unknown_keys_are_named_with_the_path_they_sit_at() {
        let json = r#"{
            "version": 1,
            "previewConfig": { "serverStartCommands": ["bun run dev --turbo"] },
            "workspaces": [
                {
                    "name": "Frontend",
                    "repos": [{ "url": "https://github.com/acme/web", "revision": "main" }],
                    "previewConfig": { "serverStartCommands": ["bun run dev"] },
                    "codeExecution": { "envVars": { "VITE_BACKEND_URL": "https://api" } }
                }
            ]
        }"#;
        let file: MoatlessWorkspacesFile = serde_json::from_str(json).expect("parse");

        let warnings = unknown_key_warnings(&file);
        let paths: Vec<String> = warnings
            .iter()
            .map(|warning| warning.path.clone())
            .collect();
        assert_eq!(
            paths,
            [
                "previewConfig",
                "workspaces[0].previewConfig",
                "workspaces[0].codeExecution.envVars",
                "workspaces[0].repos[0].revision",
            ]
        );
        assert!(warnings.iter().all(|w| w.message == UNKNOWN_KEY));
    }

    /// A key the schema does claim must never be reported: a warning on a
    /// working config is the same misdirection as silence on a broken one. A
    /// `"//"` comment is not a mistake either — `workspaces.shadow.json` is
    /// written that way.
    #[test]
    fn a_recognised_key_or_a_comment_is_not_reported_as_unknown() {
        let json = r#"{
            "//": "the shadow workspace",
            "workspaces": [{
                "//1": "runs two scripts",
                "name": "Frontend",
                "repos": [{ "url": "https://github.com/acme/web", "mountName": "web" }],
                "codeExecution": { "setupCommands": ["bun install"] },
                "envVars": { "LOG_LEVEL": "debug" }
            }]
        }"#;
        let file: MoatlessWorkspacesFile = serde_json::from_str(json).expect("parse");

        assert!(unknown_key_warnings(&file).is_empty());
    }
}

#[cfg(all(test, feature = "db-tests"))]
#[path = "git_sync_db_tests.rs"]
mod git_sync_db_tests;
