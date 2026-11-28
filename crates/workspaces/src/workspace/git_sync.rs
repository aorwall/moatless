//! Reconcile a repo's git-declared workspaces into `workspaces` rows.
//!
//! Git is the only writer of a Workspace. Every repository has a `default`
//! Workspace placing it alone, which a `.moatless/workspaces.json` entry keyed
//! `default` overrides; the file's other entries add Workspaces. Rows are
//! flagged `source = 'git'` and keyed by `(source_repository_id, source_key)`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use chrono::Utc;
use serde::Deserialize;
use tracing::{debug, info, warn};

use crate::plugin::dao::{PluginDao, replace_workspace_activations};
use crate::plugin::models::{DeclaredActivation, Plugin, PluginSkill};
use crate::repository::config_reader::fetch_provider_file;
use crate::repository::dao::RepositoryDao;
use crate::repository::error::{RepositoryError, is_remote_url_conflict};
use crate::repository::models::Repository;
use crate::repository::remote::require_host_repo;
use crate::repository::remote::{ensure_remote_url_scheme_allowed, normalize_remote_url};
use crate::workspace::config_validation::validate_non_secret_env_vars;
use crate::workspace::dao::WorkspaceDao;
use crate::workspace::git_skills::{SkillActivations, resolve_declared};
use crate::workspace::models::WorkspacePatch;
use crate::workspace::models::{Workspace, WorkspaceKind, WorkspaceRepo, WorkspaceSource};
use moatless_core::db::DbPool;
use moatless_core::workspace_identity_grants::{WorkspaceIdentityGrant, WorkspaceIdentityGrants};
use moatless_ports::identity::GitHostTokenIssuer;
use moatless_types::access_control::Actor;
use moatless_types::access_control::{Scope, ScopeContext};
use moatless_types::http::HttpTimeouts;
use moatless_workspace_types::repository::RepositoryKind;
use moatless_workspace_types::repository::{GitHostRepo, RepositoryProvider, RepositoryRemote};
use moatless_workspace_types::schemas::workspace::{
    WorkspaceSyncSkip, WorkspaceSyncSummary, WorkspaceSyncWarning,
};
use moatless_workspace_types::workspace::LanguageServer;
use moatless_workspace_types::workspace::ResourceConfig;
use moatless_workspace_types::workspace::ScriptConfig;
use moatless_workspace_types::workspace::WorkspaceIcon;

// =============================================================================
// `.moatless/` file schemas
// =============================================================================

/// The keys of one object in `.moatless/workspaces.json` that no field of its
/// schema claims.
///
/// Serde routes what the named fields did not match into it, so a field added
/// above drops out of here with no second list to keep in step — which is what
/// lets [`unknown_key_warnings`] name a misspelled key rather than drop it.
pub(crate) type UnknownKeys = BTreeMap<String, serde_json::Value>;

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
    /// Read by nothing yet; declared so it stays out of `unknown` and draws no
    /// unknown-key warning.
    pub version: Option<u32>,
    pub workspaces: Vec<MoatlessWorkspaceEntry>,
    #[serde(flatten)]
    pub unknown: UnknownKeys,
}

