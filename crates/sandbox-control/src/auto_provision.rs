//! Provider-agnostic lifecycle plan construction.
//!
//! Lifecycle uses [`build_provision_request`] to assemble a fresh runtime plan
//! from durable Sandbox, Task, Workspace, repository, and plugin state. HTTP
//! and adapter callers do not provision through this module; they call
//! `state.sandbox`.

use std::collections::{HashMap, HashSet};

use serde_json::json;

use tracing::{info, warn};

use moatless_workspace_types::workspace::ResourceConfig;

use crate::lifecycle::dao::LifecycleDao;
use crate::preflight::preflight_validate_sandbox_control;
use crate::secrets::is_platform_owned_key;
use moatless_ports::identity::CommitIdentity;
use moatless_sandbox_runtime::error::SandboxError;
use moatless_sandbox_runtime::provisioning::{ProvisionRequest, RepoConfig};
use moatless_workspace_ports::{RepoPlacement, ResolvedWorkspace, WorkspaceResolver};
use moatless_workspace_types::workspace::{RepoCheckout, derive_mount_name, sanitize_mount_name};

// Workspace-sourced run-config builders. A task's run-config comes from its
// workspace (`build_repo_config_from_workspace` and friends); the repository row
// carries no run-config, it is only a git connection.

/// Builds a `RepoConfig` from a workspace row.
///
/// Reads the workspace's JSONB run-config columns (`resources`,
/// `network_profile`, …).
pub fn build_repo_config_from_workspace(ws: &ResolvedWorkspace) -> RepoConfig {
    let resources: Option<ResourceConfig> = ws.resources.clone();
    let code_execution = build_code_execution_from_workspace(ws);

    RepoConfig {
        name: ws.name.clone(),
        docker_image: ws.docker_image.clone(),
        system_prompt: ws.system_prompt.clone(),
        code_execution,
        resources,
        plugins: None,
        removed_plugins: None,
        language_servers: ws.language_servers.clone(),
        docker_access: ws.docker_access,
        docker_networks: None,
        sandbox_service_account: ws.sandbox_service_account.clone(),
        network_profile: ws.network_profile.clone(),
        node_selector: ws.node_selector.clone(),
    }
}

/// The environment variables a workspace declares, less the keys the platform
/// owns.
///
/// Appended into the container env after the platform's own, where a later
/// entry of the same name wins.
pub fn parse_env_vars_from_workspace(ws: &ResolvedWorkspace) -> Option<HashMap<String, String>> {
    let mut env_vars = ws.env_vars.clone()?;
    env_vars.retain(|key, _| !is_platform_owned_key(key));
    Some(env_vars)
}

/// Build the `code_execution` JSON pass-through from a workspace's run config.
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

