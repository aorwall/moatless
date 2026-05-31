//! Scope-aware addressing for Codex `provider_tokens` rows.
//!
//! [`CodexScope`] is the discriminant that drives every scope-aware operation
//! in the Codex refresh pipeline: the SQL `WHERE` fragment that targets the
//! right row, the Postgres advisory-lock key, and the in-process per-row mutex
//! key. The variants intentionally carry the scope identifier so a single
//! value uniquely addresses one row without callers having to thread
//! `(scope_kind, scope_id)` tuples through every layer.
//!
//! Three scopes:
//! - [`CodexScope::User`] — per-user row keyed by `user_id`.
//! - [`CodexScope::Team`] — per-team row keyed by `team_id` (`user_id IS NULL`).
//! - [`CodexScope::Global`] — single global row stored at `team_id = 'system'`,
//!   `user_id IS NULL`. The 'system' team is seeded by migration `0049`.
//!
//! Resolver precedence: `User > Team > Global` (most-specific wins).

/// Discriminator + identifier for a Codex `provider_tokens` row's scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodexScope {
    /// Per-user row: `(user_id = id, scope = 'user')`.
    User(String),
    /// Per-team row: `(team_id = id, user_id IS NULL, scope = 'team')`.
    Team(String),
    /// Singleton global row: `(team_id = 'system', user_id IS NULL, scope = 'global')`.
    Global,
}

/// The reserved team id used as the `team_id` of the global codex row.
/// Matches the row seeded by migration `0049_canonical_access_control.sql`.
pub const SYSTEM_TEAM_ID: &str = "system";

impl CodexScope {
    /// Convenience constructors.
    pub fn user(id: impl Into<String>) -> Self {
        Self::User(id.into())
    }

    pub fn team(id: impl Into<String>) -> Self {
        Self::Team(id.into())
    }

    /// The string the Postgres advisory lock is hashed from. Includes the
    /// scope discriminant so a user named `"foo"` and a team named `"foo"`
    /// don't collide on the same lock.
    pub fn advisory_id_input(&self) -> String {
        match self {
            Self::User(id) => format!("codex-refresh:user:{id}"),
            Self::Team(id) => format!("codex-refresh:team:{id}"),
            Self::Global => "codex-refresh:global".to_string(),
        }
    }

    /// Alias for [`advisory_id_input`]; reads naturally at call sites that
    /// also use it as the in-process mutex discriminator.
    pub fn lock_key(&self) -> String {
        self.advisory_id_input()
    }

    /// Short human-readable label for log fields (`scope=user`, `scope=team:42`,
    /// `scope=global`).
    pub fn display(&self) -> String {
        match self {
            Self::User(id) => format!("user:{id}"),
            Self::Team(id) => format!("team:{id}"),
            Self::Global => "global".to_string(),
        }
    }

    /// Bind value for the `WHERE` fragment's single `$N` placeholder. All
    /// three scopes carry exactly one identifier in the fragment (user_id,
    /// team_id, or the reserved `'system'` literal for global), so callers
    /// can bind uniformly via [`bind_value`] without branching on the
    /// variant.
    pub fn bind_value(&self) -> &str {
        match self {
            Self::User(id) | Self::Team(id) => id.as_str(),
            Self::Global => SYSTEM_TEAM_ID,
        }
    }

    /// Build the `WHERE` clause fragment that addresses this scope's row in
    /// `provider_tokens`, using `placeholder` for the dynamic identifier.
    /// Callers paste the result into the larger SQL string and bind
    /// [`bind_value`] at the same positional parameter index.
    ///
    /// All three scopes use exactly one bind, so the positional indexing of
    /// surrounding params is independent of variant.
    pub fn where_clause(&self, placeholder: &str) -> String {
        match self {
            Self::User(_) => format!(
                "user_id = {placeholder} AND provider = 'codex' AND scope = 'user'"
            ),
            Self::Team(_) => format!(
                "team_id = {placeholder} AND user_id IS NULL AND provider = 'codex' AND scope = 'team'"
            ),
            Self::Global => format!(
                "team_id = {placeholder} AND user_id IS NULL AND provider = 'codex' AND scope = 'global'"
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

impl std::fmt::Display for CodexScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.display())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_key_is_distinct_across_scopes_with_same_id() {
        let user = CodexScope::user("foo").lock_key();
        let team = CodexScope::team("foo").lock_key();
        let global = CodexScope::Global.lock_key();
        assert_ne!(user, team);
        assert_ne!(user, global);
        assert_ne!(team, global);
        assert!(user.contains("user"));
        assert!(team.contains("team"));
        assert!(global.contains("global"));
    }

    #[test]
    fn where_clause_per_scope_is_well_formed() {
        let u = CodexScope::user("u1").where_clause("$1");
        assert!(u.contains("user_id = $1"));
        assert!(u.contains("scope = 'user'"));

        let t = CodexScope::team("t1").where_clause("$3");
        assert!(t.contains("team_id = $3"));
        assert!(t.contains("user_id IS NULL"));
        assert!(t.contains("scope = 'team'"));

        let g = CodexScope::Global.where_clause("$1");
        assert!(g.contains("team_id = $1"));
        assert!(g.contains("user_id IS NULL"));
        assert!(g.contains("scope = 'global'"));
    }

    #[test]
    fn bind_value_is_the_scope_identifier() {
        assert_eq!(CodexScope::user("alice").bind_value(), "alice");
        assert_eq!(CodexScope::team("acme").bind_value(), "acme");
        assert_eq!(CodexScope::Global.bind_value(), SYSTEM_TEAM_ID);
    }

    #[test]
    fn scope_kind_matches_db_literal() {
        assert_eq!(CodexScope::user("a").scope_kind(), "user");
        assert_eq!(CodexScope::team("a").scope_kind(), "team");
        assert_eq!(CodexScope::Global.scope_kind(), "global");
    }

    #[test]
    fn display_is_compact() {
        assert_eq!(CodexScope::user("a").display(), "user:a");
        assert_eq!(CodexScope::team("b").display(), "team:b");
        assert_eq!(CodexScope::Global.display(), "global");
    }
}
