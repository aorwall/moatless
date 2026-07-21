//! Scope-aware addressing for GitHub `provider_tokens` rows.
//!
//! [`GitHubScope`] uniquely addresses one row: the SQL `WHERE` fragment that
//! targets it and the single identifier bound into that fragment.
//!
//! Three scopes:
//! - [`GitHubScope::User`] — per-user row keyed by `user_id`.
//! - [`GitHubScope::Team`] — per-team row keyed by `team_id` (`user_id IS NULL`).
//! - [`GitHubScope::Global`] — single org-wide row stored at `team_id = 'system'`,
//!   `user_id IS NULL`. The 'system' team is seeded by migration `0049`.
//!
//! Resolver precedence: `User > Team > Global` (most-specific wins).
//!
//! Deliberately a separate type from [`CodexScope`] rather than a generic
//! `ProviderScope`: genericising would drag the codex refresh pipeline
//! (advisory locks, CAS, the healthy-row sweep) along for no gain here.
//!
//! [`CodexScope`]: crate::credentials::codex::scope::CodexScope

pub use crate::credentials::codex::scope::SYSTEM_TEAM_ID;

/// Discriminator + identifier for a GitHub `provider_tokens` row's scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitHubScope {
    /// Per-user row: `(user_id = id, scope = 'user')`.
    User(String),
    /// Per-team row: `(team_id = id, user_id IS NULL, scope = 'team')`.
    Team(String),
    /// Singleton org-wide row: `(team_id = 'system', user_id IS NULL, scope = 'global')`.
    Global,
}

impl GitHubScope {
    pub fn user(id: impl Into<String>) -> Self {
        Self::User(id.into())
    }

    pub fn team(id: impl Into<String>) -> Self {
        Self::Team(id.into())
    }

    /// Parse the `(scope, team_id)` pair read back from `provider_tokens`.
    /// Unknown scope literals fall back to `User`, matching the resolver's
    /// treatment of rows written before the scope column was populated.
    pub fn from_row(scope: &str, user_id: Option<&str>, team_id: Option<&str>) -> Self {
        match scope {
            "global" => Self::Global,
            "team" => Self::Team(team_id.unwrap_or_default().to_string()),
            _ => Self::User(user_id.unwrap_or_default().to_string()),
        }
    }

    /// Short human-readable label for log fields (`scope=user:alice`,
    /// `scope=team:acme`, `scope=global`).
    pub fn display(&self) -> String {
        match self {
            Self::User(id) => format!("user:{id}"),
            Self::Team(id) => format!("team:{id}"),
            Self::Global => "global".to_string(),
        }
    }

    /// Bind value for the `WHERE` fragment's single `$N` placeholder. All three
    /// scopes carry exactly one identifier (user_id, team_id, or the reserved
    /// `'system'` literal for global), so callers bind uniformly without
    /// branching on the variant.
    pub fn bind_value(&self) -> &str {
        match self {
            Self::User(id) | Self::Team(id) => id.as_str(),
            Self::Global => SYSTEM_TEAM_ID,
        }
    }

    /// Build the `WHERE` clause fragment addressing this scope's row in
    /// `provider_tokens`, using `placeholder` for the dynamic identifier.
    /// Callers paste the result into the larger SQL string and bind
    /// [`bind_value`](Self::bind_value) at the same positional index.
    pub fn where_clause(&self, placeholder: &str) -> String {
        match self {
            Self::User(_) => {
                format!("user_id = {placeholder} AND provider = 'github' AND scope = 'user'")
            }
            Self::Team(_) => format!(
                "team_id = {placeholder} AND user_id IS NULL AND provider = 'github' AND scope = 'team'"
            ),
            Self::Global => format!(
                "team_id = {placeholder} AND user_id IS NULL AND provider = 'github' AND scope = 'global'"
            ),
        }
    }

    /// Scope kind as the literal stored in `provider_tokens.scope`.
    pub fn scope_kind(&self) -> &'static str {
        match self {
            Self::User(_) => "user",
            Self::Team(_) => "team",
            Self::Global => "global",
        }
    }
}

impl std::fmt::Display for GitHubScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.display())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn where_clause_per_scope_is_well_formed() {
        let u = GitHubScope::user("u1").where_clause("$1");
        assert!(u.contains("user_id = $1"));
        assert!(u.contains("scope = 'user'"));

        let t = GitHubScope::team("t1").where_clause("$3");
        assert!(t.contains("team_id = $3"));
        assert!(t.contains("user_id IS NULL"));
        assert!(t.contains("scope = 'team'"));

        let g = GitHubScope::Global.where_clause("$1");
        assert!(g.contains("team_id = $1"));
        assert!(g.contains("user_id IS NULL"));
        assert!(g.contains("scope = 'global'"));
    }

    /// Guards against the copy-paste that would leave `provider = 'codex'` in
    /// a fragment ported from `CodexScope`.
    #[test]
    fn every_where_clause_filters_on_the_github_provider() {
        for scope in [
            GitHubScope::user("a"),
            GitHubScope::team("b"),
            GitHubScope::Global,
        ] {
            let clause = scope.where_clause("$1");
            assert!(clause.contains("provider = 'github'"), "{clause}");
            assert!(!clause.contains("codex"), "{clause}");
        }
    }

    #[test]
    fn bind_value_is_the_scope_identifier() {
        assert_eq!(GitHubScope::user("alice").bind_value(), "alice");
        assert_eq!(GitHubScope::team("acme").bind_value(), "acme");
        assert_eq!(GitHubScope::Global.bind_value(), SYSTEM_TEAM_ID);
    }

    #[test]
    fn scope_kind_matches_db_literal() {
        assert_eq!(GitHubScope::user("a").scope_kind(), "user");
        assert_eq!(GitHubScope::team("a").scope_kind(), "team");
        assert_eq!(GitHubScope::Global.scope_kind(), "global");
    }

    #[test]
    fn display_is_compact() {
        assert_eq!(GitHubScope::user("a").display(), "user:a");
        assert_eq!(GitHubScope::team("b").display(), "team:b");
        assert_eq!(GitHubScope::Global.display(), "global");
    }

    #[test]
    fn from_row_round_trips_each_scope() {
        assert_eq!(
            GitHubScope::from_row("user", Some("u1"), None),
            GitHubScope::user("u1")
        );
        assert_eq!(
            GitHubScope::from_row("team", None, Some("t1")),
            GitHubScope::team("t1")
        );
        assert_eq!(
            GitHubScope::from_row("global", None, Some(SYSTEM_TEAM_ID)),
            GitHubScope::Global
        );
        // Unrecognised literal degrades to user scope rather than panicking.
        assert_eq!(
            GitHubScope::from_row("", Some("u1"), None),
            GitHubScope::user("u1")
        );
    }
}
