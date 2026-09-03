//! Canonical access control: which identity acts, and what it may reach.
//!
//! The `Scope` enum is owned here. `ScopeContext` is used for both query
//! filtering and pure policy checks. The policy is a pure function of
//! `(user_id, is_admin)` × `(scope, owner, visibility)` — no database input:
//!
//! - **read**: `is_admin ∨ scope = global ∨ owner = me ∨ (Task ∧ visibility = public)`
//! - **control**: `is_admin ∨ (scope = user ∧ owner = me) ∨ (Task ∧ visibility = public)`
//! - **write**: `is_admin ∨ (scope = user ∧ owner = me)`
//!
//! Owner is `COALESCE(owner_user_id, created_by)`. For tasks, read includes
//! interact (send message, follow, mark read) by product rule — a public task
//! is collaborative.
//!
//! Control is what operates a Task's Sandbox: start, stop, restart, redeploy,
//! open a shell, run the agent. Write is what changes something the owner
//! would otherwise have to find and undo: remove the Sandbox, reconfigure its
//! servers, close or rename or delete the Task. Control includes a shell, so
//! anything a shell can reach is control by construction — refusing a file
//! write while granting `terminal.open` is a rule one command undoes.
//!
//! [`Actor`] names the identity a call acts as. Every constructor on it is one
//! way an identity can be chosen, so a reader can enumerate the rules without
//! searching for them. Two rules cannot live here, because they read types
//! this band cannot see: the GitHub PR Adapter's outbound precedence
//! (`TaskBinding`, `SubscriptionMatch`) and its webhook sender mapping. Both
//! stay in `adapter-github-pr/src/auth.rs` and end in [`Actor::of`], which is
//! why that constructor exists.

#![cfg_attr(
    test,
    allow(
        clippy::disallowed_methods,
        clippy::unwrap_used,
        reason = "unit tests assert exact policy cases, including the system bypass"
    )
)]

use serde::{Deserialize, Serialize};

use crate::task::Visibility;

/// Access scope for repositories, tasks, integrations, and credentials.
///
/// Reads: `global` is readable by every authenticated user; `user` only by
/// its owner (and admins). Writes: `global` is admin-managed; `user` is
/// owner-managed. Task `visibility = 'public'` additionally grants
/// read/interact and control on tasks — never write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    Global,
    User,
}

impl Scope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::User => "user",
        }
    }
}

impl std::fmt::Display for Scope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<&str> for Scope {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "global" => Ok(Self::Global),
            "user" => Ok(Self::User),
            other => Err(format!("unknown Scope: {other:?}")),
        }
    }
}

impl TryFrom<String> for Scope {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::try_from(value.as_str())
    }
}

// ---------------------------------------------------------------------------
// ScopedEntity — declares per-entity schema facts
// ---------------------------------------------------------------------------

/// Declares access-control schema properties for each entity type.
///
/// The entity determines whether `visibility = 'public'` bypasses scope checks —
/// callers never choose this themselves, eliminating a class of bugs.
#[derive(Debug, Clone, Copy)]
pub enum ScopedEntity {
    /// `tasks` table — has a `visibility` column.
    Task,
    /// `repositories` table — canonical scoped resource; no visibility bypass.
    Repository,
    /// Scoped tables without a `visibility` column.
    Resource,
}

impl ScopedEntity {
    fn has_visibility(self) -> bool {
        match self {
            Self::Task => true,
            Self::Repository | Self::Resource => false,
        }
    }
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
// AccessFilter — encapsulates a SQL fragment + parameter tracking
// ---------------------------------------------------------------------------

/// Access-control WHERE clause fragment with parameter tracking.
///
/// Created via [`ScopeContext::access_filter()`] or
/// [`ScopeContext::write_filter()`]. Contains the SQL fragment and tracks
/// the next available positional parameter index.
pub struct AccessFilter {
    sql: String,
    next_param: usize,
}

impl AccessFilter {
    /// The SQL WHERE clause fragment (e.g. `($1 OR visibility = 'public' OR ...)`).
    pub fn sql(&self) -> &str {
        &self.sql
    }