/// Build a `ProvisionRequest` from database state. Every path to a pod (first
/// start, wake, restart, reconciler retry, scale-up) builds its plan here.
///
/// Rejects [`SandboxError::Forbidden`] so a refused pod isn't retried forever;
/// the Closed guard and GitHub probe skip this path — both apply elsewhere.
pub(crate) async fn build_provision_request(
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
    // A Workspace that places none provisions a sandbox that clones nothing.
    let placements = workspaces.placements(&shape.workspace_id).await?;
    let primary = placements
        .first()
        .map(|primary| {
            primary
                .repository
                .clone()
                .map(|repo| (primary.repository_id.clone(), repo))
                .ok_or_else(|| {
                    SandboxError::BadRequest(format!(
                        "Repository '{}' not found in database",
                        primary.repository_id
                    ))
                })
        })
        .transpose()?;

    info!(
        sandbox_id = %sandbox_id,
        repository_id = ?primary.as_ref().map(|(id, _)| id),
        workspace_id = %shape.workspace_id,
        "Auto-provision: sandbox found in DB"
    );

    // 3. Control check, before anything is built or scheduled.
    if let Err(e) = preflight_validate_sandbox_control(users, sandbox_id, &shape).await {
        warn!(
            sandbox_id = %sandbox_id,
            error = %e,
            "Auto-provision: the sandbox's user cannot control it; refusing to provision"
        );
        return Err(SandboxError::Forbidden(e.user_message()));
    }

    let repo = primary.as_ref().map(|(_, repo)| repo);
    let git_provider = repo
        .and_then(|repo| repo.provider)
        .map_or("", |provider| provider.as_str());
    let git_clone_url = repo
        .and_then(|repo| repo.remote.as_ref())
        .map(|remote| remote.url.clone())
        .unwrap_or_default();
    let git_branch = repo.and_then(|repo| shape.branch.clone().or(repo.default_branch.clone()));
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
        primary.as_ref().map_or("", |(id, _)| id.as_str()),
        &git_clone_url,
        git_branch.as_deref(),
        git_provider,
        repo.and_then(|repo| repo.remote.as_ref())
            .is_some_and(|remote| remote.uses_ssh),
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
        (
            minimal_repo_config(repo.map_or(shape.workspace_id.as_str(), |repo| &repo.name)),
            None,
        )
    };

    // The Sandbox row already holds the Loop-over-Workspace resolution made when
    // it was created, so a Loop's placement survives a pod rebuild even if the
    // Workspace changed since. A row written before this column existed is NULL
    // and falls through to the Workspace's own value, set just above.
    if let Some(selector) = shape.node_selector.as_ref() {
        repo_config.node_selector = Some(selector.0.clone());
    }

    // The profile the pod boots its first harness with; a delivery naming
    // another rebuilds it. The transport is fixed for the Sandbox lifetime.
    // Inserted after the workspace env so the Sandbox's selection overrides any
    // stray workspace-level `AGENT_PROFILE`.
    if shape.use_tmux {
        env_vars
            .get_or_insert_with(std::collections::HashMap::new)
            .insert("SANDBOX_USE_TMUX".to_string(), "true".to_string());
    }
    if let Some(agent_profile) = shape.agent_profile.clone() {
        env_vars
            .get_or_insert_with(std::collections::HashMap::new)
            .insert("AGENT_PROFILE".to_string(), agent_profile);
    }

    // Resolve the plugins the identity this pod acts as has turned on, as the
    // Workspace it runs in narrows them. A sandbox with no identifiable run-as
    // identity gets the global defaults and nothing personal — the column is
    // polymorphic text, so a sentinel like `system` or a `bot:` value is nobody
    // rather than an error.
    let run_as = shape
        .run_as_user_id
        .as_deref()
        .and_then(|id| uuid::Uuid::parse_str(id).ok());
    match workspaces
        .plugin_deliveries(run_as, &shape.workspace_id)
        .await
    {
        Ok(deliveries) => {
            if !deliveries.picked.is_empty() {
                info!(
                    sandbox_id = %sandbox_id,
                    count = deliveries.picked.len(),
                    "Auto-provision: resolved plugins"
                );
                repo_config.plugins = Some(deliveries.picked);
            }
            if !deliveries.removed.is_empty() {
                repo_config.removed_plugins = Some(deliveries.removed);
            }
        }
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

    // A headless sandbox has no one watching for a prompt, so the sandbox drops
    // the tools that park a turn on a person's answer.
    if shape.headless {
        env_vars
            .get_or_insert_with(std::collections::HashMap::new)
            .insert("SANDBOX_HEADLESS".to_string(), "true".to_string());
    }

    let plan = moatless_sandbox_runtime::provisioning::ProvisionPlan {
        repo_config,
        git_clone_url,
        git_branch,
        git_provider: git_provider.to_string(),
        env_vars,
        repos: repo_checkouts,
    };

    Ok(ProvisionRequest {
        plan,
        user_id,
        token_generation: shape.token_generation,
    })
}

/// A workspace repository placement with the git-pointer fields of its
/// repository row already resolved.
///
/// Plain owned data, so checkout assembly ([`build_repo_checkouts`]) stays a
/// pure, unit-testable function without DB fakes.
#[derive(Debug, Clone)]
struct ResolvedPlacement {
    repository_id: String,
    clone_url: String,
    uses_ssh: bool,
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
/// A placement with a missing repository row or no remote URL is dropped with
/// a warning rather than failing the whole provision.
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
        let clone_url = repo
            .remote
            .as_ref()
            .map(|remote| remote.url.clone())
            .unwrap_or_default();
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
            uses_ssh: repo.remote.as_ref().is_some_and(|remote| remote.uses_ssh),
            provider: repo
                .provider
                .map_or("", |provider| provider.as_str())
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
/// The primary claims its mount name first, sanitized and de-duplicated, so
/// an existing sandbox volume's checkout directory never moves.
fn build_repo_checkouts(
    placements: &[ResolvedPlacement],
    primary_repository_id: &str,
    primary_clone_url: &str,
    primary_branch: Option<&str>,
    primary_provider: &str,
    primary_uses_ssh: bool,
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
            uses_ssh: primary_uses_ssh,
            branch: primary_branch.map(str::to_string),
            repository_id: primary_repository_id.to_string(),
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
            uses_ssh: placement.uses_ssh,
            branch: placement.branch.clone(),
            repository_id: placement.repository_id.clone(),
            provider: placement.provider.clone(),
            mount_name,
            is_primary: false,
        });
    }

    checkouts
}

