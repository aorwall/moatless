//! Provider-agnostic lifecycle plan construction and pod-watcher config reads.
//!
//! Lifecycle uses [`build_provision_request`] to assemble a fresh runtime plan
//! from durable Task, Workspace, repository, and plugin state. The Kubernetes
//! pod watcher also uses the server-config/disabled-server parsing helpers when
//! classifying and remediating server failures. HTTP and adapter callers do not
//! provision through this module; they call `state.sandbox`.

use std::collections::HashSet;

use tracing::{info, warn};

use moatless_backend_core::scope::ScopeContext;

use crate::auth::provider_token_dao::ProviderTokenRepository;
use crate::plugin_repo::dao::PluginRepoRepository;
use crate::repository::dao::RepoRepository;
use crate::repository::models::RepositoryProvider;
use crate::sandbox::error::SandboxError;
use crate::sandbox::models::{
    derive_mount_name, sanitize_mount_name, ProvisionRequest, RepoCheckout, RepoConfig, ServerConfig,
};
use crate::task::dao::TaskRepository;
use crate::task::models::Task;
use crate::workspace::dao::WorkspaceRepository;

// Re-export the workspace-sourced config builders from k8s/auto_provision. They
// are shared by lifecycle plan construction and pod-watcher parsing.
pub use crate::sandbox::k8s::auto_provision::{
    build_repo_config_from_workspace, build_server_configs_from_workspace, filter_disabled_servers,
    parse_disabled_servers, parse_env_vars_from_workspace,
};

