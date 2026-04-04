//! Access control scope types.
//!
//! The `Scope` enum is re-exported from `moatless-api-schemas`.
//! `ScopeContext` is backend-only (used for query filtering).

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
    /// `repositories` table — has a `visibility` column.
    Repository,
    /// `channel_configs` table — no `visibility` column.
    ChannelConfig,
}

impl ScopedEntity {
    fn has_visibility(self) -> bool {
        match self {
            Self::Task | Self::Repository => true,
            Self::ChannelConfig => false,
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
             OR ({prefix}scope = 'user' AND {prefix}created_by = ${p3}))"
        );
        AccessFilter {
            sql,
            next_param: start + 3,
        }
    }

    /// Check if this context has access to a resource with the given scope.
    pub fn can_access(&self, scope: Scope, team_id: Option<&str>, owner_id: Option<&str>) -> bool {
        if self.is_admin {
            return true;
        }
        match scope {
            Scope::Global => true,
            Scope::Team => {
                if let Some(tid) = team_id {
                    self.team_ids.iter().any(|t| t == tid)
                } else {
                    false
                }
            }
            Scope::User => {
                owner_id.is_some_and(|oid| oid == self.user_id)
            }
        }
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
             OR (scope = 'user' AND created_by = $3))"
        );
        assert_eq!(f.next_param(), 4);
    }

    #[test]
    fn access_filter_repository_includes_visibility() {
        let f = test_ctx().access_filter(ScopedEntity::Repository, "", 2);
        assert_eq!(
            f.sql(),
            "($2 OR visibility = 'public' OR scope = 'global' \
             OR (scope = 'team' AND team_id = ANY($3)) \
             OR (scope = 'user' AND created_by = $4))"
        );
        assert_eq!(f.next_param(), 5);
    }

    #[test]
    fn access_filter_channel_config_no_visibility() {
        let f = test_ctx().access_filter(ScopedEntity::ChannelConfig, "", 1);
        assert_eq!(
            f.sql(),
            "($1 OR scope = 'global' \
             OR (scope = 'team' AND team_id = ANY($2)) \
             OR (scope = 'user' AND created_by = $3))"
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
             OR (t.scope = 'user' AND t.created_by = $3))"
        );
    }
}
