use super::dao::WorkspaceDao;
use super::error::WorkspaceError;
use super::models::{UpdateOutcome, WorkspacePatch};
use super::models::{Workspace, WorkspaceKind, WorkspaceRepo, WorkspaceSource};
use super::schemas::{
    AddRepoRequest, CreateWorkspaceRequest, UpdateWorkspaceRequest, WorkspaceRepoInput,
    resource_config_from_wire, script_configs_from_wire, server_configs_from_wire,
};
use crate::repository::dao::RepositoryDao;
use crate::repository::error::RepositoryError;
use crate::repository::mutations::promote_repositories_to_global;
use moatless_ports::identity::UserResolver;
use moatless_types::access_control::{Scope, ScopeContext, resolve_scope_for_create};

/// Whether this Workspace takes a write at all.
///
/// The subject is the Workspace's provenance, not the field: a git-synced one
/// refuses the whole `PATCH`, even fields `GitSyncService` never reconciles
/// (`run_as_user_id`, `icon`, `default_model`, `default_effort`) — editing
/// those is `override_workspace`, the write that stops it being git's.
fn ensure_workspace_mutable(ws: &Workspace) -> Result<(), WorkspaceError> {
    if ws.deleted {
        return Err(WorkspaceError::Conflict(
            "Workspace has been deleted; reset it from Git before editing it".to_string(),
        ));
    }
    if ws.source == WorkspaceSource::Git {
        return Err(WorkspaceError::Conflict(
            "Workspace is synced from Git and is read-only; edit .moatless/workspaces.json to change it".to_string(),
        ));
    }
    Ok(())
}

/// A repository the scope cascade could not promote fails the Workspace's move.
///
/// The caller acts on the repository, so its own message travels: a remote URL
/// already registered globally is a 409 they can resolve, not an internal
/// error.
fn scope_cascade_error(error: RepositoryError) -> WorkspaceError {
    match error {
        RepositoryError::Db(error) => WorkspaceError::Db(error),
        // `Store` is internal on the repository side too, and its own
        // `IntoResponse` redacts it. Kept out of the arm below so it is not the
        // one variant that reaches the client verbatim.
        error @ RepositoryError::Store(_) => {
            WorkspaceError::Db(sqlx::Error::Protocol(error.to_string()))
        }
        other => WorkspaceError::Conflict(format!(
            "Cannot move this workspace to global scope: {other}"
        )),
    }
}

fn kind_for(count: usize) -> WorkspaceKind {
    match count {
        0 => WorkspaceKind::Blank,
        1 => WorkspaceKind::Single,
        _ => WorkspaceKind::Multi,
    }
}

async fn ensure_repos_visible(
    repositories: &dyn RepositoryDao,
    ctx: &ScopeContext,
    placements: &[WorkspaceRepoInput],
) -> Result<(), WorkspaceError> {
    for p in placements {
        if repositories
            .find_by_id(&p.repository_id, ctx)
            .await?
            .is_none()
        {
            return Err(WorkspaceError::BadRequest(format!(
                "Repository '{}' not found or not accessible",
                p.repository_id
            )));
        }
    }
    Ok(())
}

/// Refuses the run-config fields that place a sandbox outside its default
/// posture, to anyone but an admin.
///
/// An empty `node_selector` clears the field back to the deployment default, so
/// it is not an elevation.
fn ensure_privileged_sandbox_fields(
    ctx: &ScopeContext,
    sandbox_service_account: Option<&String>,
    network_profile: Option<&String>,
    node_selector: Option<&std::collections::BTreeMap<String, String>>,
) -> Result<(), WorkspaceError> {
    if ctx.is_admin {
        return Ok(());
    }
    let mut attempted = Vec::new();
    if sandbox_service_account.is_some() {
        attempted.push("sandboxServiceAccount");
    }
    if network_profile.is_some() {
        attempted.push("networkProfile");
    }
    if node_selector.is_some_and(|selector| !selector.is_empty()) {
        attempted.push("nodeSelector");
    }
    if attempted.is_empty() {
        return Ok(());
    }
    Err(WorkspaceError::Forbidden(format!(
        "{} may only be set by an admin — these fields grant a sandbox \
         elevated Kubernetes access or dedicated capacity",
        attempted.join(", ")
    )))
}

