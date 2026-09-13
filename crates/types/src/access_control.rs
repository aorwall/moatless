//! Canonical access control: which identity acts, and what it may reach.
//!
//! [`ScopeContext`]'s policy is a pure function of `(user_id, is_admin)` ×
//! `(scope, owner, visibility)` with no database input; owner is
//! `COALESCE(owner_user_id, created_by)`, and control includes a shell, so
//! anything a shell reaches is control by construction. See
//! [`ScopeContext::can_read`], [`ScopeContext::can_write`], and
//! [`ScopeContext::can_control`] for the exact rules.

#![cfg_attr(
    test,
    allow(
        clippy::disallowed_methods,
        clippy::unwrap_used,
        reason = "unit tests assert exact policy cases, including the system bypass"
    )
)]

use crate::task::Visibility;

crate::string_enum!(
    /// Access scope for repositories, tasks, integrations, and credentials.
    ///
    /// `global` is world-readable and admin-writable; `user` is owner-managed. A
    /// public Task additionally grants read/interact and control, never write.
    Scope {
        Global => "global",
        User => "user",
    }
);

// ---------------------------------------------------------------------------
// ScopedEntity — declares per-entity schema facts
// ---------------------------------------------------------------------------

/// Declares access-control schema properties for each entity type.
///
/// The entity determines whether `visibility = 'public'` bypasses scope checks
/// — callers never choose this themselves, eliminating a class of bugs.
#[derive(Debug, Clone, Copy)]
pub enum ScopedEntity {
    /// `tasks` table — has a `visibility` column.
    Task,
    /// `repositories` table — canonical scoped resource; no visibility bypass.
    Repository,
    /// Scoped tables without a `visibility` column.
    Resource,
}

// ---------------------------------------------------------------------------
// ScopedResource — canonical persisted shape
// ---------------------------------------------------------------------------

/// Canonical access-control fields for root scoped resources.
///
/// `owner_user_id` carries the effective owner: callers pass
/// `owner_user_id.or(created_by)` to preserve the legacy fallback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopedResource {
    pub scope: Scope,
    pub owner_user_id: Option<String>,
}

impl ScopedResource {
    pub fn new<U>(scope: Scope, owner_user_id: Option<U>) -> Self
    where
        U: Into<String>,
    {
        Self {
            scope,
            owner_user_id: owner_user_id.map(Into::into),
        }
    }

    pub fn borrowed(scope: Scope, owner_user_id: Option<&str>) -> Self {
        Self {
            scope,
            owner_user_id: owner_user_id.map(str::to_string),
        }
    }
}

// ---------------------------------------------------------------------------
// ScopeContext
// ---------------------------------------------------------------------------

/// Access context derived from the authenticated user.
///
/// Used by DAO methods to filter queries by scope.
#[derive(Debug, Clone)]
pub struct ScopeContext {
    pub user_id: String,
    pub is_admin: bool,
}

impl ScopeContext {
    /// Reserved user id for the internal system principal.
    pub const SYSTEM_USER_ID: &'static str = "system";

    /// Full-access context for an internal system caller: `is_admin = true`
    /// matches every row.
    ///
    /// Each call site is a deliberate bypass; `clippy.toml`'s
    /// `disallowed-methods` refuses one without `#[allow(...)]` and a `//
    /// reason:` line.
    pub fn system() -> Self {
        Self {
            user_id: Self::SYSTEM_USER_ID.to_string(),
            is_admin: true,
        }
    }

    /// Check if this context can read a resource with the given scope.
    ///
    /// Task visibility is not an input here: the public-task read arm lives in
    /// the persistence filter and in callers that already hold the row.
    pub fn can_read(&self, resource: &ScopedResource) -> bool {
        if self.is_admin {
            return true;
        }
        match resource.scope {
            Scope::Global => true,
            Scope::User => resource
                .owner_user_id
                .as_deref()
                .is_some_and(|oid| oid == self.user_id),
        }
    }

    /// Check if this context can write (change) a resource.
    ///
    /// Global rows are admin-managed; user rows are owner-managed. Visibility
    /// never grants write.
    pub fn can_write(&self, resource: &ScopedResource) -> bool {
        if self.is_admin {
            return true;
        }
        resource.scope == Scope::User
            && resource
                .owner_user_id
                .as_deref()
                .is_some_and(|oid| oid == self.user_id)
    }

    /// Check if this context can control (operate) a Task's Sandbox.
    ///
    /// Write, plus the public-task arm. Takes [`Visibility`] not
    /// [`ScopedEntity`] since only a Task has a Sandbox; everything else calls
    /// [`Self::can_write`].
    pub fn can_control(&self, resource: &ScopedResource, visibility: Visibility) -> bool {
        visibility == Visibility::Public || self.can_write(resource)
    }