    /// Next available positional parameter index after this filter's params.
    pub fn next_param(&self) -> usize {
        self.next_param
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

    /// Full-access context for internal system callers that have no
    /// authenticated user to derive a scope from — background jobs, reapers,
    /// pod watchers, adapter/webhook runtimes, and sandbox-pod-facing requests
    /// already authenticated by their own transport.
    ///
    /// Semantically equivalent to an unscoped query: `is_admin = true` makes
    /// the filters match every row (the `$is_admin` disjunct is always true).
    /// Every call site is a deliberate access-control bypass and must carry
    /// `#[allow(clippy::disallowed_methods)]` with a `// reason:` comment —
    /// enforced via `disallowed-methods` in `clippy.toml`.
    ///
    /// The required form at a call site:
    ///
    /// ```
    /// use moatless_types::access_control::ScopeContext;
    ///
    /// // reason: reaper sweeps idle sandboxes across all owners (no user ctx)
    /// #[allow(clippy::disallowed_methods)]
    /// let sys = ScopeContext::system();
    /// assert!(sys.is_admin);
    /// ```
    pub fn system() -> Self {
        Self {
            user_id: Self::SYSTEM_USER_ID.to_string(),
            is_admin: true,
        }
    }

    /// Build a read SQL filter for the given entity.
    ///
    /// Uses 2 positional parameters starting at `start`:
    ///   - `$start`: `is_admin` (bool)
    ///   - `$start+1`: `user_id` (text)
    ///
    /// Caller must `.bind()` these from `self.is_admin`, `self.user_id`.
    ///
    /// `prefix` is the table alias (e.g. `""` or `"t."`).
    /// The entity determines whether `visibility = 'public'` is included.
    pub fn access_filter(&self, entity: ScopedEntity, prefix: &str, start: usize) -> AccessFilter {
        let (p1, p2) = (start, start + 1);
        let vis = if entity.has_visibility() {
            format!(" OR {prefix}visibility = 'public'")
        } else {
            String::new()
        };
        let sql = format!(
            "(${p1}{vis} OR {prefix}scope = 'global' \
             OR ({prefix}scope = 'user' AND COALESCE({prefix}owner_user_id, {prefix}created_by) = ${p2}))"
        );
        AccessFilter {
            sql,
            next_param: start + 2,
        }
    }

    /// Build a write (mutation) SQL filter.
    ///
    /// Write is the strictest grant: global rows are admin-managed and task
    /// visibility never grants it, so the fragment is identical for every
    /// entity and this takes no [`ScopedEntity`].
    ///
    /// Uses 2 positional parameters starting at `start`:
    ///   - `$start`: `is_admin` (bool)
    ///   - `$start+1`: `user_id` (text)
    ///
    /// Caller must `.bind()` these from `self.is_admin`, `self.user_id`.
    /// `prefix` is the table alias (e.g. `""` or `"t."`).
    ///
    /// Embedding this in a mutation's `WHERE` makes the check and the write a
    /// single atomic statement; `rows_affected == 0` maps to the module's
    /// `NotFound` error, matching the existence-hiding reads.
    pub fn write_filter(&self, prefix: &str, start: usize) -> AccessFilter {
        let (p1, p2) = (start, start + 1);
        let sql = format!(
            "(${p1} OR ({prefix}scope = 'user' \
             AND COALESCE({prefix}owner_user_id, {prefix}created_by) = ${p2}))"
        );
        AccessFilter {
            sql,
            next_param: start + 2,
        }
    }

    /// Build a control (operate) SQL filter.
    ///
    /// Control is write plus the public-task arm. The entity decides whether
    /// that arm appears, exactly as it does for [`Self::access_filter`], so a
    /// caller cannot ask for it on a table with no `visibility` column. For
    /// every entity but [`ScopedEntity::Task`] this is [`Self::write_filter`],
    /// because only a Task has anything to operate.
    ///
    /// Uses 2 positional parameters starting at `start`:
    ///   - `$start`: `is_admin` (bool)
    ///   - `$start+1`: `user_id` (text)
    ///
    /// Caller must `.bind()` these from `self.is_admin`, `self.user_id`.
    /// `prefix` is the table alias (e.g. `""` or `"t."`).
    pub fn control_filter(&self, entity: ScopedEntity, prefix: &str, start: usize) -> AccessFilter {
        let (p1, p2) = (start, start + 1);
        let vis = if entity.has_visibility() {
            format!(" OR {prefix}visibility = 'public'")
        } else {
            String::new()
        };
        let sql = format!(
            "(${p1}{vis} OR ({prefix}scope = 'user' \
             AND COALESCE({prefix}owner_user_id, {prefix}created_by) = ${p2}))"
        );
        AccessFilter {
            sql,
            next_param: start + 2,
        }
    }

    /// Check if this context can read a resource with the given scope.
    ///
    /// Task visibility is not an input here: the public-task read arm lives
    /// in [`Self::access_filter`] and in handlers that already hold the row.
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
    /// Write, plus the public-task arm: a public Task is collaborative, and a
    /// Sandbox nobody but the owner can wake, restart or work in is what makes
    /// that collaboration unusable. Visibility is an input here, unlike
    /// [`Self::can_read`], because it is the whole of what this grant adds.
    ///
    /// Only a Task has a Sandbox to operate, so this takes a [`Visibility`]
    /// rather than a [`ScopedEntity`]: a caller holding anything else has no
    /// grant between read and write and calls [`Self::can_write`].
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
    /// Today this intentionally delegates to repository read access; call sites
    /// use this method anyway so future policy can diverge without hunting for
    /// generic reads.
    pub fn can_use_repository(&self, repository: &ScopedResource) -> bool {
        self.can_read(repository)
    }

    /// Explicit run-as helper for automation rules.
    pub fn can_run_as(&self, target_user_id: &str) -> bool {
        self.is_admin || self.user_id == target_user_id
    }
}

// ---------------------------------------------------------------------------
// Actor — the identity a call acts as
// ---------------------------------------------------------------------------

/// The identity a git-host call is made as.
///
/// There is no way to obtain a git-host token without naming one — see
/// `moatless_identity::git_host_tokens::GitHostTokens`.
///
/// A Workspace may pin the token its work presents to a git host. An Actor
/// therefore carries the Workspace the call is made on behalf of, when there is
/// one: the pin outranks the user's own credential, and naming the Workspace
/// here is what lets every caller resolve the same token from the same seam
/// rather than each deciding for itself. [`Self::workspace_id`] is `None` for
/// work no Workspace is behind — a settings check, an interactive request — and
/// that resolves to the user's own credential.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Actor {
    user_id: String,
    workspace_id: Option<String>,
}

impl Actor {
    /// A user id chosen by a rule this band cannot express.
    pub fn of(user_id: impl Into<String>) -> Self {
        Self {
            user_id: user_id.into(),
            workspace_id: None,
        }
    }