/// Build a `ProvisionRequest` from database state.
///
/// Provider-agnostic: works for both K8s and Docker sandboxes.
///
/// Steps:
/// 1. Read the task row to get the workspace/repository reference + branch
/// 2. Resolve the primary repository (via the workspace when present) and read
///    its row to get repo config
/// 3. Assemble the ProvisionRequest. Runtime credentials are fetched later
///    over gRPC by the sandbox.
pub async fn build_provision_request(
    tasks: &dyn TaskRepository,
    repos: &dyn RepoRepository,
    _provider_tokens: &dyn ProviderTokenRepository,
    plugin_repos: &dyn PluginRepoRepository,
    workspaces: &dyn WorkspaceRepository,
    task_id: &str,
) -> Result<ProvisionRequest, SandboxError> {
    // 1. Read task from DB
    // reason: lifecycle plan construction acts on behalf of the system, not a user
    #[allow(clippy::disallowed_methods)]
    let sys = ScopeContext::system();
    let task = tasks
        .find_by_id(task_id, &sys)
        .await
        .map_err(|source| SandboxError::Database {
            context: "reading the task row for provisioning",
            source,
        })?
        .ok_or_else(|| SandboxError::Internal(format!("Task '{task_id}' not found in database")))?;

    // Gate: never provision a pod for a Closed task. A webhook racing a close
    // (Fix D commits `Closed` first) lands here and is rejected, so the close
    // can no longer be undone by a re-provision — the direct fix for the
    // "close started it again" race. Only `Closed` is gated: `Error` stays
    // resumable to match `start_task`.
    if task.status == crate::task::models::TaskStatus::Closed {
        return Err(SandboxError::BadRequest(format!(
            "Task '{task_id}' is closed; refusing to provision a sandbox"
        )));
    }

    // Resolve which repository this task provisions. When the task references a
    // workspace (Phase A backfills a shadow workspace per repo), use the
    // workspace's primary repo; otherwise fall back to the legacy
    // `repository_id`. In Phase A the shadow workspace's primary repo equals
    // `repository_id`, so the single-repo provisioning path is unchanged.
    let repository_id = resolve_primary_repository_id(workspaces, &task).await;

    info!(
        task_id = %task_id,
        repository_id = %repository_id,
        workspace_id = ?task.workspace_id,
        "Auto-provision: task found in DB"
    );

    // 2. Read repository from DB. A missing row is a permanent misconfiguration
    // (the task references a repository that does not exist) — retrying can't
    // create it — so it surfaces as a client error, not a transient one. A DB
    // read error stays `Internal` (transient).
    // reason: provision plan reads any repository regardless of owner
    #[allow(clippy::disallowed_methods)]
    let repo_sys = ScopeContext::system();
    let repo = repos
        .find_by_id(&repository_id, &repo_sys)
        .await
        .map_err(|source| SandboxError::Database {
            context: "reading the repository row for provisioning",
            source,
        })?
        .ok_or_else(|| {
            SandboxError::BadRequest(format!(
                "Repository '{repository_id}' not found in database"
            ))
        })?;

    let git_provider = repo.provider.unwrap_or(RepositoryProvider::Github);

    let git_clone_url = repo.remote_url.clone().unwrap_or_default();
    let git_branch = task.branch.clone().or(repo.default_branch.clone());
    let user_id = task.created_by.as_deref().unwrap_or("system").to_string();

    // Run-config source: the task's workspace. insert_task backfills a shadow
    // workspace for every task, so this is present for any real task. The repo
    // row is still read above for the primary checkout's URL/branch/provider,
    // which stay their own plan fields for the legacy `GIT_CLONE_URL` env.
    let workspace = match task.workspace_id.as_deref() {
        Some(ws_id) => workspaces
            .find_by_id(ws_id, &repo_sys)
            .await
            .map_err(|source| SandboxError::Database {
                context: "reading the workspace row for provisioning",
                source,
            })?,
        None => None,
    };

    // Every repository the workspace places in the sandbox. The primary is
    // always first and always present (it is the repo row read above), so a
    // single-repo workspace produces exactly the checkout the pod got before.
    let placements = resolve_placements(repos, workspaces, task_id, task.workspace_id.as_deref())
        .await
        .unwrap_or_default();
    let repo_checkouts = build_repo_checkouts(
        &placements,
        &repository_id,
        &git_clone_url,
        git_branch.as_deref(),
        git_provider.as_str(),
    );
    if repo_checkouts.len() > 1 {
        info!(
            task_id = %task_id,
            count = repo_checkouts.len(),
            mounts = ?repo_checkouts.iter().map(|r| r.mount_name.as_str()).collect::<Vec<_>>(),
            "Auto-provision: multi-repo workspace"
        );
    }

    let (mut repo_config, mut env_vars) = if let Some(ws) = &workspace {
        (
            build_repo_config_from_workspace(ws),
            parse_env_vars_from_workspace(ws),
        )
    } else {
        warn!(
            task_id = %task_id,
            workspace_id = ?task.workspace_id,
            "Auto-provision: task has no workspace; provisioning with git-connection only"
        );
        (minimal_repo_config(&repo.name), None)
    };

    // Select the tmux Claude transport per the catalog when the task's agent
    // requests it. Each task is its own pod, so the pod-level flag is exactly
    // the right granularity.
    if crate::agent::catalog::use_tmux_for(task.agent_type.0.as_str()) {
        env_vars
            .get_or_insert_with(std::collections::HashMap::new)
            .insert("SANDBOX_USE_TMUX".to_string(), "true".to_string());
    }

    // The Task's selected Agent harness is fixed for the Sandbox lifetime
    // (ADR-0004): pass it at construction so the sandbox resolves its harness
    // once and pending Messages can no longer switch it. Inserted after the
    // workspace env so the authoritative Task selection overrides any stray
    // workspace-level `AGENT_TYPE`.
    env_vars
        .get_or_insert_with(std::collections::HashMap::new)
        .insert("AGENT_TYPE".to_string(), task.agent_type.0.clone());

    // Resolve plugin git repos from the join table
    match plugin_repos.find_by_repository_id(&repository_id).await {
        Ok(plugins) if !plugins.is_empty() => {
            let urls: Vec<String> = plugins.into_iter().map(|p| p.git_url).collect();
            info!(
                task_id = %task_id,
                count = urls.len(),
                "Auto-provision: resolved plugin git repos"
            );
            repo_config.plugin_repos = Some(urls);
        }
        Ok(_) => {}
        Err(e) => {
            warn!(
                task_id = %task_id,
                error = %e,
                "Auto-provision: failed to query plugin repos (non-fatal)"
            );
        }
    }

    if task.headless {
        repo_config.servers = None;
        repo_config.preview_config = None;
    }

    // Drop servers this task previously auto-disabled (crash-loop remediation).
    // Persisted per-task so it survives stop/start and never affects sibling
    // tasks sharing the same repository.
    let disabled = parse_disabled_servers(&task);
    if !disabled.is_empty() {
        info!(task_id = %task_id, ?disabled, "Auto-provision: filtering disabled servers");
        filter_disabled_servers(&mut repo_config, &disabled);
    }

    // Server overrides are deliberately absent from the plan. They are files on
    // the workspace volume, so a rebuilt pod finds them already in place; there
    // is no copy here to replay.

    let plan = crate::sandbox::models::ProvisionPlan {
        repo_config,
        git_clone_url,
        git_branch,
        git_provider: git_provider.as_str().to_string(),
        env_vars,
        repos: repo_checkouts,
    };

    Ok(ProvisionRequest { plan, user_id })
}