    /// Check if this context can create a resource with the requested scope.
    ///
    /// Global requires admin; user-scoped resources must be owned by the
    /// caller.
    pub fn can_create_scope(&self, scope: Scope, owner_user_id: Option<&str>) -> bool {
        if self.is_admin {
            return true;
        }
        match scope {
            Scope::Global => false,
            Scope::User => owner_user_id.is_some_and(|oid| oid == self.user_id),
        }
    }

    /// Semantic gate for task/sandbox creation from a repository.
    ///
    /// Delegates to repository read access today; call sites use this method
    /// anyway so future policy can diverge without hunting for generic reads.
    pub fn can_use_repository(&self, repository: &ScopedResource) -> bool {
        self.can_read(repository)
    }

    /// Whether this context may start work that acts as `target_user_id`.
    ///
    /// Acting as yourself is always allowed; acting as another is an admin
    /// power over a bot only, since a bot has no session for an admin to
    /// borrow.
    pub fn can_run_as(&self, target_user_id: &str, target_is_bot: bool) -> bool {
        self.user_id == target_user_id || (self.is_admin && target_is_bot)
    }
}

// ---------------------------------------------------------------------------
// Actor — the identity a call acts as
// ---------------------------------------------------------------------------

/// The identity a git-host call is made as.
///
/// A Workspace may pin the token its work presents, outranking the user's own
/// credential; `None` means no Workspace and falls back to it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Actor {
    user_id: String,
    workspace_id: Option<String>,
}

impl Actor {
    /// A user id chosen by a rule this band cannot express — the GitHub PR
    /// Adapter's outbound precedence and webhook sender mapping, both in
    /// `adapter-github-pr/src/auth.rs`.
    pub fn of(user_id: impl Into<String>) -> Self {
        Self {
            user_id: user_id.into(),
            workspace_id: None,
        }
    }

    /// The caller of a request. An admin acts as themselves, never as the owner
    /// of what they are looking at.
    pub fn caller(ctx: &ScopeContext) -> Self {
        Self::of(ctx.user_id.clone())
    }

    /// The owner of a scoped resource — a Task, say.
    ///
    /// Reads [`ScopedResource::owner_user_id`], filled from `owner_user_id`
    /// falling back to `created_by`, so this cannot drift from
    /// `Task::scoped_resource`.
    pub fn owner_of(resource: &ScopedResource) -> Option<Self> {
        resource
            .owner_user_id
            .as_deref()
            .filter(|id| !id.is_empty())
            .map(Self::of)
    }

    /// The identity a Workspace runs as, falling back to the owner of the work.
    ///
    /// A Workspace names the account its Tasks act as, so it outranks whoever
    /// started the Task.
    pub fn run_as(workspace_run_as_user_id: Option<&str>, owner: Option<Self>) -> Option<Self> {
        workspace_run_as_user_id
            .filter(|id| !id.is_empty())
            .map(Self::of)
            .or(owner)
    }

    /// The same identity, acting on behalf of `workspace_id`.
    ///
    /// An empty id is no Workspace, so a caller holding a column that may be
    /// blank does not have to filter it first.
    #[must_use]
    pub fn on_workspace(mut self, workspace_id: Option<&str>) -> Self {
        self.workspace_id = workspace_id
            .filter(|id| !id.is_empty())
            .map(ToOwned::to_owned);
        self
    }

    pub fn user_id(&self) -> &str {
        &self.user_id
    }

    /// The Workspace whose pinned credentials this call may present, if any.
    pub fn workspace_id(&self) -> Option<&str> {
        self.workspace_id.as_deref()
    }
}

impl std::fmt::Display for Actor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.user_id)
    }
}

// ---------------------------------------------------------------------------
// AccessControlError
// ---------------------------------------------------------------------------