/// Suffix a mount name until it no longer collides with an already-claimed one.
///
/// Two placements can derive the same name (two repos both called `api`, or one
/// repo placed twice on different branches); the DB's uniqueness index only
/// covers explicitly-set names.
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
/// workspace. Provisioning then falls back to global defaults.
fn minimal_repo_config(name: &str) -> RepoConfig {
    RepoConfig {
        name: name.to_string(),
        docker_image: None,
        system_prompt: None,
        code_execution: None,
        resources: None,
        plugins: None,
        removed_plugins: None,
        language_servers: None,
        docker_access: None,
        docker_networks: None,
        sandbox_service_account: None,
        network_profile: None,
        node_selector: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkouts_keep_declared_providers_when_host_identity_is_unresolved() {
        use moatless_types::access_control::Scope;
        use moatless_workspace_types::repository::{
            Repository, RepositoryKind, RepositoryProvider, RepositoryRemote,
        };

        for (provider, expected) in [(Some(RepositoryProvider::Github), "github"), (None, "")] {
            let placements = resolve_placements(
                &[RepoPlacement {
                    repository_id: "repo".to_string(),
                    branch: None,
                    mount_name: Some("repo".to_string()),
                    is_primary: false,
                    position: 1,
                    repository: Some(Repository {
                        id: "repo".to_string(),
                        name: "repo".to_string(),
                        kind: RepositoryKind::Git,
                        remote: Some(RepositoryRemote {
                            url: "ssh://git@github.com/acme/repo.git".to_string(),
                            uses_ssh: true,
                            host_repo: None,
                        }),
                        default_branch: None,
                        description: None,
                        provider,
                        scope: Scope::Global,
                        owner_user_id: None,
                        created_by: None,
                        deleted: false,
                        created_at: None,
                        updated_at: None,
                    }),
                }],
                "sandbox",
                "workspace",
            );
            let checkouts = build_repo_checkouts(&placements, "primary", "", None, "", false);
            assert_eq!(checkouts.len(), 1);
            assert_eq!(checkouts[0].provider, expected);
            assert!(checkouts[0].uses_ssh);
        }
    }

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
            uses_ssh: false,
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
            resources: None,
            language_servers: None,
            network_profile: None,
            node_selector: None,
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
            false,
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
            false,
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
            false,
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
            false,
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
            false,
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
            false,
        );
        assert_eq!(checkouts.len(), 2);
        assert_eq!(checkouts[1].mount_name, "api-release");
        assert_eq!(checkouts[1].branch.as_deref(), Some("release"));
    }

    /// No placements readable (no workspace, or a DB error) still provisions
    /// the primary repo.
    #[test]
    fn no_placements_still_yields_the_primary_checkout() {
        let checkouts = build_repo_checkouts(
            &[],
            "repo-a",
            "https://github.com/acme/api.git",
            Some("main"),
            "github",
            false,
        );
        assert_eq!(checkouts.len(), 1);
        assert_eq!(checkouts[0].mount_name, "api");
        assert!(checkouts[0].is_primary);
    }

    /// A blank workspace (no repo at all) provisions a sandbox with nothing to
    /// clone rather than a checkout with an empty URL.
    #[test]
    fn an_empty_primary_url_yields_no_checkouts() {
        let checkouts = build_repo_checkouts(&[], "repo-a", "", None, "github", false);
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
            false,
        );
        let mounts: Vec<&str> = checkouts.iter().map(|c| c.mount_name.as_str()).collect();
        assert_eq!(mounts, vec!["api", "web"]);
    }
}
