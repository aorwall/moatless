//! Reserved secret-store key names that lower bands must name, and the
//! predicates over them.
//!
//! A key lives here once a band below `sandbox` names it. The three git-host
//! token keys are named by identity- and workspaces-band credential code; the
//! four model-credential keys are named by the tasks-band agent catalog, which
//! reads them to state each agent's required credentials. The git identity keys
//! stay in the sandbox module their only consumers sit beside.

/// The reserved key name for a task's GitHub token secret.
pub const GITHUB_TOKEN: &str = "GITHUB_TOKEN";

/// The reserved key name for a task's GitLab token secret.
pub const GITLAB_TOKEN: &str = "GITLAB_TOKEN";

/// The reserved key name for a task's Gitness token secret.
pub const GITNESS_TOKEN: &str = "GITNESS_TOKEN";

/// Anthropic API key — a Claude agent's model credential.
pub const ANTHROPIC_API_KEY: &str = "ANTHROPIC_API_KEY";

/// Claude Code OAuth token — a Claude agent's model credential.
pub const CLAUDE_CODE_OAUTH_TOKEN: &str = "CLAUDE_CODE_OAUTH_TOKEN";

/// Codex `auth.json` blob — a Codex agent's model credential.
pub const CODEX_AUTH_JSON: &str = "CODEX_AUTH_JSON";

/// OpenAI API key — a Codex agent's model credential.
pub const OPENAI_API_KEY: &str = "OPENAI_API_KEY";

/// Key names controlled by the platform/runtime.
pub const RESERVED_SECRET_KEYS: &[&str] = &[
    "TASK_ID",
    "BACKEND_GRPC_URL",
    "BACKEND_API_URL",
    "USER_AUTH_TOKEN",
    "TASK_TOKEN_SECRET",
    "SESSION_JWT_SECRET",
    "DATABASE_URL",
    "KUBERNETES_SERVICE_HOST",
];

/// Env-style key validation: `^[A-Z_][A-Z0-9_]*$`.
pub fn is_valid_secret_key(key: &str) -> bool {
    let mut chars = key.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first == '_' || first.is_ascii_uppercase()) {
        return false;
    }
    chars.all(|c| c == '_' || c.is_ascii_uppercase() || c.is_ascii_digit())
}

pub fn is_reserved_key(key: &str) -> bool {
    RESERVED_SECRET_KEYS.contains(&key)
}

/// Detect keys that should be moved from repository env vars into secrets.
pub fn looks_like_secret_key(key: &str) -> bool {
    let upper = key.to_ascii_uppercase();
    is_reserved_key(&upper)
        || upper.contains("SECRET")
        || upper.contains("TOKEN")
        || upper.contains("PASSWORD")
        || upper.contains("PRIVATE_KEY")
        || upper.ends_with("_API_KEY")
        || upper.ends_with("_KEY")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_secret_key_format() {
        assert!(is_valid_secret_key("STRIPE_SECRET_KEY"));
        assert!(is_valid_secret_key("_TOKEN"));
        assert!(is_valid_secret_key("A1"));
        assert!(!is_valid_secret_key(""));
        assert!(!is_valid_secret_key("stripe_key"));
        assert!(!is_valid_secret_key("1TOKEN"));
        assert!(!is_valid_secret_key("BAD-KEY"));
    }

    #[test]
    fn detects_likely_secret_keys() {
        assert!(looks_like_secret_key("STRIPE_SECRET_KEY"));
        assert!(looks_like_secret_key("OPENAI_API_KEY"));
        assert!(looks_like_secret_key("TASK_TOKEN_SECRET"));
        assert!(!looks_like_secret_key("NODE_ENV"));
        assert!(!looks_like_secret_key("PUBLIC_URL"));
    }
}
