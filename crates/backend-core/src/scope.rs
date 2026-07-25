//! Canonical access-control scope policy.
//!
//! The `Scope` enum is re-exported from `moatless-api-schemas`.
//! `ScopeContext` is backend-only and is used for both query filtering and
//! pure policy checks. The policy is a pure function of
//! `(user_id, is_admin)` × `(scope, owner, visibility)` — no database input:
//!
//! - **read**: `is_admin ∨ scope = global ∨ owner = me ∨ (Task ∧ visibility = public)`
//! - **control**: `is_admin ∨ (scope = user ∧ owner = me)`
//!
//! Owner is `COALESCE(owner_user_id, created_by)`. These are the only two
//! grants; for tasks, read includes interact (send message, follow,
//! mark read) by product rule — a public task is collaborative.

pub use moatless_api_schemas::enums::Scope;

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
/// [`ScopeContext::control_filter()`]. Contains the SQL fragment and tracks
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
    /// use moatless_backend_core::scope::ScopeContext;
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

    /// Build a control (mutation) SQL filter.
    ///
    /// Control is stricter than read: global rows are admin-controlled and
    /// task visibility never grants control, so the fragment is identical for
    /// every entity.
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
    /// NotFound error, matching the existence-hiding reads.
    pub fn control_filter(&self, prefix: &str, start: usize) -> AccessFilter {
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

    /// Check if this context can control (mutate, operate) a resource.
    ///
    /// Global rows are admin-controlled; user rows are owner-controlled.
    /// Visibility never grants control.
    pub fn can_control(&self, resource: &ScopedResource) -> bool {
        if self.is_admin {
            return true;
        }
        resource.scope == Scope::User
            && resource
                .owner_user_id
                .as_deref()
                .is_some_and(|oid| oid == self.user_id)
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
    fn system_context_matches_every_scope() {
        let ctx = ScopeContext::system();
        assert!(ctx.is_admin);
        assert_eq!(ctx.user_id, ScopeContext::SYSTEM_USER_ID);
        // is_admin short-circuits every read/control policy, so the system
        // principal can reach any scoped row regardless of owner.
        assert!(ctx.can_read(&ScopedResource::borrowed(Scope::User, Some("someone"))));
        assert!(ctx.can_control(&ScopedResource::borrowed(Scope::Global, None)));
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
        assert!(!ctx.can_control(&ownerless));
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
    fn control_filter_admin_or_owner_only() {
        let f = test_ctx().control_filter("", 1);
        assert_eq!(
            f.sql(),
            "($1 OR (scope = 'user' AND COALESCE(owner_user_id, created_by) = $2))"
        );
        assert_eq!(f.next_param(), 3);
    }

    #[test]
    fn control_filter_with_table_prefix() {
        let f = test_ctx().control_filter("t.", 3);
        assert_eq!(
            f.sql(),
            "($3 OR (t.scope = 'user' AND COALESCE(t.owner_user_id, t.created_by) = $4))"
        );
        assert_eq!(f.next_param(), 5);
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
            for public in [false, true] {
                for (actor_name, ctx) in &actors {
                    // read: pure policy plus the SQL filter's public-task arm.
                    let read = ctx.can_read(task) || public;
                    // interact (send message / follow / mark read) is read:
                    // a public task is collaborative by product rule.
                    let interact = read;
                    // control never follows from visibility.
                    let control = ctx.can_control(task);
                    lines.push(format!(
                        "{scope_name:<6} {vis:<7} {actor_name:<5} read={} interact={} control={}",
                        verdict(read),
                        verdict(interact),
                        verdict(control),
                        vis = if public { "public" } else { "private" },
                    ));
                }
            }
        }

        let expected = "\
global private owner read=allow interact=allow control=deny
global private other read=allow interact=allow control=deny
global private admin read=allow interact=allow control=allow
global public  owner read=allow interact=allow control=deny
global public  other read=allow interact=allow control=deny
global public  admin read=allow interact=allow control=allow
user   private owner read=allow interact=allow control=allow
user   private other read=deny interact=deny control=deny
user   private admin read=allow interact=allow control=allow
user   public  owner read=allow interact=allow control=allow
user   public  other read=allow interact=allow control=deny
user   public  admin read=allow interact=allow control=allow";
        assert_eq!(lines.join("\n"), expected);
    }
}
