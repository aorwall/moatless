//! Scope-aware addressing for Codex `provider_tokens` rows.
//!
//! [`CodexScope`] is the discriminant that drives every scope-aware operation
//! in the Codex refresh pipeline: the SQL `WHERE` fragment that targets the
//! right row, the Postgres advisory-lock key, and the in-process per-row mutex
//! key. The variants intentionally carry the scope identifier so a single
//! value uniquely addresses one row without callers having to thread
//! `(scope_kind, scope_id)` tuples through every layer.
//!
//! Two scopes:
//! - [`CodexScope::User`] — per-user row keyed by `user_id`.
//! - [`CodexScope::Global`] — single global row: `user_id IS NULL`,
//!   `team_id IS NULL`, `scope = 'global'`.
//!
//! Resolver precedence: `User > Global` (most-specific wins).

/// Discriminator + identifier for a Codex `provider_tokens` row's scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodexScope {
    /// Per-user row: `(user_id = id, scope = 'user')`.
    User(String),
    /// Singleton global row: `(user_id IS NULL, team_id IS NULL, scope = 'global')`.
    Global,
}

impl CodexScope {
    /// Convenience constructors.
    pub fn user(id: impl Into<String>) -> Self {
        Self::User(id.into())
    }

    /// The string the Postgres advisory lock is hashed from. Includes the
    /// scope discriminant so a user named `"foo"` doesn't collide with the
    /// global lock.
    pub fn advisory_id_input(&self) -> String {
        match self {
            Self::User(id) => format!("codex-refresh:user:{id}"),
            Self::Global => "codex-refresh:global".to_string(),
        }
    }

    /// Alias for [`advisory_id_input`]; reads naturally at call sites that
    /// also use it as the in-process mutex discriminator.
    pub fn lock_key(&self) -> String {
        self.advisory_id_input()
    }

    /// Short human-readable label for log fields (`scope=user`, `scope=global`).
    pub fn display(&self) -> String {
        match self {
            Self::User(id) => format!("user:{id}"),
            Self::Global => "global".to_string(),
        }
    }

    /// Bind value for the `WHERE` fragment's single `$N` placeholder.
    /// User scope carries the user_id; Global uses an empty string sentinel
    /// (the clause itself uses IS NULL checks, not a bound value).
    pub fn bind_value(&self) -> &str {
        match self {
            Self::User(id) => id.as_str(),
            Self::Global => "",
        }
    }

    /// Build the `WHERE` clause fragment that addresses this scope's row in
    /// `provider_tokens`, using `placeholder` for the dynamic identifier.
    /// Callers paste the result into the larger SQL string and bind
    /// [`bind_value`] at the same positional parameter index.
    ///
    /// Note: Global scope uses `IS NULL` checks so the bind value is unused
    /// for that variant; callers should still bind it (empty string) to keep
    /// positional parameter counts consistent.
    pub fn where_clause(&self, placeholder: &str) -> String {
        match self {
            Self::User(_) => format!(
                "user_id = {placeholder} AND provider = 'codex' AND scope = 'user'"
            ),
            Self::Global => "user_id IS NULL AND team_id IS NULL AND provider = 'codex' AND scope = 'global'".to_string(),
        }
    }

    /// Scope kind as the literal stored in `provider_tokens.scope`.
    pub fn scope_kind(&self) -> &'static str {
        match self {
            Self::User(_) => "user",
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
        let global = CodexScope::Global.lock_key();
        assert_ne!(user, global);
        assert!(user.contains("user"));
        assert!(global.contains("global"));
    }

    #[test]
    fn where_clause_per_scope_is_well_formed() {
        let u = CodexScope::user("u1").where_clause("$1");
        assert!(u.contains("user_id = $1"));
        assert!(u.contains("scope = 'user'"));

        let g = CodexScope::Global.where_clause("$1");
        assert!(g.contains("user_id IS NULL"));
        assert!(g.contains("team_id IS NULL"));
        assert!(g.contains("scope = 'global'"));
    }

    #[test]
    fn bind_value_is_the_scope_identifier() {
        assert_eq!(CodexScope::user("alice").bind_value(), "alice");
        assert_eq!(CodexScope::Global.bind_value(), "");
    }

    #[test]
    fn scope_kind_matches_db_literal() {
        assert_eq!(CodexScope::user("a").scope_kind(), "user");
        assert_eq!(CodexScope::Global.scope_kind(), "global");
    }

    #[test]
    fn display_is_compact() {
        assert_eq!(CodexScope::user("a").display(), "user:a");
        assert_eq!(CodexScope::Global.display(), "global");
    }
}