async fn resolve_run_as_user(
    ctx: &ScopeContext,
    users: &dyn UserResolver,
    requested: Option<&String>,
) -> Result<Option<Option<String>>, WorkspaceError> {
    let Some(requested) = requested else {
        return Ok(None);
    };
    if !ctx.is_admin {
        return Err(WorkspaceError::Forbidden(
            "runAsUserId may only be set by an admin — it makes every task on this \
             workspace act with another account's git-host credential"
                .to_string(),
        ));
    }
    let requested = requested.trim();
    if requested.is_empty() {
        return Ok(Some(None));
    }
    users
        .canonical_id(requested)
        .await
        .map_err(|e| WorkspaceError::Db(sqlx::Error::Protocol(e.to_string())))?
        .map(|id| Some(Some(id.to_string())))
        .ok_or_else(|| {
            WorkspaceError::BadRequest(format!("Run-as user {requested} does not exist"))
        })
}

fn ensure_unique_mount_names(placements: &[WorkspaceRepoInput]) -> Result<(), WorkspaceError> {
    let mut seen = std::collections::HashSet::new();
    for p in placements {
        if let Some(mount) = p.mount_name.as_deref() {
            if !seen.insert(mount) {
                return Err(WorkspaceError::BadRequest(format!(
                    "Duplicate mount name '{mount}' in workspace"
                )));
            }
        }
    }
    Ok(())
}

async fn ensure_writable(
    workspaces: &dyn WorkspaceDao,
    ctx: &ScopeContext,
    workspace_id: &str,
) -> Result<(), WorkspaceError> {
    let existing = workspaces
        .find_by_id(workspace_id, ctx)
        .await?
        .ok_or_else(|| WorkspaceError::NotFound(workspace_id.to_string()))?;
    ensure_workspace_mutable(&existing)?;
    // Control is enforced in the DAO via write_filter; the read above just
    // provides an early NotFound / git-sync guard.
    Ok(())
}

/// Apply the caller-authorized workspace mutation.
pub async fn create_workspace(
    workspaces: &dyn WorkspaceDao,
    repositories: &dyn RepositoryDao,
    users: &dyn UserResolver,
    ctx: &ScopeContext,
    req: CreateWorkspaceRequest,
) -> Result<String, WorkspaceError> {
    ensure_unique_mount_names(&req.repos)?;
    ensure_repos_visible(repositories, ctx, &req.repos).await?;
    ensure_privileged_sandbox_fields(
        ctx,
        req.sandbox_service_account.as_ref(),
        req.network_profile.as_ref(),
        req.node_selector.as_ref(),
    )?;
    let run_as_user_id = resolve_run_as_user(ctx, users, req.run_as_user_id.as_ref())
        .await?
        .flatten();

    let resolved = resolve_scope_for_create(ctx, req.scope)?;

    let workspace_id = format!("ws_{}", uuid::Uuid::new_v4());
    let row = Workspace {
        id: workspace_id.clone(),
        name: req.name,
        description: req.description,
        icon: req.icon,
        kind: kind_for(req.repos.len()),
        docker_image: req.docker_image,
        docker_access: req.docker_access,
        sandbox_service_account: req.sandbox_service_account,
        system_prompt: req.system_prompt,
        setup_commands: req.setup_commands,
        env_vars: req.env_vars,
        servers: req.servers.map(server_configs_from_wire).transpose()?,
        scripts: req.scripts.map(script_configs_from_wire),
        resources: req.resources.map(resource_config_from_wire).transpose()?,
        network_profile: req.network_profile,
        node_selector: req.node_selector,
        default_model: req.default_model,
        default_effort: req.default_effort,
        source: WorkspaceSource::Manual,
        source_repository_id: None,
        source_key: None,
        source_config_path: None,
        scope: resolved.scope,
        owner_user_id: resolved.owner_user_id,
        run_as_user_id,
        created_by: Some(ctx.user_id.clone()),
        deleted: false,
        created_at: None,
        updated_at: None,
    };
    workspaces.insert_workspace(&row).await?;

    // Insert placements non-primary, then elect exactly one primary so the
    // partial unique index never sees two primaries mid-insert.
    let mut placement_ids: Vec<String> = Vec::with_capacity(req.repos.len());
    let mut primary_choice: Option<String> = None;
    for (i, p) in req.repos.iter().enumerate() {
        let placement_id = format!("wsr_{}", uuid::Uuid::new_v4());
        workspaces
            .add_repo(
                &WorkspaceRepo {
                    id: placement_id.clone(),
                    workspace_id: workspace_id.clone(),
                    repository_id: p.repository_id.clone(),
                    branch: p.branch.clone(),
                    mount_name: p.mount_name.clone(),
                    is_primary: false,
                    position: p.position.unwrap_or(i as i32),
                    created_at: None,
                },
                ctx,
            )
            .await?;
        if primary_choice.is_none() && p.is_primary == Some(true) {
            primary_choice = Some(placement_id.clone());
        }
        placement_ids.push(placement_id);
    }
    // Default the first placement to primary when the caller flagged none.
    if let Some(target) = primary_choice.or_else(|| placement_ids.first().cloned()) {
        workspaces.set_primary(&workspace_id, &target, ctx).await?;
    }

    Ok(workspace_id)
}