/// One workspace entry in `.moatless/workspaces.json`.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoatlessWorkspaceEntry {
    /// Stable natural key within the repo. When present, renaming `name` keeps
    /// the workspace identity; when absent, the slugified `name` is the key.
    /// [`DEFAULT_WORKSPACE_KEY`] overrides the repository's own Workspace.
    pub key: Option<String>,
    /// Required unless `key` is [`DEFAULT_WORKSPACE_KEY`], which defaults it to
    /// the repository's name.
    pub name: Option<String>,
    pub description: Option<String>,
    /// A value rather than [`WorkspaceIcon`]: a malformed icon becomes a
    /// warning instead of failing the whole file.
    pub icon: Option<serde_json::Value>,
    /// Empty places the declaring repository alone, unless
    /// [`Self::place_repository`] is `false`.
    #[serde(default)]
    pub repos: Vec<MoatlessWorkspaceRepoRef>,
    /// `false` places no repository, so its Tasks start with nothing cloned.
    /// Honoured only by [`places_no_repository`].
    pub place_repository: Option<bool>,
    // --- Task defaults ---
    pub default_model: Option<String>,
    pub default_effort: Option<String>,
    /// Key of the Linear team T3's Issues page lists (`SP`).
    pub linear_team: Option<String>,
    // --- Run configuration ---
    pub docker_image: Option<String>,
    pub docker_access: Option<bool>,
    pub code_execution: Option<CodeExecution>,
    pub scripts: Option<Vec<ScriptConfig>>,
    pub system_prompt: Option<String>,
    pub resources: Option<ResourceConfig>,
    pub env_vars: Option<std::collections::HashMap<String, String>>,
    /// Ids rather than [`LanguageServer`]: an id this backend does not know
    /// becomes a warning instead of failing the whole file.
    pub language_servers: Option<Vec<String>>,
    /// This and the next two are honoured only as a
    /// [`WorkspaceIdentityGrant`] names them; see [`apply_identity_grant`].
    pub sandbox_service_account: Option<String>,
    pub network_profile: Option<String>,
    pub node_selector: Option<BTreeMap<String, String>>,
    /// See [`crate::workspace::git_skills`].
    pub skills: Option<SkillActivations>,
    /// The service proxies the Workspace opts into. Only an opt-in: a service
    /// whose availability is `granted` also needs a grant, read per request.
    pub services: Option<Vec<String>>,
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

/// The `source_key` of the Workspace every repository has.
pub const DEFAULT_WORKSPACE_KEY: &str = "default";

const WORKSPACES_PATH: &str = ".moatless/workspaces.json";

/// What the sync says about a `placeRepository: false` it did not honour.
const KEPT_PLACEMENT: &str =
    "ignored: only an entry other than `default`, with no `repos`, can place no repository";

