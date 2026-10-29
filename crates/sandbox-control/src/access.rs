//! Sandbox admission and resource-access policy.

use super::{
    lifecycle::{Lifecycle, TaskIntent},
    state::SandboxApiState,
};
use moatless_sandbox_runtime::error::SandboxError;
use moatless_types::{access_control::ScopeContext, task::Visibility};

/// Load the Task's Sandbox membership and require *read* access to it.
///
/// Returns `NotFound` on denial, so another tenant's Sandbox never leaks its
/// existence.
pub(super) async fn authorize_task_read(
    state: &SandboxApiState,
    scope: &Option<ScopeContext>,
    task_id: &str,
) -> Result<TaskIntent, SandboxError> {
    let intent = require_lifecycle(state)?.intent(task_id).await?;
    ensure_can_read(scope, &intent, task_id)?;
    Ok(intent)
}

/// Like [`authorize_task_read`], but additionally requires *write* access:
/// ownership, or the public visibility the Task grants.
///
/// Guards every action that operates the Sandbox: provision, stop, restart,
/// redeploy, write a file.
pub(super) async fn authorize_task_write(
    state: &SandboxApiState,
    scope: &Option<ScopeContext>,
    task_id: &str,
) -> Result<TaskIntent, SandboxError> {
    let intent = authorize_task_read(state, scope, task_id).await?;
    ensure_can_write(scope, &intent, task_id)?;
    Ok(intent)
}

/// Like [`authorize_task_read`], but additionally requires the caller to own
/// the Task: owner or platform admin, never a public visitor.
///
/// Guards actions that outlive the pod, such as removing the Sandbox.
pub(super) async fn authorize_task_owner(
    state: &SandboxApiState,
    scope: &Option<ScopeContext>,
    task_id: &str,
) -> Result<TaskIntent, SandboxError> {
    let intent = authorize_task_read(state, scope, task_id).await?;
    ensure_is_owner(scope, &intent, task_id)?;
    Ok(intent)
}

/// Read access to a Sandbox: its own scope/owner pair, or the public visibility
/// its Task grants.
///
/// Refuses as `NotFound`, never `Forbidden`, so a caller can't tell "not yours"
/// from "does not exist"; a missing identity fails closed as `Forbidden`.
fn ensure_can_read(
    scope: &Option<ScopeContext>,
    intent: &TaskIntent,
    task_id: &str,
) -> Result<(), SandboxError> {
    let ctx = scope
        .as_ref()
        .ok_or_else(|| SandboxError::Forbidden("authentication required".into()))?;
    if intent.visibility == Visibility::Public || ctx.can_read(&intent.scoped_resource()) {
        return Ok(());
    }
    Err(SandboxError::NotFound(format!("task {task_id}")))
}

/// Write access to a Sandbox: ownership, or the public visibility its Task
/// grants.
fn ensure_can_write(
    scope: &Option<ScopeContext>,
    intent: &TaskIntent,
    task_id: &str,
) -> Result<(), SandboxError> {
    let ctx = scope
        .as_ref()
        .ok_or_else(|| SandboxError::Forbidden("authentication required".into()))?;
    if ctx.can_write(&intent.scoped_resource(), Some(intent.visibility)) {
        return Ok(());
    }
    Err(SandboxError::Forbidden(format!(
        "no write access to task {task_id}"
    )))
}

/// Ownership of a Sandbox's Task. Visibility never grants it.
fn ensure_is_owner(
    scope: &Option<ScopeContext>,
    intent: &TaskIntent,
    task_id: &str,
) -> Result<(), SandboxError> {
    let ctx = scope
        .as_ref()
        .ok_or_else(|| SandboxError::Forbidden("authentication required".into()))?;
    if ctx.is_owner(&intent.scoped_resource()) {
        return Ok(());
    }
    Err(SandboxError::Forbidden(format!(
        "only the owner of task {task_id} may do this"
    )))
}

// =============================================================================
// Private helpers
// =============================================================================

/// The shared lifecycle handle, or a clean "unavailable" error in the
/// no-database mode where the lifecycle handlers cannot operate.
pub(super) fn require_lifecycle(state: &SandboxApiState) -> Result<&Lifecycle, SandboxError> {
    state
        .lifecycle
        .as_deref()
        .ok_or_else(|| SandboxError::Internal("Sandbox lifecycle unavailable (no database)".into()))
}

#[cfg(test)]
mod authz_tests {
    //! Unit tests for the per-sandbox authorization gate. The policy itself
    //! lives in `ScopeContext`; these exercise how the handlers compose read +
    //! write + ownership over one `TaskIntent`, and which refusal each case earns.
    use super::*;

    use moatless_types::access_control::Scope;
    use moatless_types::sandbox::ArchiveRetention;
    use moatless_types::sandbox::SandboxDesiredState;
    use moatless_types::task::TaskStatus;

