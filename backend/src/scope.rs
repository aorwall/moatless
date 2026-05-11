//! Canonical access-control scope policy.
//!
//! The `Scope` enum is re-exported from `moatless-api-schemas`.
//! `ScopeContext` is backend-only and is used for both query filtering and
//! pure policy checks. Database-backed team-role validation lives in handlers
//! and passes a parsed [`TeamRole`] into the write/create helpers.

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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopedResource {
    pub scope: Scope,
    pub team_id: Option<String>,
    pub owner_user_id: Option<String>,
}

impl ScopedResource {
    pub fn new<T, U>(scope: Scope, team_id: Option<T>, owner_user_id: Option<U>) -> Self
    where
        T: Into<String>,
        U: Into<String>,
    {
        Self {
            scope,
            team_id: team_id.map(Into::into),
            owner_user_id: owner_user_id.map(Into::into),
        }
    }

    pub fn borrowed(scope: Scope, team_id: Option<&str>, owner_user_id: Option<&str>) -> Self {
        Self {
            scope,
            team_id: team_id.map(str::to_string),
            owner_user_id: owner_user_id.map(str::to_string),
        }
    }
}

/// Team membership role used by write/create checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TeamRole {
    Owner,
    Admin,
    Member,
}

impl TeamRole {
    pub fn can_manage(self) -> bool {
        matches!(self, Self::Owner | Self::Admin)
    }
}

impl TryFrom<&str> for TeamRole {
    type Error = ();

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "owner" => Ok(Self::Owner),
            "admin" => Ok(Self::Admin),
            "member" => Ok(Self::Member),
            _ => Err(()),
        }
    }
}

// ---------------------------------------------------------------------------
// AccessFilter — encapsulates a SQL fragment + parameter tracking
// ---------------------------------------------------------------------------

/// Access-control WHERE clause fragment with parameter tracking.
///
/// Created via [`ScopeContext::access_filter()`]. Contains the SQL fragment
/// and tracks the next available positional parameter index.
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
    pub team_ids: Vec<String>,
    pub is_admin: bool,
}

impl ScopeContext {
    /// Build an access-control SQL filter for the given entity.
    ///
    /// Uses 3 positional parameters starting at `start`:
    ///   - `$start`: `is_admin` (bool)
    ///   - `$start+1`: `team_ids` (text[])
    ///   - `$start+2`: `user_id` (text)
    ///
    /// Caller must `.bind()` these from `self.is_admin`, `self.team_ids`, `self.user_id`.
    ///
    /// `prefix` is the table alias (e.g. `""` or `"t."`).
    /// The entity determines whether `visibility = 'public'` is included.
    pub fn access_filter(&self, entity: ScopedEntity, prefix: &str, start: usize) -> AccessFilter {
        let (p1, p2, p3) = (start, start + 1, start + 2);
        let vis = if entity.has_visibility() {
            format!(" OR {prefix}visibility = 'public'")
        } else {
            String::new()
        };
        let sql = format!(
            "(${p1}{vis} OR {prefix}scope = 'global' \
             OR ({prefix}scope = 'team' AND {prefix}team_id = ANY(${p2})) \
             OR ({prefix}scope = 'user' AND COALESCE({prefix}owner_user_id, {prefix}created_by) = ${p3}))"
        );
        AccessFilter {
            sql,
            next_param: start + 3,
        }
    }

    /// Check if this context can read a resource with the given scope.
    pub fn can_read(&self, resource: &ScopedResource) -> bool {
        if self.is_admin {
            return true;
        }
        match resource.scope {
            Scope::Global => true,
            Scope::Team => {
                if let Some(tid) = resource.team_id.as_deref() {
                    self.team_ids.iter().any(|t| t == tid)
                } else {
                    false
                }
            }
            Scope::User => resource
                .owner_user_id
                .as_deref()
                .is_some_and(|oid| oid == self.user_id),
        }
    }

    /// Backwards-compatible read check for call sites that have not yet been
    /// converted to `ScopedResource`.
    pub fn can_access(&self, scope: Scope, team_id: Option<&str>, owner_id: Option<&str>) -> bool {
        self.can_read(&ScopedResource::borrowed(scope, team_id, owner_id))
    }

    /// Check if this context can write an existing resource.
    ///
    /// `team_role` is the caller's role in `resource.team_id` when known.
    pub fn can_write(&self, resource: &ScopedResource, team_role: Option<TeamRole>) -> bool {
        if self.is_admin {
            return true;
        }
        match resource.scope {
            Scope::Global => false,
            Scope::Team => team_role.is_some_and(TeamRole::can_manage),
            Scope::User => resource
                .owner_user_id
                .as_deref()
                .is_some_and(|oid| oid == self.user_id),
        }
    }