/// Why an access-control decision refused.
#[derive(Debug, thiserror::Error)]
pub enum AccessControlError {
    #[error("Bad request: {0}")]
    BadRequest(String),
    #[error("Forbidden: {0}")]
    Forbidden(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_serde_roundtrip() {
        assert_eq!(serde_json::to_string(&Scope::Global).unwrap(), "\"global\"");
        assert_eq!(serde_json::to_string(&Scope::User).unwrap(), "\"user\"");

        let parsed: Scope = serde_json::from_str("\"user\"").unwrap();
        assert_eq!(parsed, Scope::User);
        assert!(serde_json::from_str::<Scope>("\"team\"").is_err());
    }

    #[test]
    fn scope_try_from() {
        assert_eq!(Scope::try_from("global"), Ok(Scope::Global));
        assert_eq!(Scope::try_from("user"), Ok(Scope::User));
        assert!(Scope::try_from("team").is_err());
        assert!(Scope::try_from("invalid").is_err());
    }

    #[test]
    fn scope_try_from_owned_string() {
        // The owned form is what a persisted column is read back through.
        assert_eq!(Scope::try_from("global".to_string()), Ok(Scope::Global));
        assert_eq!(Scope::try_from("user".to_string()), Ok(Scope::User));
        assert!(Scope::try_from("team".to_string()).is_err());
    }

    #[test]
    fn system_context_matches_every_scope() {
        let ctx = ScopeContext::system();
        assert!(ctx.is_admin);
        assert_eq!(ctx.user_id, ScopeContext::SYSTEM_USER_ID);
        // is_admin short-circuits every read/write policy, so the system
        // principal can reach any scoped row regardless of owner.
        assert!(ctx.can_read(&ScopedResource::borrowed(Scope::User, Some("someone"))));
        assert!(ctx.can_write(&ScopedResource::borrowed(Scope::Global, None)));
    }

    #[test]
    fn create_policy_requires_admin_for_global_and_self_for_user() {
        let ctx = ScopeContext {
            user_id: "u1".into(),
            is_admin: false,
        };
        assert!(ctx.can_create_scope(Scope::User, Some("u1")));
        assert!(!ctx.can_create_scope(Scope::User, Some("u2")));
        assert!(!ctx.can_create_scope(Scope::User, None));
        assert!(!ctx.can_create_scope(Scope::Global, None));

        let admin = ScopeContext {
            user_id: "root".into(),
            is_admin: true,
        };
        assert!(admin.can_create_scope(Scope::Global, None));
    }

    #[test]
    fn run_as_allows_self_or_an_admin_over_a_bot() {
        let user = ScopeContext {
            user_id: "u1".into(),
            is_admin: false,
        };
        let admin = ScopeContext {
            is_admin: true,
            ..user.clone()
        };

        assert!(user.can_run_as("u1", false));
        assert!(!user.can_run_as("u2", false));
        assert!(!user.can_run_as("bot", true), "a bot needs admin too");

        assert!(admin.can_run_as("bot", true));
        assert!(
            !admin.can_run_as("u2", false),
            "an admin may not borrow a person's account"
        );
    }

    #[test]
    fn repository_use_delegates_to_read_policy() {
        let ctx = ScopeContext {
            user_id: "u1".into(),
            is_admin: false,
        };

        for resource in [
            ScopedResource::borrowed(Scope::Global, None),
            ScopedResource::borrowed(Scope::User, Some("u1")),
            ScopedResource::borrowed(Scope::User, Some("u2")),
        ] {
            assert_eq!(ctx.can_use_repository(&resource), ctx.can_read(&resource));
        }
    }

    #[test]
    fn missing_owner_is_denied_for_user_scope() {
        let ctx = ScopeContext {
            user_id: "u1".into(),
            is_admin: false,
        };
        let ownerless = ScopedResource::borrowed(Scope::User, None);

        assert!(!ctx.can_read(&ownerless));
        assert!(!ctx.can_write(&ownerless));
    }

    #[test]
    fn control_follows_visibility_where_write_does_not() {
        let stranger = ScopeContext {
            user_id: "stranger".into(),
            is_admin: false,
        };
        let owned = ScopedResource::borrowed(Scope::User, Some("owner"));

        assert!(stranger.can_control(&owned, Visibility::Public));
        assert!(!stranger.can_control(&owned, Visibility::Private));
        assert!(!stranger.can_write(&owned));

        // An unowned row names nobody to match, so control still follows
        // visibility and write has no path at all.
        let unowned = ScopedResource::borrowed(Scope::User, None);
        assert!(stranger.can_control(&unowned, Visibility::Public));
        assert!(!stranger.can_write(&unowned));
    }

    // -- policy matrix --
    //
    // Exhaustive allow/deny table over actor × scope × task visibility ×
    // action. This test IS the policy's documentation: any behavioral change
    // to access control must show up as a diff in the expected table below.

    #[test]
    fn policy_matrix() {
        let owner = "owner";

        let actors: Vec<(&str, ScopeContext)> = vec![
            (
                "owner",
                ScopeContext {
                    user_id: owner.into(),
                    is_admin: false,
                },
            ),
            (
                "other",
                ScopeContext {
                    user_id: "other".into(),
                    is_admin: false,
                },
            ),
            (
                "admin",
                ScopeContext {
                    user_id: "root".into(),
                    is_admin: true,
                },
            ),
        ];

        let tasks = [
            ("global", ScopedResource::borrowed(Scope::Global, None)),
            ("user", ScopedResource::borrowed(Scope::User, Some(owner))),
        ];

        let verdict = |allowed: bool| if allowed { "allow" } else { "deny" };
        let mut lines = Vec::new();
        for (scope_name, task) in &tasks {
            for visibility in [Visibility::Private, Visibility::Public] {
                let public = visibility == Visibility::Public;
                for (actor_name, ctx) in &actors {
                    // read: pure policy plus the SQL filter's public-task arm.
                    let read = ctx.can_read(task) || public;
                    // interact (send message / follow / mark read) is read:
                    // a public task is collaborative by product rule.
                    let interact = read;
                    // control: operating the Sandbox. Visibility grants it.
                    let control = ctx.can_control(task, visibility);
                    // write: changing the Task or its durable config. It never
                    // follows from visibility.
                    let write = ctx.can_write(task);
                    lines.push(format!(
                        "{scope_name:<6} {vis:<7} {actor_name:<5} \
                         read={} interact={} control={} write={}",
                        verdict(read),
                        verdict(interact),
                        verdict(control),
                        verdict(write),
                        vis = if public { "public" } else { "private" },
                    ));
                }
            }
        }

        let expected = "\
global private owner read=allow interact=allow control=deny write=deny
global private other read=allow interact=allow control=deny write=deny
global private admin read=allow interact=allow control=allow write=allow
global public  owner read=allow interact=allow control=allow write=deny
global public  other read=allow interact=allow control=allow write=deny
global public  admin read=allow interact=allow control=allow write=allow
user   private owner read=allow interact=allow control=allow write=allow
user   private other read=deny interact=deny control=deny write=deny
user   private admin read=allow interact=allow control=allow write=allow
user   public  owner read=allow interact=allow control=allow write=allow
user   public  other read=allow interact=allow control=allow write=deny
user   public  admin read=allow interact=allow control=allow write=allow";
        assert_eq!(lines.join("\n"), expected);
    }

    // -- actor --

    #[test]
    fn a_caller_acts_as_themselves_even_as_admin() {
        let ctx = ScopeContext {
            user_id: "admin".to_string(),
            is_admin: true,
        };
        assert_eq!(Actor::caller(&ctx).user_id(), "admin");
    }

    #[test]
    fn owner_of_falls_back_to_created_by_through_scoped_resource() {
        // `Task::scoped_resource` is `owner_user_id.or(created_by)`, so a row
        // predating `owner_user_id` still names an identity here.
        let backfilled = ScopedResource::borrowed(Scope::User, Some("creator"));
        assert_eq!(
            Actor::owner_of(&backfilled),
            Some(Actor::of("creator")),
            "a resource carrying only created_by must still name an actor"
        );

        let ownerless = ScopedResource::borrowed(Scope::Global, None);
        assert_eq!(Actor::owner_of(&ownerless), None);
    }

    #[test]
    fn a_workspace_run_as_identity_outranks_the_owner() {
        let owner = Some(Actor::of("starter"));
        assert_eq!(
            Actor::run_as(Some("bot"), owner.clone()),
            Some(Actor::of("bot"))
        );
        assert_eq!(Actor::run_as(None, owner.clone()), owner);
        assert_eq!(
            Actor::run_as(Some(""), owner.clone()),
            owner,
            "an empty column is not a named identity"
        );
        assert_eq!(Actor::run_as(None, None), None);
    }
}

#[derive(Debug, Clone)]
pub struct ResolvedScope {
    pub scope: Scope,
    pub owner_user_id: Option<String>,
}

/// Resolve the scope for a new resource: defaults to `User` owned by the
/// caller; requesting `Global` requires admin.
pub fn resolve_scope_for_create(
    ctx: &ScopeContext,
    requested_scope: Option<Scope>,
) -> Result<ResolvedScope, AccessControlError> {
    let scope = requested_scope.unwrap_or(Scope::User);
    let owner_user_id = (scope == Scope::User).then(|| ctx.user_id.clone());

    if !ctx.can_create_scope(scope, owner_user_id.as_deref()) {
        return Err(AccessControlError::Forbidden(format!(
            "Cannot create {scope} scoped resources"
        )));
    }

    Ok(ResolvedScope {
        scope,
        owner_user_id,
    })
}

pub fn ensure_can_use_repository(
    ctx: &ScopeContext,
    repository: &ScopedResource,
) -> Result<(), AccessControlError> {
    if ctx.can_use_repository(repository) {
        Ok(())
    } else {
        Err(AccessControlError::Forbidden(
            "No access to repository".to_string(),
        ))
    }
}
