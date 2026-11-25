# claude-agent-sdk

## Building and testing

```bash
# Build, test and clippy: the root AGENTS.md lines cover this crate.

# E2E, subprocess backend (requires the `claude` CLI and `ANTHROPIC_API_KEY` or
# `CLAUDE_CODE_OAUTH_TOKEN`, ~$0.01-0.05/test)
CLAUDE_SDK_E2E=1 cargo test -p claude-agent-sdk -- --ignored --nocapture
# E2E, tmux backend (additionally requires `tmux` on PATH)
CLAUDE_TMUX_E2E=1 cargo test -p claude-agent-sdk -- --ignored --nocapture
# Both backends in one run
CLAUDE_SDK_E2E=1 CLAUDE_TMUX_E2E=1 cargo test -p claude-agent-sdk -- --ignored --nocapture

# Smoke: version assertion + roundtrip + per-turn --resume (~$0.005, ~30s)
CLAUDE_SDK_E2E=1 cargo test -p claude-agent-sdk --test smoke -- --ignored --nocapture
# Smoke in Docker against the pinned Claude CLI (catches version-pin drift)
./crates/claude-agent-sdk/scripts/test-docker.sh smoke
```

## Adding an E2E test (`tests/e2e/`)

`main.rs` holds the two gating macros and the module list. Put a new test in the
child that owns its subject, and put anything two children need in `harness.rs`.

- Name a paired test `test_<name>_subprocess` and `test_<name>_tmux`. Both share
  one `run_<name>(Backend)` body, and a capability difference gates on
  `Backend::has_cost_data()` or `channel_stays_open_after_result()`.
- Name a subprocess-only test `test_<name>`, with no suffix. Use that form when
  the test exercises a capability tmux lacks. Those are the permission handler,
  real System init, and real cost data.
- Name a tmux-only test `test_<name>_tmux`, the same suffix a paired test takes.
  Both of them live in `tui.rs`, which is where a test that drives the TUI
  across many turns belongs.

## Rules

- Deserialize `SdkMessage` with the custom `from_value()`, not `#[serde(tag)]`.
- Cancel through `tokio_util::sync::CancellationToken` inside `tokio::select!`.
- Use `thiserror` only. This is a library crate and takes no `anyhow`.

## Where the flags and the types come from

- The Claude Code CLI reference at https://code.claude.com/docs/en/cli-reference
  is authoritative for supported flags.
- The types mirror the TypeScript SDK at
  `node_modules/@anthropic-ai/claude-agent-sdk/sdk.d.ts`.