/// Whether `entry` places no repository at all: `placeRepository: false` on an
/// entry with no `repos` that is not the repository's `default` Workspace.
fn places_no_repository(entry: &MoatlessWorkspaceEntry, is_default: bool) -> bool {
    entry.place_repository == Some(false) && entry.repos.is_empty() && !is_default
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
    github_api_base: &str,
    remote: &RepositoryRemote,
    branch: Option<&str>,
    token: Option<&str>,
) -> Option<WorkspaceSyncWarning> {
    fetch_provider_file(
        http,
        github_api_base,
        remote,
        RETIRED_CONFIG_PATH,
        branch,
        token,
    )
    .await
    .ok()
    .flatten()
    .map(|_| WorkspaceSyncWarning {
        path: RETIRED_CONFIG_PATH.to_string(),
        message: "not read: declare workspaces in .moatless/workspaces.json instead".to_string(),
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
        if let Some(skills) = entry.skills.as_ref() {
            for key in commentless(&skills.unknown) {
                ignored(format!("workspaces[{i}].skills.{key}"));
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

/// What the sync says about a language server id it does not know.
const UNKNOWN_LANGUAGE_SERVER: &str = "unknown language server, ignored";

/// Every `languageServers` entry that names no [`LanguageServer`], as JSON
/// paths into the file.
fn unknown_language_server_warnings(file: &MoatlessWorkspacesFile) -> Vec<WorkspaceSyncWarning> {
    let mut warnings = Vec::new();
    for (i, entry) in file.workspaces.iter().enumerate() {
        for (j, id) in entry.language_servers.iter().flatten().enumerate() {
            if id.parse::<LanguageServer>().is_err() {
                warnings.push(WorkspaceSyncWarning {
                    path: format!("workspaces[{i}].languageServers[{j}]"),
                    message: UNKNOWN_LANGUAGE_SERVER.to_string(),
                });
            }
        }
    }
    warnings
}

/// What the sync says about an `icon` that is no [`WorkspaceIcon`].
const INVALID_ICON: &str = "not a lucide or emoji icon, ignored";

/// Every `icon` that does not parse as a [`WorkspaceIcon`], as JSON paths into
/// the file.
fn invalid_icon_warnings(file: &MoatlessWorkspacesFile) -> Vec<WorkspaceSyncWarning> {
    file.workspaces
        .iter()
        .enumerate()
        .filter(|(_, entry)| entry.icon.is_some() && parse_icon(entry).is_none())
        .map(|(i, _)| WorkspaceSyncWarning {
            path: format!("workspaces[{i}].icon"),
            message: INVALID_ICON.to_string(),
        })
        .collect()
}

fn parse_icon(entry: &MoatlessWorkspaceEntry) -> Option<WorkspaceIcon> {
    entry
        .icon
        .clone()
        .and_then(|icon| serde_json::from_value(icon).ok())
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
    github_api_base: &str,
    remote: &RepositoryRemote,
    branch: Option<&str>,
    token: Option<&str>,
) -> Result<Option<MoatlessWorkspacesFile>, RepositoryError> {
    let Some(body) = fetch_provider_file(
        http,
        github_api_base,
        remote,
        WORKSPACES_PATH,
        branch,
        token,
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

/// What happened to one git-declared workspace during a sync.
enum UpsertOutcome {
    Created,
    Updated,
    /// A legacy manual override taken back under git control.
    Reclaimed,
}

/// What [`GitSyncService::default_branch_head`] read.
pub struct BranchHead {
    pub sha: String,
    /// Whether a credential was presented. An unauthenticated read spends
    /// GitHub's shared per-IP budget of 60 requests an hour.
    pub authenticated: bool,
}

/// Reconciles a repository's git-declared workspaces, holding the ports the
/// reconcile tree threads through every frame (the workspace and repository
/// DAOs, the pool, and the git-host token seam).
pub struct GitSyncService {
    db: DbPool,
    workspaces: Arc<dyn WorkspaceDao>,
    repositories: Arc<dyn RepositoryDao>,
    /// Read for the registry a `skills` key resolves against.
    plugins: Arc<dyn PluginDao>,
    git_host_tokens: Arc<dyn GitHostTokenIssuer>,
    http: reqwest::Client,
    /// `Config::github_api_base`.
    github_api_base: String,
    identity_grants: WorkspaceIdentityGrants,
}

impl GitSyncService {
    #[expect(
        clippy::too_many_arguments,
        reason = "`compose::git_sync`, the one caller, names each store and setting"
    )]
    pub(crate) fn new(
        db: DbPool,
        workspaces: Arc<dyn WorkspaceDao>,
        repositories: Arc<dyn RepositoryDao>,
        plugins: Arc<dyn PluginDao>,
        git_host_tokens: Arc<dyn GitHostTokenIssuer>,
        git_content: HttpTimeouts,
        github_api_base: String,
        identity_grants: WorkspaceIdentityGrants,
    ) -> Self {
        Self {
            db,
            workspaces,
            repositories,
            plugins,
            git_host_tokens,
            http: crate::repository::config_reader::client(git_content),
            github_api_base,
            identity_grants,
        }
    }

    /// Fetch, resolve, and reconcile the workspaces declared in a repository.
    ///
    /// A repo with no `.moatless/workspaces.json` syncs its default Workspace
    /// and deletes nothing: a provider answering 404 for a file it would not
    /// let us read would otherwise tombstone every other Workspace and Task in
    /// it. Best-effort/non-fatal: callers log the outcome.
    pub async fn sync_repo_workspaces_for(
        &self,
        repo: &Repository,
        scope: &ScopeContext,
    ) -> Result<WorkspaceSyncSummary, RepositoryError> {
        let mut summary = WorkspaceSyncSummary::default();

        let Some(remote) = repo.remote.as_ref() else {
            return Ok(summary);
        };
        let host_repo = require_host_repo(remote)?;
        let token = self.resolve_sync_token(scope, host_repo).await;
        let token_str = token.as_deref();
        let branch = repo.default_branch.as_deref();
        let full_name = format!("{}/{}", host_repo.owner(), host_repo.name());

        let Some(file) =
            fetch_moatless_workspaces(&self.http, &self.github_api_base, remote, branch, token_str)
                .await?
        else {
            debug!(repository_id = %repo.id, "No .moatless/workspaces.json; syncing the default workspace");
            if let Some(warning) =
                retired_config_warning(&self.http, &self.github_api_base, remote, branch, token_str)
                    .await
            {
                warn!(
                    repository_id = %repo.id,
                    path = %warning.path,
                    "Repository declares its config in a file the sync no longer reads"
                );
                summary.warnings.push(warning);
            }
            let entry = default_entry();
            if let Some(desired) = self
                .desired_workspace(repo, &full_name, &entry, None, None, &mut summary)
                .await?
            {
                let outcome = self.upsert_git_workspace(repo, &desired).await?;
                record_outcome(&mut summary, outcome);
            }
            return Ok(summary);
        };

        // Reported before anything is applied, so a repo whose committed config
        // was parsed and partly dropped learns which keys went nowhere.
        summary.warnings = unknown_key_warnings(&file);
        summary
            .warnings
            .extend(unknown_language_server_warnings(&file));
        summary.warnings.extend(invalid_icon_warnings(&file));
        for warning in &summary.warnings {
            warn!(
                repository_id = %repo.id,
                source_config_path = WORKSPACES_PATH,
                path = %warning.path,
                "Ignoring unknown entry in git-declared workspace config"
            );
        }

        // Read once, and only when some entry declares `skills`.
        let registry = if file.workspaces.iter().any(|e| e.skills.is_some()) {
            Some((
                self.plugins.list_all().await?,
                self.plugins.list_all_skills().await?,
            ))
        } else {
            None
        };

        let synthesized = default_entry();
        let declares_default = file
            .workspaces
            .iter()
            .any(|e| source_key_for(e).as_deref() == Some(DEFAULT_WORKSPACE_KEY));
        let entries = file
            .workspaces
            .iter()
            .enumerate()
            .map(|(i, e)| (Some(i), e))
            .chain((!declares_default).then_some((None, &synthesized)));

        // source_keys present in git this round — reconcile-delete keeps only these.
        let mut present: HashSet<String> = HashSet::new();
        for (index, entry) in entries {
            let Some(desired) = self
                .desired_workspace(
                    repo,
                    &full_name,
                    entry,
                    index,
                    registry.as_ref(),
                    &mut summary,
                )
                .await?
            else {
                continue;
            };
            if !present.insert(desired.source_key.clone()) {
                // A duplicate source_key would violate workspaces_git_source_unique.
                warn!(repository_id = %repo.id, source_key = %desired.source_key, "Duplicate workspace source_key; skipping later entry");
                summary.skipped.push(WorkspaceSyncSkip {
                    source_key: desired.source_key,
                    reason: "duplicate source_key".to_string(),
                });
                continue;
            }
            let outcome = self.upsert_git_workspace(repo, &desired).await?;
            record_outcome(&mut summary, outcome);
        }

        // Reconcile-delete: soft-delete any git workspace for this repo whose
        // source_key is no longer declared. Soft so in-flight tasks pointing at it
        // keep working — provision lookups don't filter `deleted`.
        let existing = super::dao::git_source_keys(&self.db, &repo.id).await?;
        #[expect(
            clippy::disallowed_methods,
            reason = "the caller's access to trigger the sync was checked"
        )]
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

    /// Resolve one entry to the Workspace it declares, or `None` with the
    /// reason pushed onto `summary.skipped`.
    ///
    /// `index` is the entry's position in the file, `None` for the synthesized
    /// default, and prefixes the JSON path of every warning.
    async fn desired_workspace(
        &self,
        repo: &Repository,
        full_name: &str,
        entry: &MoatlessWorkspaceEntry,
        index: Option<usize>,
        registry: Option<&(Vec<Plugin>, Vec<PluginSkill>)>,
        summary: &mut WorkspaceSyncSummary,
    ) -> Result<Option<DesiredWorkspace>, RepositoryError> {
        let Some(source_key) = source_key_for(entry) else {
            summary.skipped.push(WorkspaceSyncSkip {
                source_key: String::new(),
                reason: "an entry needs a name or a key".to_string(),
            });
            return Ok(None);
        };
        let is_default = source_key == DEFAULT_WORKSPACE_KEY;
        let Some(name) = entry
            .name
            .clone()
            .or_else(|| is_default.then(|| repo.name.clone()))
        else {
            summary.skipped.push(WorkspaceSyncSkip {
                source_key,
                reason: "name is required".to_string(),
            });
            return Ok(None);
        };
        if let Err(e) = validate_non_secret_env_vars(entry.env_vars.as_ref()) {
            warn!(repository_id = %repo.id, %source_key, error = %e, "Skipping workspace with invalid envVars");
            summary.skipped.push(WorkspaceSyncSkip {
                source_key,
                reason: e.to_string(),
            });
            return Ok(None);
        }

        let mut run = run_config_from_entry(entry);
        // Only the global registration: anyone may register a remote for
        // themselves, and would otherwise receive its grant.
        let grant = (repo.scope == Scope::Global)
            .then(|| self.identity_grants.find(full_name, &source_key))
            .flatten();
        for field in apply_identity_grant(&mut run, grant, &repo.id, &source_key) {
            summary.warnings.push(WorkspaceSyncWarning {
                path: match index {
                    Some(i) => format!("workspaces[{i}].{field}"),
                    None => field.to_string(),
                },
                message: UNGRANTED_IDENTITY.to_string(),
            });
        }
        let activations = match index {
            Some(i) => {
                let reported = summary.warnings.len();
                let activations = declared_activations(entry, registry, i, &mut summary.warnings);
                for warning in &summary.warnings[reported..] {
                    warn!(
                        repository_id = %repo.id,
                        source_config_path = WORKSPACES_PATH,
                        path = %warning.path,
                        message = %warning.message,
                        "Ignoring skill name in git-declared workspace config"
                    );
                }
                activations
            }
            None => None,
        };
        let places_nothing = places_no_repository(entry, is_default);
        if entry.place_repository == Some(false) && !places_nothing {
            summary.warnings.push(WorkspaceSyncWarning {
                path: match index {
                    Some(i) => format!("workspaces[{i}].placeRepository"),
                    None => "placeRepository".to_string(),
                },
                message: KEPT_PLACEMENT.to_string(),
            });
        }
        let placements = if places_nothing {
            Vec::new()
        } else if entry.repos.is_empty() {
            vec![ResolvedPlacement {
                repository_id: repo.id.clone(),
                branch: None,
                mount_name: None,
                primary: true,
            }]
        } else {
            self.resolve_placements(repo, &entry.repos).await?
        };
        let (scope, owner_user_id) = match grant.and_then(|g| g.owner_user_id.clone()) {
            Some(owner) => (Scope::User, Some(owner)),
            None => (repo.scope, repo.owner_user_id.clone()),
        };

        Ok(Some(DesiredWorkspace {
            source_key,
            source_config_path: WORKSPACES_PATH.to_string(),
            name,
            description: entry.description.clone(),
            scope,
            owner_user_id,
            run,
            placements,
            activations,
        }))
    }

    /// The commit sha a GitHub repository's default branch points at, read
    /// with the credential `scope` resolves.
    ///
    /// Errors on a non-GitHub remote, and on any answer but a sha.
    pub async fn default_branch_head(
        &self,
        repo: &Repository,
        scope: &ScopeContext,
    ) -> Result<BranchHead, RepositoryError> {
        let remote = repo
            .remote
            .as_ref()
            .ok_or_else(|| RepositoryError::BadRequest("Repository has no remote URL".into()))?;
        let host_repo = require_host_repo(remote)?;
        if host_repo.provider() != RepositoryProvider::Github {
            return Err(RepositoryError::BadRequest(
                "Only a GitHub repository's head is read".into(),
            ));
        }
        let token = self.resolve_sync_token(scope, host_repo).await;
        let sha = crate::repository::config_reader::github_branch_head(
            &self.http,
            &self.github_api_base,
            remote,
            repo.default_branch.as_deref(),
            token.as_deref(),
        )
        .await?;
        Ok(BranchHead {
            sha,
            authenticated: token.is_some(),
        })
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
            &self.http,
            &self.github_api_base,
            remote,
            branch,
            token_str,
        )
        .await
        {
            debug!(repository_id = %repo.id, "Config auto-sync: no .moatless directory, skipping");
            return;
        }

        match self.sync_repo_workspaces_for(repo, scope).await {
            Ok(workspaces) => {
                info!(
                    repository_id = %repo.id,
                    workspaces_created = workspaces.created,
                    workspaces_updated = workspaces.updated,
                    workspaces_skipped = workspaces.skipped.len(),
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
fn record_outcome(summary: &mut WorkspaceSyncSummary, outcome: UpsertOutcome) {
    match outcome {
        UpsertOutcome::Created => summary.created += 1,
        UpsertOutcome::Updated => summary.updated += 1,
        UpsertOutcome::Reclaimed => summary.reclaimed += 1,
    }
}

/// The entry a repository with no `default` entry is synced from.
fn default_entry() -> MoatlessWorkspaceEntry {
    MoatlessWorkspaceEntry {
        key: Some(DEFAULT_WORKSPACE_KEY.to_string()),
        ..MoatlessWorkspaceEntry::default()
    }
}

/// What the sync says about an identity field it dropped.
const UNGRANTED_IDENTITY: &str =
    "ignored: no WORKSPACE_IDENTITY_GRANTS entry grants this value to this workspace";

/// Drop each privileged sandbox field whose declared value `grant` does not
/// name, returning the fields dropped.
///
/// Anyone with commit access edits the file, so an ungranted value would
/// reach admin-gated Secrets. Dropped rather than rejected, so one stray
/// field cannot fail the whole sync.
fn apply_identity_grant(
    run: &mut RunConfig,
    grant: Option<&WorkspaceIdentityGrant>,
    repository_id: &str,
    source_key: &str,
) -> Vec<&'static str> {
    let mut stripped = Vec::new();
    if run.sandbox_service_account.is_some()
        && run.sandbox_service_account.as_ref()
            != grant.and_then(|g| g.sandbox_service_account.as_ref())
    {
        run.sandbox_service_account = None;
        stripped.push("sandboxServiceAccount");
    }
    if run.network_profile.is_some()
        && run.network_profile.as_ref() != grant.and_then(|g| g.network_profile.as_ref())
    {
        run.network_profile = None;
        stripped.push("networkProfile");
    }
    if run.node_selector.is_some()
        && run.node_selector.as_ref() != grant.and_then(|g| g.node_selector.as_ref())
    {
        run.node_selector = None;
        stripped.push("nodeSelector");
    }
    for field in &stripped {
        warn!(
            %repository_id,
            %source_key,
            field,
            "Ignoring a repo-declared sandbox identity no grant allows"
        );
    }
    stripped
}

/// The run-config fields a git-declared workspace carries, with its icon and
/// Task defaults.
struct RunConfig {
    icon: Option<WorkspaceIcon>,
    default_model: Option<String>,
    default_effort: Option<String>,
    linear_team: Option<String>,
    docker_image: Option<String>,
    docker_access: Option<bool>,
    sandbox_service_account: Option<String>,
    network_profile: Option<String>,
    node_selector: Option<BTreeMap<String, String>>,
    system_prompt: Option<String>,
    setup_commands: Option<Vec<String>>,
    env_vars: Option<HashMap<String, String>>,
    scripts: Option<Vec<ScriptConfig>>,
    resources: Option<ResourceConfig>,
    language_servers: Option<Vec<LanguageServer>>,
    services: Option<Vec<String>>,
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
    /// The repository's, unless a grant makes the Workspace user-scoped.
    scope: Scope,
    owner_user_id: Option<String>,
    run: RunConfig,
    placements: Vec<ResolvedPlacement>,
    /// `None` when the entry has no `skills` key, which leaves the
    /// Workspace's activation records alone.
    activations: Option<Vec<DeclaredActivation>>,
}

/// The stable natural key for a workspaces.json entry: its `key`, else the
/// slugified `name`, `None` with neither. Documenting `key` as the stability
/// contract — renaming a keyed entry updates in place; a name-keyed entry
/// churns (prune + create).
fn source_key_for(entry: &MoatlessWorkspaceEntry) -> Option<String> {
    entry
        .key
        .as_deref()
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .map(str::to_string)
        .or_else(|| entry.name.as_deref().map(slugify))
}

/// The records an entry's `skills` declares, reporting each name it cannot
/// honour into `warnings`.
fn declared_activations(
    entry: &MoatlessWorkspaceEntry,
    registry: Option<&(Vec<Plugin>, Vec<PluginSkill>)>,
    index: usize,
    warnings: &mut Vec<WorkspaceSyncWarning>,
) -> Option<Vec<DeclaredActivation>> {
    let declared = entry.skills.as_ref()?;
    let (plugins, skills) = registry?;
    let path = format!("workspaces[{index}].skills");
    let (activations, found) = resolve_declared(declared, plugins, skills, &path);
    warnings.extend(found);
    Some(activations)
}

fn run_config_from_entry(entry: &MoatlessWorkspaceEntry) -> RunConfig {
    RunConfig {
        icon: parse_icon(entry),
        default_model: entry.default_model.clone(),
        default_effort: entry.default_effort.clone(),
        linear_team: entry
            .linear_team
            .as_deref()
            .map(|team| team.trim().to_uppercase())
            .filter(|team| !team.is_empty()),
        docker_image: entry.docker_image.clone(),
        docker_access: entry.docker_access,
        sandbox_service_account: entry.sandbox_service_account.clone(),
        network_profile: entry.network_profile.clone(),
        node_selector: entry.node_selector.clone(),
        system_prompt: entry.system_prompt.clone(),
        setup_commands: entry
            .code_execution
            .as_ref()
            .and_then(|c| c.setup_commands.clone()),
        env_vars: entry.env_vars.clone(),
        scripts: entry.scripts.clone(),
        resources: entry.resources.clone(),
        language_servers: entry.language_servers.as_ref().map(|ids| {
            ids.iter()
                .filter_map(|id| id.parse::<LanguageServer>().ok())
                .collect()
        }),
        services: entry.services.clone(),
    }
}

impl GitSyncService {
    /// Insert or update the workspace row + placements for one desired
    /// workspace, taking back a legacy manual override that holds its key.
    async fn upsert_git_workspace(
        &self,
        repo: &Repository,
        desired: &DesiredWorkspace,
    ) -> Result<UpsertOutcome, RepositoryError> {
        #[expect(
            clippy::disallowed_methods,
            reason = "workspaces derive from the repository row, not caller input"
        )]
        let sys_ctx = ScopeContext::system();
        // Look up by natural key — include 'manual' so a legacy override is
        // reclaimed rather than re-created under the same key. Soft-deleted
        // rows are reused too, so a delete never orphans the key behind a row
        // that no lookup would find again.
        let existing =
            super::dao::workspace_for_git_source(&self.db, &repo.id, &desired.source_key).await?;

        let (workspace_id, outcome) = match existing {
            Some((id, source)) if source == "manual" => (id, UpsertOutcome::Reclaimed),
            Some((id, _)) => (id, UpsertOutcome::Updated),
            None => {
                let id = format!("ws_{}", uuid::Uuid::new_v4());
                let row = Workspace {
                    id: id.clone(),
                    name: desired.name.clone(),
                    description: desired.description.clone(),
                    icon: desired.run.icon.clone(),
                    default_model: desired.run.default_model.clone(),
                    default_effort: desired.run.default_effort.clone(),
                    linear_team: desired.run.linear_team.clone(),
                    kind: WorkspaceKind::Single, // `replace_git_placements` derives it
                    docker_image: desired.run.docker_image.clone(),
                    docker_access: desired.run.docker_access,
                    sandbox_service_account: desired.run.sandbox_service_account.clone(),
                    system_prompt: desired.run.system_prompt.clone(),
                    setup_commands: desired.run.setup_commands.clone(),
                    env_vars: desired.run.env_vars.clone(),
                    scripts: desired.run.scripts.clone(),
                    resources: desired.run.resources.clone(),
                    language_servers: desired.run.language_servers.clone(),
                    network_profile: desired.run.network_profile.clone(),
                    source: WorkspaceSource::Git,
                    source_repository_id: Some(repo.id.clone()),
                    source_key: Some(desired.source_key.clone()),
                    source_config_path: Some(desired.source_config_path.clone()),
                    scope: desired.scope,
                    owner_user_id: desired.owner_user_id.clone(),
                    // Never declared in git: commit access to a repository
                    // would otherwise name the account its Tasks act as.
                    run_as_user_id: None,
                    node_selector: desired.run.node_selector.clone(),
                    services: desired.run.services.clone(),
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
            scope: Some(desired.scope),
            owner_user_id: Some(desired.owner_user_id.clone()),
            // Absent from the patch, so a reconcile leaves whatever an admin set.
            run_as_user_id: None,
            node_selector: Some(desired.run.node_selector.clone()),
            services: Some(desired.run.services.clone()),
            icon: Some(desired.run.icon.clone()),
            default_model: Some(desired.run.default_model.clone()),
            default_effort: Some(desired.run.default_effort.clone()),
            linear_team: Some(desired.run.linear_team.clone()),
            docker_image: Some(desired.run.docker_image.clone()),
            docker_access: Some(desired.run.docker_access),
            sandbox_service_account: Some(desired.run.sandbox_service_account.clone()),
            system_prompt: Some(desired.run.system_prompt.clone()),
            setup_commands: Some(desired.run.setup_commands.clone()),
            env_vars: Some(desired.run.env_vars.clone()),
            scripts: Some(desired.run.scripts.clone()),
            resources: Some(desired.run.resources.clone()),
            language_servers: Some(desired.run.language_servers.clone()),
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
                position: i32::try_from(i).unwrap_or(i32::MAX),
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

        if let Some(activations) = &desired.activations {
            replace_workspace_activations(&self.db, &workspace_id, activations).await?;
        }

        if matches!(outcome, UpsertOutcome::Reclaimed) {
            info!(
                %workspace_id, source_key = %desired.source_key,
                "Reclaimed a manual override from git; local edits replaced"
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
        ensure_remote_url_scheme_allowed(url)?;
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

pub async fn log_workspace_sync_for(
    git_sync: &GitSyncService,
    repo: &Repository,
    scope: &ScopeContext,
    phase: &str,
) -> WorkspaceSyncSummary {
    match git_sync.sync_repo_workspaces_for(repo, scope).await {
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

#[cfg(test)]
mod tests;
