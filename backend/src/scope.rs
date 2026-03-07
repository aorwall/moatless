//! Access control scope types.
//!
//! Defines the 3-level scope model (Global / Team / User) and the
//! `ScopeContext` used to filter queries by access level.

use serde::{Deserialize, Serialize};

/// 3-level scope for repositories, tasks, channels, and credentials.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// Visible to all authenticated users.
    Global,
    /// Visible to team members.
    Team,
    /// Visible only to the creator.
    User,
}

impl Scope {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Team => "team",
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
    fn try_from(s: &str) -> Result<Self, String> {
        match s {
            "global" => Ok(Self::Global),
            "team" => Ok(Self::Team),
            "user" => Ok(Self::User),
            other => Err(format!("unknown Scope: {other:?}")),
        }
    }
}

impl sqlx::Type<sqlx::Postgres> for Scope {
    fn type_info() -> sqlx::postgres::PgTypeInfo {
        <String as sqlx::Type<sqlx::Postgres>>::type_info()
    }
}

impl<'r> sqlx::Decode<'r, sqlx::Postgres> for Scope {
    fn decode(
        value: sqlx::postgres::PgValueRef<'r>,
    ) -> Result<Self, sqlx::error::BoxDynError> {
        let s = <String as sqlx::Decode<sqlx::Postgres>>::decode(value)?;
        Scope::try_from(s.as_str()).map_err(Into::into)
    }
}

impl sqlx::Encode<'_, sqlx::Postgres> for Scope {
    fn encode_by_ref(
        &self,
        buf: &mut sqlx::postgres::PgArgumentBuffer,
    ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
        <&str as sqlx::Encode<sqlx::Postgres>>::encode_by_ref(&self.as_str(), buf)
    }
}

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