/// A workspace repository placement with the git-pointer fields of its
/// repository row already resolved.
///
/// Plain owned data so checkout assembly ([`build_repo_checkouts`]) stays a pure,
/// unit-testable function instead of needing DB fakes.
#[derive(Debug, Clone)]
struct ResolvedPlacement {
    repository_id: String,
    clone_url: String,
    provider: String,
    /// The placement's branch override, else the repository's default branch.
    branch: Option<String>,
    /// The placement's explicit mount name, if the author set one.
    mount_name: Option<String>,
    is_primary: bool,
    position: i32,
}

/// Read the task workspace's repository placements and pair each with its
/// repository row.
///
/// Returns `None` when the task has no workspace or the placements can't be
/// read — provisioning then falls back to the primary-only checkout list, which
/// is exactly the pre-multi-repo behaviour. Individual placements whose
/// repository row is missing or has no remote URL are dropped with a warning
/// rather than failing the whole provision: the workspace still has a usable
/// primary, and a dangling placement is a config problem, not a runtime one.
async fn resolve_placements(
    repos: &dyn RepoRepository,
    workspaces: &dyn WorkspaceRepository,
    task_id: &str,
    workspace_id: Option<&str>,
) -> Option<Vec<ResolvedPlacement>> {
    let workspace_id = workspace_id?;
    let placements = match workspaces.find_repos(workspace_id).await {
        Ok(placements) => placements,
        Err(e) => {
            warn!(
                task_id = %task_id,
                workspace_id,
                error = %e,
                "Auto-provision: failed to read workspace repo placements; \
                 provisioning the primary repo only"
            );
            return None;
        }
    };

    // reason: provision plan reads any repository regardless of owner
    #[allow(clippy::disallowed_methods)]
    let sys = ScopeContext::system();
    let mut resolved = Vec::with_capacity(placements.len());
    for placement in placements {
        let repo = match repos.find_by_id(&placement.repository_id, &sys).await {
            Ok(Some(repo)) => repo,
            Ok(None) => {
                warn!(
                    task_id = %task_id,
                    workspace_id,
                    repository_id = %placement.repository_id,
                    "Auto-provision: workspace placement references a missing repository; skipping"
                );
                continue;
            }
            Err(e) => {
                warn!(
                    task_id = %task_id,
                    workspace_id,
                    repository_id = %placement.repository_id,
                    error = %e,
                    "Auto-provision: failed to read a placement's repository row; skipping"
                );
                continue;
            }
        };
        let clone_url = repo.remote_url.clone().unwrap_or_default();
        if clone_url.is_empty() {
            warn!(
                task_id = %task_id,
                repository_id = %placement.repository_id,
                "Auto-provision: workspace placement has no remote URL; skipping"
            );
            continue;
        }
        resolved.push(ResolvedPlacement {
            repository_id: placement.repository_id,
            clone_url,
            provider: repo
                .provider
                .unwrap_or(RepositoryProvider::Github)
                .as_str()
                .to_string(),
            branch: placement.branch.or(repo.default_branch),
            mount_name: placement.mount_name,
            is_primary: placement.is_primary,
            position: placement.position,
        });
    }
    Some(resolved)
}

