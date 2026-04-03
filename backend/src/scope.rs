//! Access control scope types.
//!
//! The `Scope` enum is re-exported from `moatless-api-schemas`.
//! `ScopeContext` is backend-only (used for query filtering).

pub use moatless_api_schemas::enums::Scope;

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
    /// Generate a SQL WHERE clause fragment for access control.
    ///
    /// Uses 3 positional parameters starting at `start`:
    ///   - `$start`: `is_admin` (bool)
    ///   - `$start+1`: `team_ids` (text[])
    ///   - `$start+2`: `user_id` (text)
    ///
    /// Caller must `.bind()` these from `self.is_admin`, `self.team_ids`, `self.user_id`.
    ///
    /// `prefix` is the table alias (e.g. `""` or `"t."`).
    /// `with_visibility` adds `OR visibility = 'public'` so public resources bypass scope.
    pub fn sql_filter(&self, prefix: &str, start: usize, with_visibility: bool) -> String {
        let (p1, p2, p3) = (start, start + 1, start + 2);
        let vis = if with_visibility {
            format!(" OR {prefix}visibility = 'public'")
        } else {
            String::new()
        };
        format!(
            "(${p1}{vis} OR {prefix}scope = 'global' \
             OR ({prefix}scope = 'team' AND {prefix}team_id = ANY(${p2})) \
             OR ({prefix}scope = 'user' AND {prefix}created_by = ${p3}))"
        )
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

    #[test]
    fn sql_filter_without_visibility() {
        let ctx = ScopeContext {
            user_id: "u1".into(),
            team_ids: vec![],
            is_admin: false,
        };
        let sql = ctx.sql_filter("", 1, false);
        assert_eq!(
            sql,
            "($1 OR scope = 'global' \
             OR (scope = 'team' AND team_id = ANY($2)) \
             OR (scope = 'user' AND created_by = $3))"
        );
    }

    #[test]
    fn sql_filter_with_visibility() {
        let ctx = ScopeContext {
            user_id: "u1".into(),
            team_ids: vec![],
            is_admin: false,
        };
        let sql = ctx.sql_filter("t.", 2, true);
        assert_eq!(
            sql,
            "($2 OR t.visibility = 'public' OR t.scope = 'global' \
             OR (t.scope = 'team' AND t.team_id = ANY($3)) \
             OR (t.scope = 'user' AND t.created_by = $4))"
        );
    }
}
