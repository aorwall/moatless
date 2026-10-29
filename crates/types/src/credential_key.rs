//! Credential key names the backend and the sandbox process both name.
//!
//! The sandbox depends on no backend crate, so a key crossing
//! `GetCredentials` or `UpdateCredential` has its one definition here.

/// Codex `auth.json` blob — a Codex agent's model credential, and the one
/// credential a sandbox writes back after a token refresh.
pub const CODEX_AUTH_JSON: &str = "CODEX_AUTH_JSON";

/// Anthropic API key — a Claude agent's model credential.
pub const ANTHROPIC_API_KEY: &str = "ANTHROPIC_API_KEY";

/// A bearer token for an Anthropic-compatible endpoint, such as a hub.
pub const ANTHROPIC_AUTH_TOKEN: &str = "ANTHROPIC_AUTH_TOKEN";

/// The Anthropic API root a Claude agent calls instead of Anthropic's own.
pub const ANTHROPIC_BASE_URL: &str = "ANTHROPIC_BASE_URL";

/// Long-lived user auth token the sandbox authenticates its gRPC calls back to
/// the backend with.
pub const USER_AUTH_TOKEN: &str = "USER_AUTH_TOKEN";

/// The pod provider's token under one name, derived by the sandbox's
/// `credentials::fetch` and the name `init::git` clones with.
pub const GIT_TOKEN: &str = "GIT_TOKEN";

/// The provider token under the name `gh` reads, derived beside [`GIT_TOKEN`].
pub const GH_TOKEN: &str = "GH_TOKEN";

/// `Name <email>` of the person a Task's work is for, sent only when the
/// sandbox commits as another identity.
pub const GIT_CO_AUTHOR: &str = "GIT_CO_AUTHOR";
