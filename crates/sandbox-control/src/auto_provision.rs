//! Provider-agnostic lifecycle plan construction and server-config reads.
//!
//! Lifecycle uses [`build_provision_request`] to assemble a fresh runtime plan
//! from durable Sandbox, Task, Workspace, repository, and plugin state, and
//! [`get_server_configs_for_task`] to list a task's servers. HTTP and adapter
//! callers do not provision through this module; they call `state.sandbox`.

use std::collections::{HashMap, HashSet};

use serde_json::json;

use tracing::{info, warn};

use moatless_types::workspace::{Port, ResourceConfig, ServerConfig};

use crate::error::SandboxError;
use crate::models::{ProvisionRequest, RepoCheckout, RepoConfig, derive_mount_name, sanitize_mount_name};
use crate::lifecycle::orchestrator::LifecycleDao;
use crate::preflight::preflight_validate_sandbox_control;
use crate::secrets::is_platform_owned_key;
use moatless_backend_core::identity::CommitIdentity;
use moatless_backend_core::workspace::{RepoPlacement, ResolvedWorkspace, WorkspaceResolver};
use moatless_types::repository::RepositoryProvider;

// Workspace-sourced run-config builders. A task's run-config comes from its
// workspace (`build_repo_config_from_workspace` and friends); the repository row
// carries no run-config, it is only a git connection.

/// Build a `RepoConfig` from a workspace row — the runtime config source once a
/// task references a workspace. Reads the workspace's JSONB run-config columns
/// (`resources`, `network_profile`, `servers`, …). The workspace has no
/// flattened preview columns, so `preview_config` is `None`; server entries come
/// from the workspace's `servers` JSONB (data-driven).
pub fn build_repo_config_from_workspace(ws: &ResolvedWorkspace) -> RepoConfig {
    let servers = build_server_configs_from_workspace(ws);
    let resources: Option<ResourceConfig> = ws.resources.clone();
    let code_execution = build_code_execution_from_workspace(ws);

    RepoConfig {
        name: ws.name.clone(),
        docker_image: ws.docker_image.clone(),
        servers: if servers.is_empty() {
            None
        } else {
            Some(servers)
        },
        system_prompt: ws.system_prompt.clone(),
        code_execution,
        preview_config: None,
        resources,
        plugins: None,
        docker_access: ws.docker_access,
        docker_networks: None,
        sandbox_service_account: ws.sandbox_service_account.clone(),
        network_profile: ws.network_profile.clone(),
    }
}

/// Build the server list from a workspace's declared servers, ensuring a
/// default is elected and every server has a port assigned.
pub fn build_server_configs_from_workspace(ws: &ResolvedWorkspace) -> Vec<ServerConfig> {
    let mut servers: Vec<ServerConfig> = ws.servers.clone().unwrap_or_default();

    // Ensure at least one default among any present servers, then assign ports.
    if !servers.iter().any(|s| s.default) {
        if let Some(s) = servers.iter_mut().last() {
            s.default = true;
        }
    }
    assign_missing_ports(&mut servers);
    servers
}

/// The environment variables a workspace declares, less the keys the platform
/// owns.
///
/// The plan's `env_vars` are appended to the container env *after* the
/// platform's own — `TASK_ID`, the auth tokens, `BACKEND_GRPC_URL` — in both
/// runtime builders, and a later entry of the same name wins. So this is a
/// channel a workspace could redefine the pod's identity or its git credential
/// through, and it filters with the same [`is_platform_owned_key`] the
/// credential channel does. Neither channel may be the only one that filters:
/// a pod env is just the process's starting environment and the credential map
/// overlays it, so each has to hold on its own.
pub fn parse_env_vars_from_workspace(ws: &ResolvedWorkspace) -> Option<HashMap<String, String>> {
    let mut env_vars = ws.env_vars.clone()?;
    env_vars.retain(|key, _| !is_platform_owned_key(key));
    Some(env_vars)
}

/// Build the code_execution JSON pass-through from a workspace's run config.
fn build_code_execution_from_workspace(ws: &ResolvedWorkspace) -> Option<serde_json::Value> {
    let has_any = ws.setup_commands.is_some() || ws.docker_image.is_some() || ws.env_vars.is_some();
    if !has_any {
        return None;
    }

    let mut config = serde_json::Map::new();
    if let Some(ref cmds) = ws.setup_commands {
        config.insert("setupCommands".to_string(), json!(cmds));
    }
    if let Some(ref image) = ws.docker_image {
        config.insert(
            "dockerImage".to_string(),
            serde_json::Value::String(image.clone()),
        );
    }
    if let Some(ref vars) = ws.env_vars {
        config.insert("envVars".to_string(), json!(vars));
    }
    Some(serde_json::Value::Object(config))
}