    /// Check if this context can create a resource with the requested scope.
    ///
    /// User-scoped resources must be created for the current user inside the
    /// resolved team. Team/global resources require elevated privileges.
    pub fn can_create_scope(
        &self,
        scope: Scope,
        owner_user_id: Option<&str>,
        team_role: Option<TeamRole>,
    ) -> bool {
        if self.is_admin {
            return true;
        }
        match scope {
            Scope::Global => false,
            Scope::Team => team_role.is_some_and(TeamRole::can_manage),
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

    /// Explicit task-control helper for mutating or operating a task.
    pub fn can_control_task(&self, task: &ScopedResource, team_role: Option<TeamRole>) -> bool {
        self.can_write(task, team_role)
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
        assert_eq!(serde_json::to_string(&Scope::Team).unwrap(), "\"team\"");
        assert_eq!(serde_json::to_string(&Scope::User).unwrap(), "\"user\"");

        let parsed: Scope = serde_json::from_str("\"team\"").unwrap();
        assert_eq!(parsed, Scope::Team);
    }

    #[test]
    fn scope_try_from() {
        assert_eq!(Scope::try_from("global"), Ok(Scope::Global));
        assert_eq!(Scope::try_from("team"), Ok(Scope::Team));
        assert_eq!(Scope::try_from("user"), Ok(Scope::User));
        assert!(Scope::try_from("invalid").is_err());
    }

    #[test]
    fn can_access_global() {
        let ctx = ScopeContext {
            user_id: "u1".into(),
            team_ids: vec![],
            is_admin: false,
        };
        assert!(ctx.can_access(Scope::Global, None, None));
        assert!(ctx.can_read(&ScopedResource::borrowed(Scope::Global, None, None)));
    }

    #[test]
    fn can_access_team_member() {
        let ctx = ScopeContext {
            user_id: "u1".into(),
            team_ids: vec!["t1".into()],
            is_admin: false,
        };
        assert!(ctx.can_access(Scope::Team, Some("t1"), None));
        assert!(!ctx.can_access(Scope::Team, Some("t2"), None));
    }

    #[test]
    fn can_access_user_owner() {
        let ctx = ScopeContext {
            user_id: "u1".into(),
            team_ids: vec![],
            is_admin: false,
        };
        assert!(ctx.can_access(Scope::User, None, Some("u1")));
        assert!(!ctx.can_access(Scope::User, None, Some("u2")));
    }

    #[test]
    fn admin_bypasses_all() {
        let ctx = ScopeContext {
            user_id: "u1".into(),
            team_ids: vec![],
            is_admin: true,
        };
        assert!(ctx.can_access(Scope::User, None, Some("u2")));
        assert!(ctx.can_access(Scope::Team, Some("t99"), None));
        assert!(ctx.can_write(
            &ScopedResource::borrowed(Scope::Global, Some("system"), None),
            None
        ));
    }

    #[test]
    fn write_policy_is_stricter_than_read_for_team_resources() {
        let ctx = ScopeContext {
            user_id: "u1".into(),
            team_ids: vec!["t1".into()],
            is_admin: false,
        };
        let resource = ScopedResource::borrowed(Scope::Team, Some("t1"), None);
        assert!(ctx.can_read(&resource));
        assert!(!ctx.can_write(&resource, Some(TeamRole::Member)));
        assert!(ctx.can_write(&resource, Some(TeamRole::Admin)));
        assert!(ctx.can_write(&resource, Some(TeamRole::Owner)));
    }

    #[test]
    fn create_policy_uses_requested_scope() {
        let ctx = ScopeContext {
            user_id: "u1".into(),
            team_ids: vec!["t1".into()],
            is_admin: false,
        };
        assert!(ctx.can_create_scope(Scope::User, Some("u1"), None));
        assert!(!ctx.can_create_scope(Scope::User, Some("u2"), None));
        assert!(!ctx.can_create_scope(Scope::Team, None, Some(TeamRole::Member)));
        assert!(ctx.can_create_scope(Scope::Team, None, Some(TeamRole::Admin)));
        assert!(!ctx.can_create_scope(Scope::Global, None, None));
    }

    // -- access_filter tests --

    fn test_ctx() -> ScopeContext {
        ScopeContext {
            user_id: "u1".into(),
            team_ids: vec![],
            is_admin: false,
        }
    }

    #[test]
    fn access_filter_task_includes_visibility() {
        let f = test_ctx().access_filter(ScopedEntity::Task, "", 1);
        assert_eq!(
            f.sql(),
            "($1 OR visibility = 'public' OR scope = 'global' \
             OR (scope = 'team' AND team_id = ANY($2)) \
             OR (scope = 'user' AND COALESCE(owner_user_id, created_by) = $3))"
        );
        assert_eq!(f.next_param(), 4);
    }

    #[test]
    fn access_filter_repository_excludes_visibility() {
        let f = test_ctx().access_filter(ScopedEntity::Repository, "", 2);
        assert_eq!(
            f.sql(),
            "($2 OR scope = 'global' \
             OR (scope = 'team' AND team_id = ANY($3)) \
             OR (scope = 'user' AND COALESCE(owner_user_id, created_by) = $4))"
        );
        assert_eq!(f.next_param(), 5);
    }

    #[test]
    fn access_filter_resource_no_visibility() {
        let f = test_ctx().access_filter(ScopedEntity::Resource, "", 1);
        assert_eq!(
            f.sql(),
            "($1 OR scope = 'global' \
             OR (scope = 'team' AND team_id = ANY($2)) \
             OR (scope = 'user' AND COALESCE(owner_user_id, created_by) = $3))"
        );
        assert_eq!(f.next_param(), 4);
    }

    #[test]
    fn access_filter_with_table_prefix() {
        let f = test_ctx().access_filter(ScopedEntity::Task, "t.", 1);
        assert_eq!(
            f.sql(),
            "($1 OR t.visibility = 'public' OR t.scope = 'global' \
             OR (t.scope = 'team' AND t.team_id = ANY($2)) \
             OR (t.scope = 'user' AND COALESCE(t.owner_user_id, t.created_by) = $3))"
        );
    }
}