/// Assemble the ordered checkout list the sandbox clones: the primary first,
/// then the remaining placements by `position`.
///
/// The primary's URL/branch/provider come from the caller (they are the plan's
/// legacy single-repo fields, and the task's branch override already applied),
/// so `repos[0]` is byte-identical to what a single-repo plan carried before —
/// including its mount name, which an existing sandbox's persistent volume
/// already holds the checkout at. Mount names are sanitized to one path segment
/// and de-duplicated, with the primary claiming its name first so its directory
/// never moves.
fn build_repo_checkouts(
    placements: &[ResolvedPlacement],
    primary_repository_id: &str,
    primary_clone_url: &str,
    primary_branch: Option<&str>,
    primary_provider: &str,
) -> Vec<RepoCheckout> {
    // The primary placement is the one flagged in the DB; when nothing is
    // flagged, `resolve_primary_repository_id` picked the task's repository, so
    // match on that instead.
    let primary_idx = placements
        .iter()
        .position(|p| p.is_primary)
        .or_else(|| {
            placements
                .iter()
                .position(|p| p.repository_id == primary_repository_id)
        });

    let mut checkouts = Vec::new();
    let mut used: HashSet<String> = HashSet::new();

    if !primary_clone_url.is_empty() {
        let mount_name = primary_idx
            .and_then(|i| placements[i].mount_name.as_deref())
            .and_then(sanitize_mount_name)
            .unwrap_or_else(|| derive_mount_name(primary_clone_url));
        used.insert(mount_name.clone());
        checkouts.push(RepoCheckout {
            clone_url: primary_clone_url.to_string(),
            branch: primary_branch.map(str::to_string),
            provider: primary_provider.to_string(),
            mount_name,
            is_primary: true,
        });
    }

    let mut secondary: Vec<&ResolvedPlacement> = placements
        .iter()
        .enumerate()
        .filter(|(i, _)| Some(*i) != primary_idx)
        .map(|(_, p)| p)
        .collect();
    secondary.sort_by_key(|p| p.position);

    for placement in secondary {
        let requested = placement
            .mount_name
            .as_deref()
            .and_then(sanitize_mount_name)
            .unwrap_or_else(|| derive_mount_name(&placement.clone_url));
        let mount_name = unique_mount_name(requested, &used);
        used.insert(mount_name.clone());
        checkouts.push(RepoCheckout {
            clone_url: placement.clone_url.clone(),
            branch: placement.branch.clone(),
            provider: placement.provider.clone(),
            mount_name,
            is_primary: false,
        });
    }

    checkouts
}

/// Suffix a mount name until it no longer collides with an already-claimed one.
///
/// Two placements can derive the same name (two repos both called `api`, or the
/// same repo placed twice on different branches) — the DB's uniqueness index
/// only covers explicitly-set names.
fn unique_mount_name(requested: String, used: &HashSet<String>) -> String {
    if !used.contains(&requested) {
        return requested;
    }
    // `used.len() + 2` candidates always contain a free one: there are only
    // `used.len()` taken names.
    (2..=used.len() + 2)
        .map(|n| format!("{requested}-{n}"))
        .find(|candidate| !used.contains(candidate))
        .unwrap_or(requested)
}

/// Resolve the repository a task provisions against: the workspace's primary
/// repo when `task.workspace_id` is set, else the legacy `task.repository_id`.
///
/// Falls back to `repository_id` when the workspace is empty or the lookup
/// fails, so provisioning is never blocked by workspace-resolution issues. In
/// Phase A a task's shadow workspace has exactly one primary repo equal to
/// `repository_id`, so the resolved repository is unchanged.
async fn resolve_primary_repository_id(
    workspaces: &dyn WorkspaceRepository,
    task: &Task,
) -> String {
    let Some(workspace_id) = task.workspace_id.as_deref() else {
        return task.repository_id.clone();
    };
    match workspaces.find_primary_repo(workspace_id).await {
        Ok(Some(placement)) => placement.repository_id,
        Ok(None) => {
            warn!(
                task_id = %task.task_id,
                workspace_id,
                "Auto-provision: workspace has no repos; falling back to task.repository_id"
            );
            task.repository_id.clone()
        }
        Err(e) => {
            warn!(
                task_id = %task.task_id,
                workspace_id,
                error = %e,
                "Auto-provision: failed to resolve workspace primary repo; \
                 falling back to task.repository_id"
            );
            task.repository_id.clone()
        }
    }
}

/// A run-config with only the repo name — used when a task unexpectedly has no
/// workspace. Provisioning then falls back to global defaults (no servers).
fn minimal_repo_config(name: &str) -> RepoConfig {
    RepoConfig {
        name: name.to_string(),
        docker_image: None,
        servers: None,
        plugin_ids: None,
        system_prompt: None,
        code_execution: None,
        preview_config: None,
        resources: None,
        plugin_repos: None,
        docker_access: None,
        docker_networks: None,
        sandbox_service_account: None,
        network_profile: None,
    }
}