/// Apply the caller-authorized workspace mutation.
pub async fn update_workspace(
    workspaces: &dyn WorkspaceDao,
    repositories: &dyn RepositoryDao,
    users: &dyn UserResolver,
    ctx: &ScopeContext,
    workspace_id: String,
    req: UpdateWorkspaceRequest,
) -> Result<(), WorkspaceError> {
    let existing = workspaces
        .find_by_id(&workspace_id, ctx)
        .await?
        .ok_or_else(|| WorkspaceError::NotFound(workspace_id.clone()))?;
    ensure_workspace_mutable(&existing)?;
    ensure_privileged_sandbox_fields(
        ctx,
        req.sandbox_service_account.as_ref(),
        req.network_profile.as_ref(),
        req.node_selector.as_ref(),
    )?;
    let run_as_user_id = resolve_run_as_user(ctx, users, req.run_as_user_id.as_ref()).await?;

    // Re-resolve owner_user_id when the scope changes.
    let (scope, owner_user_id) = match req.scope {
        Some(new_scope) => {
            let resolved = resolve_scope_for_create(ctx, Some(new_scope))?;
            (Some(resolved.scope), Some(resolved.owner_user_id))
        }
        None => (None, None),
    };

    // A Workspace that becomes global is readable by everyone, so the
    // repositories placed in it move with it — otherwise the Workspace lists
    // repositories its new readers cannot open. Before `workspaces.update`, so a
    // repository that cannot be promoted leaves the Workspace where it was.
    if scope == Some(Scope::Global) {
        let repository_ids: Vec<String> = workspaces
            .find_repos(&workspace_id)
            .await?
            .into_iter()
            .map(|placement| placement.repository_id)
            .collect();
        promote_repositories_to_global(repositories, ctx, &repository_ids)
            .await
            .map_err(scope_cascade_error)?;
    }

    let patch = WorkspacePatch {
        name: req.name,
        description: req.description.map(Some),
        scope,
        owner_user_id,
        docker_image: req.docker_image.map(Some),
        docker_access: req.docker_access.map(Some),
        sandbox_service_account: req.sandbox_service_account.map(Some),
        system_prompt: req.system_prompt.map(Some),
        setup_commands: req.setup_commands.map(Some),
        env_vars: req.env_vars.map(Some),
        servers: req
            .servers
            .map(|configs| server_configs_from_wire(configs).map(Some))
            .transpose()?,
        scripts: req
            .scripts
            .map(|configs| Some(script_configs_from_wire(configs))),
        resources: req
            .resources
            .map(|config| resource_config_from_wire(config).map(Some))
            .transpose()?,
        network_profile: req.network_profile.map(Some),
        node_selector: req.node_selector.map(Some),
        icon: req.icon,
        default_model: req.default_model,
        default_effort: req.default_effort,
        run_as_user_id,
        kind: None,
        // Provenance not editable via the write API.
        source: None,
        source_repository_id: None,
        source_key: None,
        source_config_path: None,
    };
    if workspaces.update(&workspace_id, &patch, ctx).await? == UpdateOutcome::NoMatch {
        return Err(WorkspaceError::NotFound(workspace_id));
    }

    Ok(())
}

