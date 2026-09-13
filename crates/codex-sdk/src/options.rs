use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use codex_app_server_protocol::AskForApproval;
use codex_app_server_protocol::SandboxMode;
use tokio_util::sync::CancellationToken;
use toml::Value as TomlValue;

use crate::approvals::ApprovalHandler;
use crate::hooks::HookObserver;

/// Configuration options for a Codex SDK session.
///
/// This is the Codex equivalent of the Claude SDK's `ClaudeOptions`.
#[derive(Clone, Default)]
pub struct CodexOptions {
    // -- Model --
    /// Model slug (e.g. "o3", "gpt-4o"). Uses config default if `None`.
    pub model: Option<String>,
    /// Model provider (e.g. "openai", "oss").
    pub model_provider: Option<String>,

    // -- Instructions --
    /// Base system instructions (replaces the default system prompt).
    pub base_instructions: Option<String>,
    /// Developer instructions appended after the base prompt.
    pub developer_instructions: Option<String>,

    // -- Permissions / Sandbox --
    /// Approval policy for command/file operations.
    pub approval_policy: Option<AskForApproval>,
    /// Sandbox mode controlling file/network access.
    pub sandbox_mode: Option<SandboxMode>,

    // -- Limits --
    /// Maximum successful turn submissions, including the initial prompt.
    /// `None` is unlimited; zero admits none. Concurrent sends are serialized;
    /// failed submissions do not consume the limit.
    pub max_turns: Option<u32>,

    // -- Directories --
    /// Working directory for the agent. Defaults to current directory.
    pub cwd: Option<PathBuf>,

    // -- Thread --
    /// Resume an existing thread by ID instead of creating a new one.
    pub resume_thread_id: Option<String>,
    /// When true, the thread is not persisted to disk.
    pub ephemeral: Option<bool>,

    // -- Config overrides --
    /// Escape hatch: raw config.toml overrides as `(dotted.key, value)` pairs.
    /// These are passed directly to `ConfigBuilder`.
    pub config_overrides: Vec<(String, TomlValue)>,

    // -- Runtime callbacks --
    /// Handler for approval requests (command execution, file changes,
    /// permissions). If `None`, all approval requests are auto-approved.
    pub approval_handler: Option<Arc<dyn ApprovalHandler>>,
    /// Read-only observer for hook events. If `None`, hooks are silently
    /// ignored.
    pub hook_observer: Option<Arc<dyn HookObserver>>,
    /// Cancellation token for cooperative shutdown.
    pub cancellation_token: Option<CancellationToken>,

    // -- Timeouts --
    /// Timeout for runtime initialization (default: 60s). Timeout errors wait
    /// for runtime cleanup, bounded to five additional seconds.
    pub init_timeout: Option<Duration>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_options_all_none() {
        let opts = CodexOptions::default();
        assert!(opts.model.is_none());
        assert!(opts.model_provider.is_none());
        assert!(opts.base_instructions.is_none());
        assert!(opts.developer_instructions.is_none());
        assert!(opts.approval_policy.is_none());
        assert!(opts.sandbox_mode.is_none());
        assert!(opts.max_turns.is_none());
        assert!(opts.cwd.is_none());
        assert!(opts.resume_thread_id.is_none());
        assert!(opts.ephemeral.is_none());
        assert!(opts.config_overrides.is_empty());
        assert!(opts.approval_handler.is_none());
        assert!(opts.hook_observer.is_none());
        assert!(opts.cancellation_token.is_none());
        assert!(opts.init_timeout.is_none());
    }
}