/// Assign sequential ports starting at 3000 to servers that declared none.
fn assign_missing_ports(servers: &mut [ServerConfig]) {
    let mut used: HashSet<u16> = servers
        .iter()
        .filter_map(|s| s.port.map(Port::get))
        .collect();

    let mut next_port: u16 = 3000;
    for server in servers.iter_mut() {
        if server.port.is_some() {
            continue;
        }
        while used.contains(&next_port) {
            next_port += 1;
        }
        used.insert(next_port);
        server.port = Port::new(next_port);
        next_port += 1;
    }
}

/// Build a `ProvisionRequest` from database state.
///
/// Provider-agnostic: works for both K8s and Docker sandboxes.
///
/// Steps:
/// 1. Read the Sandbox row for the pod's shape: its workspace, branch, harness,
///    headless flag, and the two identities it carries
/// 2. Resolve the workspace's primary repository and read its row for repo config
/// 3. Check the identity the pod authenticates as can control the Sandbox
/// 4. Assemble the ProvisionRequest. Runtime credentials are fetched later
///    over gRPC by the sandbox.
///
/// Every path to a pod builds its plan here — a first start, a wake, a restart,
/// a reconciler retry, the scale-up of a reaped sandbox — so step 3 is what
/// keeps a sandbox whose pod would be refused its credentials from being
/// re-provisioned indefinitely. It rejects as [`SandboxError::Forbidden`],
/// which lifecycle classifies as permanent, so the failure is published once
/// instead of retried.
///
/// The Closed guard is not here: it is per-Task, and a Sandbox has members
/// rather than a status. `Lifecycle::guard_toward_running` applies it to the
/// Task actually driving the operation.
///
/// Only the control half of the preflight runs here. The GitHub probe stays on
/// creation and task-start: this function also runs on every message wake of an
/// already-running sandbox, and a network call on that path would put message
/// delivery behind GitHub's availability.
pub async fn build_provision_request(
    sandboxes: &dyn LifecycleDao,
    workspaces: &dyn WorkspaceResolver,
    users: &dyn CommitIdentity,
    sandbox_id: &str,
) -> Result<ProvisionRequest, SandboxError> {
    // 1. Read the Sandbox row — the whole pod shape, no member task involved.
    let shape = sandboxes
        .sandbox_shape(sandbox_id)
        .await
        .map_err(|e| SandboxError::Internal(format!("reading sandbox '{sandbox_id}': {e}")))?
        .ok_or_else(|| {
            SandboxError::Internal(format!("Sandbox '{sandbox_id}' not found in database"))
        })?;

    // 2. Every repository the workspace places, primary first, each with its
    // repository row already resolved. A read failure is transient and
    // propagates; a placement pointing at a row that is gone is a permanent
    // misconfiguration, reported below.
    let placements = workspaces.placements(&shape.workspace_id).await?;
    let primary = placements.first().ok_or_else(|| {
        SandboxError::BadRequest(format!(
            "Workspace '{}' has no primary repository; \
             refusing to provision sandbox '{sandbox_id}'",
            shape.workspace_id
        ))
    })?;
    let repository_id = primary.repository_id.clone();

    info!(
        sandbox_id = %sandbox_id,
        repository_id = %repository_id,
        workspace_id = %shape.workspace_id,
        "Auto-provision: sandbox found in DB"
    );

    let repo = primary.repository.clone().ok_or_else(|| {
        SandboxError::BadRequest(format!(
            "Repository '{repository_id}' not found in database"
        ))
    })?;

    // 3. Control check, before anything is built or scheduled.
    if let Err(e) = preflight_validate_sandbox_control(users, sandbox_id, &shape).await {
        warn!(
            sandbox_id = %sandbox_id,
            error = %e,
            "Auto-provision: the sandbox's user cannot control it; refusing to provision"
        );
        return Err(SandboxError::Forbidden(e.user_message()));
    }

    let git_provider = repo.provider.unwrap_or(RepositoryProvider::Github);

    let git_clone_url = repo.remote_url.clone().unwrap_or_default();
    let git_branch = shape.branch.clone().or(repo.default_branch.clone());
    // The identity the pod authenticates back to the backend as — the one the
    // gRPC control gate checks, which is why it is the Sandbox's owner and not
    // the account it acts as outward.
    let user_id = shape
        .owner_user_id
        .clone()
        .unwrap_or_else(|| "system".to_string());

    // Run-config source: the sandbox's workspace. The primary placement above
    // supplies the checkout's URL/branch/provider, which also feed the
    // `GIT_CLONE_URL` env.
    let workspace = workspaces.resolve(&shape.workspace_id).await?;

    let resolved = resolve_placements(&placements, sandbox_id, &shape.workspace_id);
    let repo_checkouts = build_repo_checkouts(
        &resolved,
        &repository_id,
        &git_clone_url,
        git_branch.as_deref(),
        git_provider.as_str(),
    );
    if repo_checkouts.len() > 1 {
        info!(
            sandbox_id = %sandbox_id,
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
            sandbox_id = %sandbox_id,
            workspace_id = %shape.workspace_id,
            "Auto-provision: sandbox workspace is missing; provisioning with git-connection only"
        );
        (minimal_repo_config(&repo.name), None)
    };

    // The Agent harness is fixed for the Sandbox lifetime: pass it at
    // construction so the sandbox resolves it once. Inserted after the workspace
    // env so the authoritative Sandbox selection overrides any stray
    // workspace-level `AGENT_TYPE`.
    if shape.use_tmux {
        env_vars
            .get_or_insert_with(std::collections::HashMap::new)
            .insert("SANDBOX_USE_TMUX".to_string(), "true".to_string());
    }
    if let Some(agent_type) = shape.agent_type.clone() {
        env_vars
            .get_or_insert_with(std::collections::HashMap::new)
            .insert("AGENT_TYPE".to_string(), agent_type);
    }

    // Resolve the plugins the identity this pod acts as has turned on. A sandbox
    // with no identifiable run-as identity gets the global defaults and nothing
    // personal — the column is polymorphic text, so a sentinel like `system` or
    // a `bot:` value is nobody rather than an error.
    let run_as = shape
        .run_as_user_id
        .as_deref()
        .and_then(|id| uuid::Uuid::parse_str(id).ok());
    match workspaces.plugin_deliveries(run_as).await {
        Ok(deliveries) if !deliveries.is_empty() => {
            info!(
                sandbox_id = %sandbox_id,
                count = deliveries.len(),
                "Auto-provision: resolved plugins"
            );
            repo_config.plugins = Some(deliveries);
        }
        Ok(_) => {}
        Err(e) => {
            // A task that starts without its plugins is worse than one that
            // does not start at all only in theory; in practice the agent still
            // works, so this stays non-fatal as it always has.
            warn!(
                sandbox_id = %sandbox_id,
                error = %e,
                "Auto-provision: failed to resolve plugins (non-fatal)"
            );
        }
    }

    // A headless sandbox has no browser on it: no preview servers to run, and no
    // one watching for a prompt, so the sandbox also drops the tools that park
    // a turn on a person's answer.
    if shape.headless {
        repo_config.servers = None;
        repo_config.preview_config = None;
        env_vars
            .get_or_insert_with(std::collections::HashMap::new)
            .insert("SANDBOX_HEADLESS".to_string(), "true".to_string());
    }

    // Server overrides are deliberately absent from the plan. They are files on
    // the workspace volume, so a rebuilt pod finds them already in place; there
    // is no copy here to replay.

    let plan = crate::models::ProvisionPlan {
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

/// Narrow the workspace's placements to the ones a sandbox can actually check
/// out.
///
/// A placement whose repository row is missing or has no remote URL is dropped
/// with a warning rather than failing the whole provision: the workspace still
/// has a usable primary, and a dangling placement is a config problem, not a
/// runtime one.
fn resolve_placements(
    placements: &[RepoPlacement],
    sandbox_id: &str,
    workspace_id: &str,
) -> Vec<ResolvedPlacement> {
    let mut resolved = Vec::with_capacity(placements.len());
    for placement in placements {
        let Some(repo) = placement.repository.as_ref() else {
            warn!(
                sandbox_id = %sandbox_id,
                workspace_id,
                repository_id = %placement.repository_id,
                "Auto-provision: workspace placement references a missing repository; skipping"
            );
            continue;
        };
        let clone_url = repo.remote_url.clone().unwrap_or_default();
        if clone_url.is_empty() {
            warn!(
                sandbox_id = %sandbox_id,
                repository_id = %placement.repository_id,
                "Auto-provision: workspace placement has no remote URL; skipping"
            );
            continue;
        }
        resolved.push(ResolvedPlacement {
            repository_id: placement.repository_id.clone(),
            clone_url,
            provider: repo
                .provider
                .unwrap_or(RepositoryProvider::Github)
                .as_str()
                .to_string(),
            branch: placement.branch.clone().or(repo.default_branch.clone()),
            mount_name: placement.mount_name.clone(),
            is_primary: placement.is_primary,
            position: placement.position,
        });
    }
    resolved
}

/// Assemble the ordered checkout list the sandbox clones: the primary first,
/// then the remaining placements by `position`.
///
/// The primary's URL/branch/provider come from the caller with the task's branch
/// override already applied. The primary mount name stays stable because an
/// existing sandbox's persistent volume already holds the checkout there. Mount
/// names are sanitized to one path segment and de-duplicated, with the primary
/// claiming its name first so its directory never moves.
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
    let primary_idx = placements.iter().position(|p| p.is_primary).or_else(|| {
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

/// A run-config with only the repo name — used when a task unexpectedly has no
/// workspace. Provisioning then falls back to global defaults (no servers).
fn minimal_repo_config(name: &str) -> RepoConfig {
    RepoConfig {
        name: name.to_string(),
        docker_image: None,
        servers: None,
        system_prompt: None,
        code_execution: None,
        preview_config: None,
        resources: None,
        plugins: None,
        docker_access: None,
        docker_networks: None,
        sandbox_service_account: None,
        network_profile: None,
    }
}

/// Get a workspace's server configs, returning an empty vec on any failure.
pub async fn get_server_configs_for_workspace(
    workspaces: &dyn WorkspaceResolver,
    workspace_id: &str,
) -> Vec<ServerConfig> {
    match resolve_server_configs(workspaces, workspace_id).await {
        Ok(configs) => configs,
        Err(e) => {
            warn!(workspace_id, error = %e, "Failed to get server configs");
            vec![]
        }
    }
}

/// Resolve server configs from a workspace's `servers` JSONB.
///
/// Returns `Err` on DB failures so callers can log. A workspace that no longer
/// exists reads as no servers rather than a failure to report on every status
/// pass — the same answer as a workspace that declares none.
async fn resolve_server_configs(
    workspaces: &dyn WorkspaceResolver,
    workspace_id: &str,
) -> Result<Vec<ServerConfig>, String> {
    let Some(ws) = workspaces
        .resolve(workspace_id)
        .await
        .map_err(|e| format!("Failed to fetch workspace {workspace_id}: {e}"))?
    else {
        return Ok(vec![]);
    };
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

    fn workspace_declaring(env_vars: HashMap<String, String>) -> ResolvedWorkspace {
        ResolvedWorkspace {
            id: "ws_1".to_string(),
            name: "ws".to_string(),
            docker_image: None,
            docker_access: None,
            sandbox_service_account: None,
            system_prompt: None,
            setup_commands: None,
            env_vars: Some(env_vars),
            servers: None,
            resources: None,
            network_profile: None,
            run_as_user_id: None,
            deleted: false,
        }
    }

    /// The plan's vars are appended after the platform's own in both runtime
    /// builders and a later entry of the same name wins, so the pod spec has to
    /// drop these itself — the credential channel's filter never sees them.
    #[test]
    fn a_workspace_cannot_reach_the_pod_spec_with_a_platform_owned_key() {
        let env_vars = parse_env_vars_from_workspace(&workspace_declaring(HashMap::from([
            ("TASK_ID".to_string(), "not-my-task".to_string()),
            ("USER_AUTH_TOKEN".to_string(), "forged".to_string()),
            ("BACKEND_GRPC_URL".to_string(), "http://evil".to_string()),
            ("GITHUB_TOKEN".to_string(), "ghp_bypass".to_string()),
            ("GH_TOKEN".to_string(), "ghp_bypass".to_string()),
            ("GIT_TOKEN".to_string(), "ghp_bypass".to_string()),
            ("BUILD_PROFILE".to_string(), "release".to_string()),
        ])))
        .expect("a workspace that declares vars still yields a map");

        assert_eq!(
            env_vars,
            HashMap::from([("BUILD_PROFILE".to_string(), "release".to_string())])
        );
    }

    #[test]
    fn a_workspace_declaring_no_vars_still_yields_none() {
        let mut ws = workspace_declaring(HashMap::new());
        ws.env_vars = None;
        assert!(parse_env_vars_from_workspace(&ws).is_none());
    }

    /// A single-repo workspace produces one primary checkout at the URL-derived
    /// mount name its volume already uses.
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
    /// primary repo.
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
