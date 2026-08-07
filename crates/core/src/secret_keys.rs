//! Reserved secret-store key names that lower bands must name.
//!
//! A key lives here once a band below `sandbox` names it. `GITHUB_TOKEN` is
//! named by identity- and workspaces-band credential code; the four
//! model-credential keys are named by the tasks-band agent catalog, which reads
//! them to state each agent's required credentials. The remaining provider keys
//! (GitLab/Gitness tokens, git identity) stay in the sandbox module their only
//! consumers sit beside.

/// The reserved key name for a task's GitHub token secret.
pub const GITHUB_TOKEN: &str = "GITHUB_TOKEN";

/// Anthropic API key — a Claude agent's model credential.
pub const ANTHROPIC_API_KEY: &str = "ANTHROPIC_API_KEY";

/// Claude Code OAuth token — a Claude agent's model credential.
pub const CLAUDE_CODE_OAUTH_TOKEN: &str = "CLAUDE_CODE_OAUTH_TOKEN";

/// Codex `auth.json` blob — a Codex agent's model credential.
pub const CODEX_AUTH_JSON: &str = "CODEX_AUTH_JSON";

/// OpenAI API key — a Codex agent's model credential.
pub const OPENAI_API_KEY: &str = "OPENAI_API_KEY";
