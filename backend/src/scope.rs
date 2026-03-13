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
}
