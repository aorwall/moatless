#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "tests panic on failure and print diagnostics"
)]
#![expect(
    clippy::ignore_without_reason,
    reason = "the module doc gives the one reason every test is ignored"
)]
//! End-to-end tests against the real `claude` CLI, one module per capability.
//!
//! Every test here spawns the CLI and makes real API calls, so each is
//! `#[ignore]`d and gated on `CLAUDE_SDK_E2E=1` for the subprocess backend or
//! `CLAUDE_TMUX_E2E=1` for the tmux one. `crates/claude-agent-sdk/AGENTS.md`
//! holds the run commands, what they require and how to name a test.

/// Skip subprocess-backed test unless `CLAUDE_SDK_E2E=1` is set.
macro_rules! skip_unless_subprocess_e2e {
    () => {
        if std::env::var("CLAUDE_SDK_E2E").as_deref() != Ok("1") {
            eprintln!("Skipping subprocess E2E test (set CLAUDE_SDK_E2E=1 to run)");
            return;
        }
    };
}

/// Skip tmux-backed test unless `CLAUDE_TMUX_E2E=1` is set AND `tmux` binary is
/// on PATH.
macro_rules! skip_unless_tmux_e2e {
    () => {
        if std::env::var("CLAUDE_TMUX_E2E").as_deref() != Ok("1") {
            eprintln!("Skipping tmux E2E test (set CLAUDE_TMUX_E2E=1 to run)");
            return;
        }
        if !crate::harness::tmux_is_available() {
            eprintln!("Skipping tmux E2E test: `tmux` binary not found on PATH");
            return;
        }
    };
}

mod diagnostics;
mod harness;
mod permissions;
mod query;
mod sessions;
mod streaming;
mod subagents;
mod tui;
