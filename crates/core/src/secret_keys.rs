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

/// The reserved key name for the Forgejo token a Workspace pins.
///
/// Single-valued, unlike the per-host secrets [`forgejo_token_key`] names: a
/// pin is the token every call on that Workspace presents, and a Workspace
/// spanning two Forgejo hosts authenticates to only one of them.
pub const FORGEJO_TOKEN: &str = "FORGEJO_TOKEN";

/// The secret key holding `host`'s Forgejo personal access token.
///
/// Forgejo is self-hostable and one person may hold a token per instance, so
/// the host is part of the key rather than a column beside it.
pub fn forgejo_token_key(host: &str) -> String {
    format!("{FORGEJO_TOKEN}_{}", host_key_segment(host))
}

/// The secret key holding the Forgejo OAuth credential `host`'s personal access
/// token displaced, so removing the token restores a live credential.
///
/// Mirrors the GitHub stash in `moatless_identity::auth::credentials`.
pub fn forgejo_oauth_stash_key(host: &str) -> String {
    format!("FORGEJO_OAUTH_ACCESS_TOKEN_{}", host_key_segment(host))
}

/// `host` as a key segment: an alphanumeric byte uppercased, every other
/// byte escaped as `_` plus two hex digits.
///
/// Fixed-width escaping keeps two hosts from colliding — mapping separators
/// to plain `_` instead merged `git.acme.dev` with `git-acme.dev`,
/// overwriting one host's token with the other's.
fn host_key_segment(host: &str) -> String {
    const HEX: [u8; 16] = *b"0123456789ABCDEF";

    let mut segment = String::with_capacity(host.len());
    for byte in host.bytes() {
        if byte.is_ascii_alphanumeric() {
            segment.push(byte.to_ascii_uppercase() as char);
        } else {
            segment.push('_');
            segment.push(HEX[usize::from(byte >> 4)] as char);
            segment.push(HEX[usize::from(byte & 0x0f)] as char);
        }
    }
    segment
}

/// Whether `key` names a Forgejo credential: the Workspace pin
/// [`FORGEJO_TOKEN`], a per-host [`forgejo_token_key`], or a per-host
/// [`forgejo_oauth_stash_key`].
pub fn is_forgejo_credential_key(key: &str) -> bool {
    key == FORGEJO_TOKEN
        || key.starts_with("FORGEJO_TOKEN_")
        || key.starts_with("FORGEJO_OAUTH_ACCESS_TOKEN_")
}

/// Anthropic API key — a Claude agent's model credential.
pub const ANTHROPIC_API_KEY: &str = "ANTHROPIC_API_KEY";

/// Claude Code OAuth token — a Claude agent's model credential.
pub const CLAUDE_CODE_OAUTH_TOKEN: &str = "CLAUDE_CODE_OAUTH_TOKEN";

pub use moatless_types::credential_key::CODEX_AUTH_JSON;
use moatless_types::credential_key::USER_AUTH_TOKEN;

/// OpenAI API key — a Codex agent's model credential.
pub const OPENAI_API_KEY: &str = "OPENAI_API_KEY";

/// Key names controlled by the platform/runtime.
pub const RESERVED_SECRET_KEYS: &[&str] = &[
    "TASK_ID",
    "BACKEND_GRPC_URL",
    "BACKEND_API_URL",
    USER_AUTH_TOKEN,
    "TASK_TOKEN_SECRET",
    "SESSION_JWT_SECRET",
    "DATABASE_URL",
    "KUBERNETES_SERVICE_HOST",
    // The AWS SDK reads these from the environment before anything else. A
    // Workspace that set them would redirect the workspace repository's writes
    // to an account of its choosing.
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
    "AWS_SESSION_TOKEN",
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
    fn a_forgejo_host_becomes_a_storable_key() {
        // A raw host carries dots and dashes, which `is_valid_secret_key`
        // rejects — a key built from one would never store.
        assert_eq!(
            forgejo_token_key("git.acme.dev"),
            "FORGEJO_TOKEN_GIT_2EACME_2EDEV"
        );
        assert_eq!(
            forgejo_oauth_stash_key("git-eu.acme.dev"),
            "FORGEJO_OAUTH_ACCESS_TOKEN_GIT_2DEU_2EACME_2EDEV"
        );
        for key in [
            forgejo_token_key("codeberg.org"),
            forgejo_oauth_stash_key("codeberg.org"),
            forgejo_token_key("git.acme.dev:3000"),
        ] {
            assert!(is_valid_secret_key(&key), "{key} must store");
        }
    }

    #[test]
    fn every_forgejo_key_shape_is_recognised() {
        for key in [
            FORGEJO_TOKEN.to_string(),
            forgejo_token_key("codeberg.org"),
            forgejo_oauth_stash_key("codeberg.org"),
        ] {
            assert!(is_forgejo_credential_key(&key), "{key}");
        }
        for key in [GITHUB_TOKEN, GITLAB_TOKEN, "FORGEJO_HOST"] {
            assert!(!is_forgejo_credential_key(key), "{key}");
        }
    }

    /// Two hosts differing only in a separator are both registrable — the
    /// `forgejo_instances` host check admits the dot and the dash alike — so a
    /// shared key would hand one instance's holder the other's token.
    #[test]
    fn two_forgejo_hosts_never_share_a_key() {
        let hosts = [
            "git.acme.dev",
            "git.other.dev",
            "git-acme.dev",
            "git.acme.dev:3000",
            "a.d.b",
            "a-b",
        ];
        for (i, left) in hosts.iter().enumerate() {
            for right in &hosts[i + 1..] {
                assert_ne!(
                    forgejo_token_key(left),
                    forgejo_token_key(right),
                    "{left} and {right}"
                );
                assert_ne!(
                    forgejo_oauth_stash_key(left),
                    forgejo_oauth_stash_key(right),
                    "{left} and {right}"
                );
            }
        }
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