    /// Deterministic uuid for a readable test name: distinct names map to
    /// distinct ids, equal names to equal ids.
    fn uid(name: &str) -> uuid::Uuid {
        uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, name.as_bytes())
    }

    /// A `TaskIntent` carrying the given access-control pair; every other field
    /// is an inert default the access check never reads.
    fn intent(scope: Scope, owner_user_id: Option<&str>, visibility: Visibility) -> TaskIntent {
        TaskIntent {
            status: TaskStatus::Open,
            visibility,
            scope,
            owner_user_id: owner_user_id.map(|u| uid(u).to_string()),
            sandbox_id: "task-1".to_string(),
            workspace_id: "ws-1".to_string(),
            headless: false,
            desired_state: SandboxDesiredState::Stopped,
            stop_reason: None,
            retention: None::<ArchiveRetention>,
            marker: None,
            desired_state_updated_at: chrono::Utc::now(),
        }
    }

    fn private(owner_user_id: Option<&str>) -> TaskIntent {
        intent(Scope::User, owner_user_id, Visibility::Private)
    }

    fn ctx(user_id: &str, is_admin: bool) -> ScopeContext {
        ScopeContext::new(uid(user_id).to_string(), is_admin)
    }

    #[test]
    fn read_by_another_tenant_is_not_found() {
        // NotFound, not Forbidden: the refusal never leaks that the sandbox exists.
        let err = ensure_can_read(
            &Some(ctx("intruder", false)),
            &private(Some("owner")),
            "task-1",
        )
        .unwrap_err();
        assert!(matches!(err, SandboxError::NotFound(_)));
    }

    #[test]
    fn read_by_the_owner_is_allowed() {
        assert!(
            ensure_can_read(
                &Some(ctx("owner", false)),
                &private(Some("owner")),
                "task-1"
            )
            .is_ok()
        );
    }

    #[test]
    fn read_of_a_public_task_is_allowed() {
        let public = intent(Scope::User, Some("owner"), Visibility::Public);
        assert!(ensure_can_read(&Some(ctx("stranger", false)), &public, "task-1").is_ok());
    }

    #[test]
    fn write_of_a_public_task_is_allowed_for_any_signed_in_caller() {
        let public = intent(Scope::User, Some("owner"), Visibility::Public);
        assert!(ensure_can_write(&Some(ctx("stranger", false)), &public, "task-1").is_ok());
    }

    #[test]
    fn write_of_a_private_task_by_a_stranger_is_forbidden() {
        // Write widens by visibility only. A private Task is unchanged.
        assert!(matches!(
            ensure_can_write(
                &Some(ctx("stranger", false)),
                &private(Some("owner")),
                "task-1"
            ),
            Err(SandboxError::Forbidden(_))
        ));
    }

    #[test]
    fn write_by_the_owner_and_admin_is_allowed() {
        assert!(
            ensure_can_write(
                &Some(ctx("owner", false)),
                &private(Some("owner")),
                "task-1"
            )
            .is_ok()
        );
        assert!(
            ensure_can_write(
                &Some(ctx("root", true)),
                &private(Some("other-user")),
                "task-1"
            )
            .is_ok()
        );
    }

    #[test]
    fn write_without_identity_is_forbidden() {
        // "Every signed-in user" is the grant — an unauthenticated caller is
        // still refused, even on a public Task.
        let public = intent(Scope::User, Some("owner"), Visibility::Public);
        assert!(matches!(
            ensure_can_write(&None, &public, "task-1"),
            Err(SandboxError::Forbidden(_))
        ));
    }

    #[test]
    fn write_never_widens_to_ownership() {
        // Removing the Sandbox deletes its archives, which outlives anything a
        // visitor is doing, so it stays with the owner.
        let public = intent(Scope::User, Some("owner"), Visibility::Public);
        assert!(ensure_can_write(&Some(ctx("stranger", false)), &public, "task-1").is_ok());
        assert!(matches!(
            ensure_is_owner(&Some(ctx("stranger", false)), &public, "task-1"),
            Err(SandboxError::Forbidden(_))
        ));
    }

    #[test]
    fn read_of_a_global_sandbox_is_allowed() {
        let global = intent(Scope::Global, None, Visibility::Private);
        assert!(ensure_can_read(&Some(ctx("anyone", false)), &global, "task-1").is_ok());
    }

    #[test]
    fn read_without_identity_is_forbidden() {
        // No identity means the request never passed auth_middleware — fail
        // closed rather than resolving the sandbox under a system context.
        assert!(matches!(
            ensure_can_read(&None, &private(Some("someone")), "task-1"),
            Err(SandboxError::Forbidden(_))
        ));
    }

    #[test]
    fn read_of_an_ownerless_sandbox_is_admin_only() {
        // No Sandbox row leaves the pair ownerless. An admin still reaches it;
        // nobody else does.
        let ownerless = private(None);
        assert!(matches!(
            ensure_can_read(&Some(ctx("anyone", false)), &ownerless, "task-1"),
            Err(SandboxError::NotFound(_))
        ));
        assert!(ensure_can_read(&Some(ctx("root", true)), &ownerless, "task-1").is_ok());
    }

    #[test]
    fn ownership_by_the_owner_is_allowed() {
        assert!(
            ensure_is_owner(
                &Some(ctx("owner", false)),
                &private(Some("owner")),
                "task-1"
            )
            .is_ok()
        );
    }

    #[test]
    fn ownership_by_a_read_only_viewer_is_forbidden() {
        // Read access never implies ownership: a global sandbox is admin-owned.
        let global = intent(Scope::Global, None, Visibility::Private);
        assert!(ensure_can_read(&Some(ctx("viewer", false)), &global, "task-1").is_ok());
        assert!(matches!(
            ensure_is_owner(&Some(ctx("viewer", false)), &global, "task-1"),
            Err(SandboxError::Forbidden(_))
        ));
    }

    #[test]
    fn ownership_by_a_stranger_is_forbidden() {
        let err = ensure_is_owner(
            &Some(ctx("stranger", false)),
            &private(Some("owner")),
            "task-1",
        )
        .unwrap_err();
        assert!(matches!(err, SandboxError::Forbidden(_)));
    }

    #[test]
    fn ownership_by_an_admin_bypasses_scope() {
        assert!(
            ensure_is_owner(
                &Some(ctx("root", true)),
                &private(Some("other-user")),
                "task-1"
            )
            .is_ok()
        );
    }

    #[test]
    fn ownership_without_identity_is_forbidden() {
        assert!(matches!(
            ensure_is_owner(&None, &private(Some("someone")), "task-1"),
            Err(SandboxError::Forbidden(_))
        ));
    }
}