    /// The caller of a request. An admin acts as themselves, never as the
    /// owner of what they are looking at.
    pub fn caller(ctx: &ScopeContext) -> Self {
        Self::of(ctx.user_id.clone())
    }

    /// The owner of a scoped resource — a Task, say.
    ///
    /// Reads [`ScopedResource::owner_user_id`], which a Task fills from
    /// `owner_user_id` falling back to `created_by`, so this cannot drift from
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
    fn system_access_filter_admin_param_is_true() {
        // The filter's first positional param is `is_admin`; for the system
        // principal it binds `true`, which makes the whole disjunct match all
        // rows — identical semantics to an unscoped `SELECT ... WHERE id = $1`.
        let ctx = ScopeContext::system();
        assert!(ctx.is_admin);
        let f = ctx.access_filter(ScopedEntity::Task, "", 1);
        assert!(f.sql().starts_with("($1 OR"));
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
    fn run_as_allows_self_or_admin_only() {
        let user = ScopeContext {
            user_id: "u1".into(),
            is_admin: false,
        };
        let admin = ScopeContext {
            is_admin: true,
            ..user.clone()
        };

        assert!(user.can_run_as("u1"));
        assert!(!user.can_run_as("u2"));
        assert!(admin.can_run_as("u2"));
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

    // -- filter tests --

    fn test_ctx() -> ScopeContext {
        ScopeContext {
            user_id: "u1".into(),
            is_admin: false,
        }
    }

    #[test]
    fn access_filter_task_includes_visibility() {
        let f = test_ctx().access_filter(ScopedEntity::Task, "", 1);
        assert_eq!(
            f.sql(),
            "($1 OR visibility = 'public' OR scope = 'global' \
             OR (scope = 'user' AND COALESCE(owner_user_id, created_by) = $2))"
        );
        assert_eq!(f.next_param(), 3);
    }

    #[test]
    fn access_filter_repository_excludes_visibility() {
        let f = test_ctx().access_filter(ScopedEntity::Repository, "", 2);
        assert_eq!(
            f.sql(),
            "($2 OR scope = 'global' \
             OR (scope = 'user' AND COALESCE(owner_user_id, created_by) = $3))"
        );
        assert_eq!(f.next_param(), 4);
    }

    #[test]
    fn access_filter_resource_no_visibility() {
        let f = test_ctx().access_filter(ScopedEntity::Resource, "", 1);
        assert_eq!(
            f.sql(),
            "($1 OR scope = 'global' \
             OR (scope = 'user' AND COALESCE(owner_user_id, created_by) = $2))"
        );
        assert_eq!(f.next_param(), 3);
    }

    #[test]
    fn access_filter_with_table_prefix() {
        let f = test_ctx().access_filter(ScopedEntity::Task, "t.", 1);
        assert_eq!(
            f.sql(),
            "($1 OR t.visibility = 'public' OR t.scope = 'global' \
             OR (t.scope = 'user' AND COALESCE(t.owner_user_id, t.created_by) = $2))"
        );
    }

    #[test]
    fn write_filter_admin_or_owner_only() {
        let f = test_ctx().write_filter("", 1);
        assert_eq!(
            f.sql(),
            "($1 OR (scope = 'user' AND COALESCE(owner_user_id, created_by) = $2))"
        );
        assert_eq!(f.next_param(), 3);
    }

    #[test]
    fn write_filter_with_table_prefix() {
        let f = test_ctx().write_filter("t.", 3);
        assert_eq!(
            f.sql(),
            "($3 OR (t.scope = 'user' AND COALESCE(t.owner_user_id, t.created_by) = $4))"
        );
        assert_eq!(f.next_param(), 5);
    }

    #[test]
    fn control_filter_task_adds_the_public_arm_to_the_write_fragment() {
        let f = test_ctx().control_filter(ScopedEntity::Task, "", 1);
        assert_eq!(
            f.sql(),
            "($1 OR visibility = 'public' \
             OR (scope = 'user' AND COALESCE(owner_user_id, created_by) = $2))"
        );
        assert_eq!(f.next_param(), 3);
    }

    #[test]
    fn control_filter_without_visibility_is_the_write_fragment() {
        // Only a Task has a Sandbox to operate. On every other entity the two
        // grants coincide, and the fragment must not name a column the table
        // does not have.
        let ctx = test_ctx();
        for entity in [ScopedEntity::Repository, ScopedEntity::Resource] {
            assert_eq!(
                ctx.control_filter(entity, "t.", 3).sql(),
                ctx.write_filter("t.", 3).sql()
            );
        }
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