/// Get server configs for a task from its workspace, returning an empty vec on
/// any failure.
///
/// Convenience wrapper that handles the Option<Arc<dyn Repo>> pattern
/// used throughout handlers.
pub async fn get_server_configs_for_task(
    tasks: &dyn TaskRepository,
    workspaces: &dyn WorkspaceRepository,
    task_id: &str,
) -> Vec<ServerConfig> {
    match resolve_server_configs(tasks, workspaces, task_id).await {
        Ok(configs) => configs,
        Err(e) => {
            warn!(task_id = %task_id, error = %e, "Failed to get server configs");
            vec![]
        }
    }
}

/// Resolve a task's server configs from its workspace's `servers` JSONB.
///
/// Resolves task_id → task → workspace → `build_server_configs_from_workspace`.
/// Returns `Err` on DB failures or missing rows so callers can log.
async fn resolve_server_configs(
    tasks: &dyn TaskRepository,
    workspaces: &dyn WorkspaceRepository,
    task_id: &str,
) -> Result<Vec<ServerConfig>, String> {
    // reason: server config resolution reads across all task owners for pod watcher
    #[allow(clippy::disallowed_methods)]
    let sys = ScopeContext::system();
    let task = tasks
        .find_by_id(task_id, &sys)
        .await
        .map_err(|e| format!("Failed to fetch task {task_id}: {e}"))?
        .ok_or_else(|| format!("Task {task_id} not found"))?;
    let Some(ws_id) = task.workspace_id.as_deref() else {
        return Ok(vec![]);
    };
    let ws = workspaces
        .find_by_id(ws_id, &sys)
        .await
        .map_err(|e| format!("Failed to fetch workspace {ws_id}: {e}"))?
        .ok_or_else(|| format!("Workspace {ws_id} not found"))?;
    Ok(build_server_configs_from_workspace(&ws))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn placement(
        repository_id: &str,
        clone_url: &str,
        mount_name: Option<&str>,
        is_primary: bool,
        position: i32,
    ) -> ResolvedPlacement {
        ResolvedPlacement {
            repository_id: repository_id.to_string(),
            clone_url: clone_url.to_string(),
            provider: "github".to_string(),
            branch: Some("main".to_string()),
            mount_name: mount_name.map(str::to_string),
            is_primary,
            position,
        }
    }

    /// A single-repo workspace must produce exactly the checkout the pod carried
    /// before multi-repo support: the primary URL/branch/provider from the plan's
    /// legacy fields, mounted at the URL-derived name its volume already uses.
    #[test]
    fn single_repo_workspace_yields_one_primary_checkout() {
        let placements = vec![placement(
            "repo-a",
            "https://github.com/acme/api.git",
            None,
            true,
            0,
        )];
        let checkouts = build_repo_checkouts(
            &placements,
            "repo-a",
            "https://github.com/acme/api.git",
            Some("feature/x"),
            "github",
        );
        assert_eq!(checkouts.len(), 1);
        assert_eq!(checkouts[0].mount_name, "api");
        assert_eq!(checkouts[0].branch.as_deref(), Some("feature/x"));
        assert!(checkouts[0].is_primary);
    }

    #[test]
    fn secondary_repos_follow_the_primary_in_position_order() {
        let placements = vec![
            placement("repo-c", "https://github.com/acme/web.git", None, false, 2),
            placement("repo-a", "https://github.com/acme/api.git", None, true, 0),
            placement(
                "repo-b",
                "https://github.com/acme/shared.git",
                Some("lib"),
                false,
                1,
            ),
        ];
        let checkouts = build_repo_checkouts(
            &placements,
            "repo-a",
            "https://github.com/acme/api.git",
            Some("main"),
            "github",
        );
        let mounts: Vec<&str> = checkouts.iter().map(|c| c.mount_name.as_str()).collect();
        assert_eq!(mounts, vec!["api", "lib", "web"]);
        assert!(checkouts[0].is_primary);
        assert!(!checkouts[1].is_primary && !checkouts[2].is_primary);
    }

    /// The task's branch override applies to the primary only; each secondary
    /// keeps its own placement/default branch.
    #[test]
    fn secondary_repos_keep_their_own_branch_and_provider() {
        let mut secondary = placement(
            "repo-b",
            "https://gitlab.com/acme/shared.git",
            None,
            false,
            1,
        );
        secondary.branch = Some("release".to_string());
        secondary.provider = "gitlab".to_string();
        let placements = vec![
            placement("repo-a", "https://github.com/acme/api.git", None, true, 0),
            secondary,
        ];
        let checkouts = build_repo_checkouts(
            &placements,
            "repo-a",
            "https://github.com/acme/api.git",
            Some("feature/x"),
            "github",
        );
        assert_eq!(checkouts[1].branch.as_deref(), Some("release"));
        assert_eq!(checkouts[1].provider, "gitlab");
        assert_eq!(checkouts[0].branch.as_deref(), Some("feature/x"));
        assert_eq!(checkouts[0].provider, "github");
    }

    /// Two repos can derive the same mount name (the DB index only covers
    /// explicit ones). The primary keeps its directory — its volume already
    /// holds the checkout there — and the collider is suffixed.
    #[test]
    fn colliding_mount_names_are_suffixed_leaving_the_primary_put() {
        let placements = vec![
            placement("repo-a", "https://github.com/one/api.git", None, true, 0),
            placement("repo-b", "https://github.com/two/api.git", None, false, 1),
            placement("repo-c", "https://github.com/three/api.git", None, false, 2),
        ];
        let checkouts = build_repo_checkouts(
            &placements,
            "repo-a",
            "https://github.com/one/api.git",
            None,
            "github",
        );
        let mounts: Vec<&str> = checkouts.iter().map(|c| c.mount_name.as_str()).collect();
        assert_eq!(mounts, vec!["api", "api-2", "api-3"]);
    }

    /// A mount name is joined onto `WORKSPACE_DIR`, so an author-supplied one
    /// that escapes its directory must be reduced to a single segment.
    #[test]
    fn traversal_in_a_mount_name_is_sanitized() {
        let placements = vec![
            placement("repo-a", "https://github.com/acme/api.git", None, true, 0),
            placement(
                "repo-b",
                "https://github.com/acme/shared.git",
                Some("../../etc"),
                false,
                1,
            ),
        ];
        let checkouts = build_repo_checkouts(
            &placements,
            "repo-a",
            "https://github.com/acme/api.git",
            None,
            "github",
        );
        assert_eq!(checkouts[1].mount_name, "..-..-etc");
    }

    /// The same repository placed twice (e.g. two branches side by side) is two
    /// checkouts, not one — placements are identified by position, not repo id.
    #[test]
    fn the_same_repository_can_be_placed_twice() {
        let mut second = placement(
            "repo-a",
            "https://github.com/acme/api.git",
            Some("api-release"),
            false,
            1,
        );
        second.branch = Some("release".to_string());
        let placements = vec![
            placement("repo-a", "https://github.com/acme/api.git", None, true, 0),
            second,
        ];
        let checkouts = build_repo_checkouts(
            &placements,
            "repo-a",
            "https://github.com/acme/api.git",
            Some("main"),
            "github",
        );
        assert_eq!(checkouts.len(), 2);
        assert_eq!(checkouts[1].mount_name, "api-release");
        assert_eq!(checkouts[1].branch.as_deref(), Some("release"));
    }

    /// No placements readable (no workspace, or a DB error) still provisions the
    /// primary repo — the pre-multi-repo path.
    #[test]
    fn no_placements_still_yields_the_primary_checkout() {
        let checkouts = build_repo_checkouts(
            &[],
            "repo-a",
            "https://github.com/acme/api.git",
            Some("main"),
            "github",
        );
        assert_eq!(checkouts.len(), 1);
        assert_eq!(checkouts[0].mount_name, "api");
        assert!(checkouts[0].is_primary);
    }

    /// A blank workspace (no repo at all) provisions a sandbox with nothing to
    /// clone rather than a checkout with an empty URL.
    #[test]
    fn an_empty_primary_url_yields_no_checkouts() {
        let checkouts = build_repo_checkouts(&[], "repo-a", "", None, "github");
        assert!(checkouts.is_empty());
    }

    /// Nothing flagged primary in the DB: the placement matching the task's
    /// resolved repository takes the primary slot and is not duplicated.
    #[test]
    fn unflagged_placements_promote_the_task_repository() {
        let placements = vec![
            placement("repo-b", "https://github.com/acme/web.git", None, false, 0),
            placement("repo-a", "https://github.com/acme/api.git", None, false, 1),
        ];
        let checkouts = build_repo_checkouts(
            &placements,
            "repo-a",
            "https://github.com/acme/api.git",
            None,
            "github",
        );
        let mounts: Vec<&str> = checkouts.iter().map(|c| c.mount_name.as_str()).collect();
        assert_eq!(mounts, vec!["api", "web"]);
    }
}