/// Apply the caller-authorized workspace mutation.
pub async fn override_workspace(
    workspaces: &dyn WorkspaceDao,
    users: &dyn UserResolver,
    ctx: &ScopeContext,
    workspace_id: String,
    req: UpdateWorkspaceRequest,
) -> Result<(), WorkspaceError> {
    let existing = workspaces
        .find_by_id(&workspace_id, ctx)
        .await?
        .ok_or_else(|| WorkspaceError::NotFound(workspace_id.clone()))?;
    if existing.deleted {
        return Err(WorkspaceError::Conflict(
            "Workspace has been deleted; reset it from Git before overriding it".to_string(),
        ));
    }
    // Intentionally skipping `ensure_workspace_mutable` — this endpoint is the
    // explicit way to take ownership of a git-synced workspace. The DAO
    // write_filter ensures only the owner/admin can do this.
    //
    // The privileged-field gate is NOT skipped: taking ownership of a workspace
    // is not the same as being allowed to elevate it.
    ensure_privileged_sandbox_fields(
        ctx,
        req.sandbox_service_account.as_ref(),
        req.network_profile.as_ref(),
        req.node_selector.as_ref(),
    )?;
    let run_as_user_id = resolve_run_as_user(ctx, users, req.run_as_user_id.as_ref()).await?;

    let patch = WorkspacePatch {
        name: req.name,
        description: req.description.map(Some),
        docker_image: req.docker_image.map(Some),
        docker_access: req.docker_access.map(Some),
        sandbox_service_account: req.sandbox_service_account.map(Some),
        system_prompt: req.system_prompt.map(Some),
        setup_commands: req.setup_commands.map(Some),
        env_vars: req.env_vars.map(Some),
        servers: req
            .servers
            .map(|configs| server_configs_from_wire(configs).map(Some))
            .transpose()?,
        scripts: req
            .scripts
            .map(|configs| Some(script_configs_from_wire(configs))),
        resources: req
            .resources
            .map(|config| resource_config_from_wire(config).map(Some))
            .transpose()?,
        network_profile: req.network_profile.map(Some),
        node_selector: req.node_selector.map(Some),
        icon: req.icon,
        default_model: req.default_model,
        default_effort: req.default_effort,
        run_as_user_id,
        // Mark as manually managed; provenance columns stay unchanged.
        source: Some(WorkspaceSource::Manual),
        kind: None,
        scope: None,
        owner_user_id: None,
        source_repository_id: None,
        source_key: None,
        source_config_path: None,
    };
    if workspaces.update(&workspace_id, &patch, ctx).await? == UpdateOutcome::NoMatch {
        return Err(WorkspaceError::NotFound(workspace_id));
    }

    Ok(())
}

/// Apply the caller-authorized workspace mutation.
pub async fn delete_workspace(
    workspaces: &dyn WorkspaceDao,
    ctx: &ScopeContext,
    workspace_id: String,
) -> Result<(), WorkspaceError> {
    let existing = workspaces
        .find_by_id(&workspace_id, ctx)
        .await?
        .ok_or_else(|| WorkspaceError::NotFound(workspace_id.clone()))?;

    let deleted = if existing.source_repository_id.is_some() && existing.source_key.is_some() {
        workspaces
            .soft_delete_git_origin(&workspace_id, ctx)
            .await?
    } else {
        ensure_workspace_mutable(&existing)?;
        workspaces.soft_delete(&workspace_id, ctx).await?
    };
    if !deleted {
        return Err(WorkspaceError::NotFound(workspace_id));
    }
    Ok(())
}

/// Apply the caller-authorized workspace mutation.
pub async fn add_workspace_repo(
    workspaces: &dyn WorkspaceDao,
    repositories: &dyn RepositoryDao,
    ctx: &ScopeContext,
    workspace_id: String,
    req: AddRepoRequest,
) -> Result<(), WorkspaceError> {
    ensure_writable(workspaces, ctx, &workspace_id).await?;

    if repositories
        .find_by_id(&req.repository_id, ctx)
        .await?
        .is_none()
    {
        return Err(WorkspaceError::BadRequest(format!(
            "Repository '{}' not found or not accessible",
            req.repository_id
        )));
    }

    let existing = workspaces.find_repos(&workspace_id).await?;
    if let Some(mount) = req.mount_name.as_deref() {
        if existing
            .iter()
            .any(|r| r.mount_name.as_deref() == Some(mount))
        {
            return Err(WorkspaceError::BadRequest(format!(
                "Duplicate mount name '{mount}' in workspace"
            )));
        }
    }

    let placement_id = format!("wsr_{}", uuid::Uuid::new_v4());
    let position = req.position.unwrap_or(existing.len() as i32);
    workspaces
        .add_repo(
            &WorkspaceRepo {
                id: placement_id.clone(),
                workspace_id: workspace_id.clone(),
                repository_id: req.repository_id,
                branch: req.branch,
                mount_name: req.mount_name,
                is_primary: false,
                position,
                created_at: None,
            },
            ctx,
        )
        .await?;

    // First repo in a previously-blank workspace, or an explicit request, makes
    // this placement primary.
    if req.is_primary == Some(true) || existing.is_empty() {
        workspaces
            .set_primary(&workspace_id, &placement_id, ctx)
            .await?;
    }
    workspaces.refresh_kind(&workspace_id, ctx).await?;

    Ok(())
}

/// Apply the caller-authorized workspace mutation.
pub async fn remove_workspace_repo(
    workspaces: &dyn WorkspaceDao,
    ctx: &ScopeContext,
    workspace_id: String,
    placement_id: String,
) -> Result<(), WorkspaceError> {
    ensure_writable(workspaces, ctx, &workspace_id).await?;

    let removed = workspaces
        .remove_repo(&workspace_id, &placement_id, ctx)
        .await?;
    if !removed {
        return Err(WorkspaceError::NotFound(placement_id));
    }
    workspaces.refresh_kind(&workspace_id, ctx).await?;

    // Preserve the "exactly one primary when n>=1" invariant if we removed the
    // primary: promote the next-best remaining placement.
    let repos = workspaces.find_repos(&workspace_id).await?;
    if !repos.is_empty() && !repos.iter().any(|r| r.is_primary) {
        workspaces
            .set_primary(&workspace_id, &repos[0].id, ctx)
            .await?;
    }

    Ok(())
}

/// Apply the caller-authorized workspace mutation.
pub async fn set_workspace_primary_repo(
    workspaces: &dyn WorkspaceDao,
    ctx: &ScopeContext,
    workspace_id: String,
    placement_id: String,
) -> Result<(), WorkspaceError> {
    ensure_writable(workspaces, ctx, &workspace_id).await?;

    let ok = workspaces
        .set_primary(&workspace_id, &placement_id, ctx)
        .await?;
    if !ok {
        return Err(WorkspaceError::NotFound(placement_id));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::dao::MockWorkspaceDao;
    use super::super::reset::reset_source;
    use super::*;
    use crate::repository::dao::MockRepositoryDao;
    use crate::repository::models::{Repository, RepositoryKind};
    use moatless_ports::identity::MockUserResolver;

    fn input(mount: Option<&str>) -> WorkspaceRepoInput {
        WorkspaceRepoInput {
            repository_id: "repo".to_string(),
            branch: None,
            mount_name: mount.map(str::to_string),
            is_primary: None,
            position: None,
        }
    }

    fn ctx(is_admin: bool) -> ScopeContext {
        ScopeContext {
            user_id: "u1".to_string(),
            is_admin,
        }
    }

    /// The fields that opt a workspace into an elevated `sandboxAccess` profile
    /// or onto dedicated capacity are admin-only, so enabling a profile in Helm
    /// values or standing up a node pool stays an operator decision rather than
    /// becoming self-serve.
    #[test]
    fn privileged_sandbox_fields_are_admin_only() {
        let sa = "moatless-admin".to_string();
        let profile = "moatless-admin".to_string();
        let selector: std::collections::BTreeMap<String, String> =
            [("moatless.io/pool".to_string(), "on-demand".to_string())]
                .into_iter()
                .collect();

        for (account, network, node) in [
            (Some(&sa), None, None),
            (None, Some(&profile), None),
            (None, None, Some(&selector)),
            (Some(&sa), Some(&profile), Some(&selector)),
        ] {
            let err =
                ensure_privileged_sandbox_fields(&ctx(false), account, network, node).unwrap_err();
            assert!(
                matches!(err, WorkspaceError::Forbidden(_)),
                "non-admin must not select a privileged sandbox identity"
            );
            assert!(
                ensure_privileged_sandbox_fields(&ctx(true), account, network, node).is_ok(),
                "admin may select a privileged sandbox identity"
            );
        }
    }

    /// An ordinary edit that never mentions the fields is untouched — otherwise
    /// a non-admin could not rename their own workspace.
    #[test]
    fn absent_privileged_fields_are_allowed_for_anyone() {
        assert!(ensure_privileged_sandbox_fields(&ctx(false), None, None, None).is_ok());
    }

    /// Clearing the selector back to the deployment default is not an
    /// elevation, so it needs no admin.
    #[test]
    fn an_empty_node_selector_is_not_privileged() {
        let empty = std::collections::BTreeMap::new();
        assert!(ensure_privileged_sandbox_fields(&ctx(false), None, None, Some(&empty)).is_ok());
    }

    #[test]
    fn kind_for_derives_label_from_count() {
        assert_eq!(kind_for(0), WorkspaceKind::Blank);
        assert_eq!(kind_for(1), WorkspaceKind::Single);
        assert_eq!(kind_for(2), WorkspaceKind::Multi);
        assert_eq!(kind_for(9), WorkspaceKind::Multi);
    }

    fn workspace_with_source(source: WorkspaceSource) -> Workspace {
        Workspace {
            id: "ws_1".to_string(),
            name: "w".to_string(),
            description: None,
            icon: None,
            default_model: None,
            default_effort: None,
            kind: WorkspaceKind::Single,
            docker_image: None,
            docker_access: None,
            sandbox_service_account: None,
            system_prompt: None,
            setup_commands: None,
            env_vars: None,
            servers: None,
            scripts: None,
            resources: None,
            network_profile: None,
            node_selector: None,
            source,
            source_repository_id: None,
            source_key: None,
            source_config_path: None,
            scope: moatless_types::access_control::Scope::User,
            owner_user_id: None,
            run_as_user_id: None,
            created_by: None,
            deleted: false,
            created_at: None,
            updated_at: None,
        }
    }

    #[test]
    fn git_workspace_is_read_only() {
        let err =
            ensure_workspace_mutable(&workspace_with_source(WorkspaceSource::Git)).unwrap_err();
        assert!(matches!(err, WorkspaceError::Conflict(_)));
    }

    #[test]
    fn manual_workspace_is_mutable() {
        assert!(ensure_workspace_mutable(&workspace_with_source(WorkspaceSource::Manual)).is_ok());
    }

    #[test]
    fn deleted_workspace_is_not_mutable() {
        let mut workspace = workspace_with_source(WorkspaceSource::Manual);
        workspace.deleted = true;

        let err = ensure_workspace_mutable(&workspace).unwrap_err();
        assert!(matches!(err, WorkspaceError::Conflict(_)));
    }

    #[test]
    fn reset_source_requires_full_provenance() {
        // A workspace that was never git-synced has nothing to reset to.
        let err = reset_source(&workspace_with_source(WorkspaceSource::Manual)).unwrap_err();
        assert!(matches!(err, WorkspaceError::BadRequest(_)));

        // A half-stamped row is just as unreachable for the sync.
        let mut half = workspace_with_source(WorkspaceSource::Manual);
        half.source_repository_id = Some("repo-1".to_string());
        assert!(reset_source(&half).is_err());

        let mut overridden = workspace_with_source(WorkspaceSource::Manual);
        overridden.source_repository_id = Some("repo-1".to_string());
        overridden.source_key = Some("api".to_string());
        assert_eq!(reset_source(&overridden).unwrap(), ("repo-1", "api"));
    }

    #[test]
    fn unique_mount_names_accepts_distinct_and_nulls() {
        let placements = vec![
            input(Some("api")),
            input(Some("web")),
            input(None),
            input(None),
        ];
        assert!(ensure_unique_mount_names(&placements).is_ok());
    }

    #[test]
    fn unique_mount_names_rejects_duplicates() {
        let placements = vec![input(Some("api")), input(Some("api"))];
        let err = ensure_unique_mount_names(&placements).unwrap_err();
        assert!(matches!(err, WorkspaceError::BadRequest(_)));
    }

    fn repository(id: &str, scope: Scope) -> Repository {
        Repository {
            id: id.to_string(),
            name: id.to_string(),
            kind: RepositoryKind::Remote,
            remote: Some(format!("https://github.com/acme/{id}")).map(|url| {
                crate::repository::remote::resolve_remote(
                    Some(moatless_workspace_types::repository::RepositoryProvider::Github),
                    url,
                )
            }),
            default_branch: None,
            description: None,
            provider: None,
            scope,
            owner_user_id: (scope == Scope::User).then(|| "u1".to_string()),
            created_by: None,
            deleted: false,
            created_at: None,
            updated_at: None,
        }
    }

    fn placement(repository_id: &str) -> WorkspaceRepo {
        WorkspaceRepo {
            id: format!("wsr_{repository_id}"),
            workspace_id: "ws_1".to_string(),
            repository_id: repository_id.to_string(),
            branch: None,
            mount_name: None,
            is_primary: false,
            position: 0,
            created_at: None,
        }
    }

    fn manual_workspace_dao() -> MockWorkspaceDao {
        let mut workspaces = MockWorkspaceDao::new();
        workspaces
            .expect_find_by_id()
            .returning(|_, _| Ok(Some(workspace_with_source(WorkspaceSource::Manual))));
        workspaces
            .expect_update()
            .returning(|_, _, _| Ok(UpdateOutcome::Applied));
        workspaces
    }

    fn scope_request(scope: Scope) -> UpdateWorkspaceRequest {
        UpdateWorkspaceRequest {
            scope: Some(scope),
            ..Default::default()
        }
    }

    /// A Workspace that becomes global takes its repositories with it, so
    /// everyone who can now read it can open what it places. A repository
    /// already global is left alone — `expect_update` accepts no other id.
    #[tokio::test]
    async fn global_move_promotes_the_repositories_it_places() {
        let mut workspaces = manual_workspace_dao();
        workspaces
            .expect_find_repos()
            .returning(|_| Ok(vec![placement("r_user"), placement("r_global")]));

        let mut repositories = MockRepositoryDao::new();
        repositories.expect_find_by_id().returning(|id, _| {
            let scope = if id == "r_global" {
                Scope::Global
            } else {
                Scope::User
            };
            Ok(Some(repository(id, scope)))
        });
        repositories
            .expect_find_by_normalized_remote_url()
            .returning(|_, _, _| Ok(None));
        repositories
            .expect_update()
            .withf(|id, patch, _| {
                id == "r_user"
                    && patch.scope == Some(Scope::Global)
                    && patch.owner_user_id == Some(None)
            })
            .once()
            .returning(|_, _, _| Ok(true));

        update_workspace(
            &workspaces,
            &repositories,
            &MockUserResolver::new(),
            &ctx(true),
            "ws_1".to_string(),
            scope_request(Scope::Global),
        )
        .await
        .unwrap();
    }

    /// The cascade runs one way only. Both mocks are bare here, so reading the
    /// placements or writing a repository would fail the test.
    #[tokio::test]
    async fn user_move_leaves_the_repositories_global() {
        let workspaces = manual_workspace_dao();
        let repositories = MockRepositoryDao::new();

        update_workspace(
            &workspaces,
            &repositories,
            &MockUserResolver::new(),
            &ctx(true),
            "ws_1".to_string(),
            scope_request(Scope::User),
        )
        .await
        .unwrap();
    }

    /// A caller who may read a workspace but not control it is refused.
    ///
    /// The DAO's scope clause matches no row for them, which used to reach the
    /// handler as success and answer `200` with the workspace unchanged.
    #[tokio::test]
    async fn an_update_outside_the_callers_control_scope_is_refused() {
        let mut workspaces = MockWorkspaceDao::new();
        workspaces
            .expect_find_by_id()
            .returning(|_, _| Ok(Some(workspace_with_source(WorkspaceSource::Manual))));
        workspaces
            .expect_update()
            .returning(|_, _, _| Ok(UpdateOutcome::NoMatch));

        let err = update_workspace(
            &workspaces,
            &MockRepositoryDao::new(),
            &MockUserResolver::new(),
            &ctx(true),
            "ws_1".to_string(),
            scope_request(Scope::User),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, WorkspaceError::NotFound(id) if id == "ws_1"));
    }

    /// A repository whose remote URL is already registered globally blocks the
    /// move, and nothing is written.
    ///
    /// Not the Workspace (`expect_update` never set), and not the repository
    /// checked before the colliding one, whose promotion would drop an owner no
    /// later failure can hand back.
    #[tokio::test]
    async fn a_colliding_repository_blocks_the_global_move() {
        let mut workspaces = MockWorkspaceDao::new();
        workspaces
            .expect_find_by_id()
            .returning(|_, _| Ok(Some(workspace_with_source(WorkspaceSource::Manual))));
        workspaces
            .expect_find_repos()
            .returning(|_| Ok(vec![placement("r_first"), placement("r_colliding")]));

        let mut repositories = MockRepositoryDao::new();
        repositories
            .expect_find_by_id()
            .returning(|id, _| Ok(Some(repository(id, Scope::User))));
        repositories
            .expect_find_by_normalized_remote_url()
            .returning(|url_key, _, _| {
                Ok(url_key
                    .contains("r_colliding")
                    .then(|| repository("r_other", Scope::Global)))
            });
        repositories.expect_update().never();

        let err = update_workspace(
            &workspaces,
            &repositories,
            &MockUserResolver::new(),
            &ctx(true),
            "ws_1".to_string(),
            scope_request(Scope::Global),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, WorkspaceError::Conflict(_)));
    }

    /// One repository placed twice is one row to promote.
    ///
    /// `workspace_repos` has no unique `(workspace_id, repository_id)`, so a
    /// Workspace may place the same repository at two branches, and the move
    /// must not read the second placement as a second claim on its URL.
    #[tokio::test]
    async fn a_repository_placed_twice_is_promoted_once() {
        let mut workspaces = manual_workspace_dao();
        workspaces
            .expect_find_repos()
            .returning(|_| Ok(vec![placement("r_twice"), placement("r_twice")]));

        let mut repositories = MockRepositoryDao::new();
        repositories
            .expect_find_by_id()
            .returning(|id, _| Ok(Some(repository(id, Scope::User))));
        repositories
            .expect_find_by_normalized_remote_url()
            .returning(|_, _, _| Ok(None));
        repositories
            .expect_update()
            .withf(|id, _, _| id == "r_twice")
            .once()
            .returning(|_, _, _| Ok(true));

        update_workspace(
            &workspaces,
            &repositories,
            &MockUserResolver::new(),
            &ctx(true),
            "ws_1".to_string(),
            scope_request(Scope::Global),
        )
        .await
        .unwrap();
    }

    /// Two rows for one upstream URL are legal while each has its own owner,
    /// and illegal once the promotion has dropped both.
    ///
    /// No global row holds the URL here, so only the batch's own claims catch
    /// it — before either write, as `expect_update().never()` asserts.
    #[tokio::test]
    async fn two_placements_of_one_url_block_the_global_move() {
        let mut workspaces = MockWorkspaceDao::new();
        workspaces
            .expect_find_by_id()
            .returning(|_, _| Ok(Some(workspace_with_source(WorkspaceSource::Manual))));
        workspaces
            .expect_find_repos()
            .returning(|_| Ok(vec![placement("r_mine"), placement("r_theirs")]));

        let mut repositories = MockRepositoryDao::new();
        repositories.expect_find_by_id().returning(|id, _| {
            let mut existing = repository(id, Scope::User);
            existing.remote = Some(crate::repository::remote::resolve_remote(
                existing.provider,
                "https://github.com/acme/shared.git".to_string(),
            ));
            existing.owner_user_id = Some(if id == "r_mine" { "u1" } else { "u2" }.to_string());
            Ok(Some(existing))
        });
        repositories
            .expect_find_by_normalized_remote_url()
            .returning(|_, _, _| Ok(None));
        repositories.expect_update().never();

        let err = update_workspace(
            &workspaces,
            &repositories,
            &MockUserResolver::new(),
            &ctx(true),
            "ws_1".to_string(),
            scope_request(Scope::Global),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, WorkspaceError::Conflict(_)));
    }
}
