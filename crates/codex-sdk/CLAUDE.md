# codex-sdk

Rust SDK for the [Codex](https://github.com/openai/codex) coding agent, mirroring the `claude-agent-sdk` interface.

## Overview

This crate provides an async API for programmatically running Codex sessions from Rust. It uses Codex's **in-process app-server runtime** (not subprocess spawning) for full protocol access including streaming deltas, approval handling, and hook observation.

## Relationship to claude-agent-sdk

This SDK mirrors the Claude Agent SDK's interface patterns:

| Claude Agent SDK | codex-sdk | Notes |
|---|---|---|
| `query(prompt, opts)` | `query(prompt, opts)` | One-shot query |
| `query_streaming(opts)` | `query_streaming(opts)` | Multi-turn session |
| `ClaudeClient` | `CodexClient` | High-level `send()`/`recv()` wrapper |
| `QueryHandle` | `QueryHandle` | Message stream + control |
| `SdkMessage` | `CodexMessage` | Event enum (~20 variants) |
| `ClaudeOptions` | `CodexOptions` | Configuration struct |
| `PermissionHandler` | `ApprovalHandler` | 3 methods vs Claude's 1 |
| `HookHandler` | `HookObserver` | Read-only (Codex hooks run server-side) |

### Not supported (no Codex equivalent)
- `SdkMcpServer` / in-process MCP tools
- `AgentDefinition` / sub-agents
- Runtime `set_model()` / `set_permission_mode()`
- `max_budget_usd`

### Codex-specific features
- `QueryHandle::steer()` — inject context mid-turn
- Rich sandbox policies (file/network isolation)
- Three separate approval types (command, file change, permissions)
- Thread management (list/resume/fork)

## Architecture

- **Transport**: In-process via `InProcessAppServerClient` (from `codex-app-server-client`)
- **Orchestrator**: Background tokio task maps `ServerNotification` -> `CodexMessage` and routes `ServerRequest` (approvals) to `ApprovalHandler`
- **Dependencies**: Path dependencies to the codex repo at `~/repos/stuffs/codex/codex-rs/`

## Key files

- `src/lib.rs` — Public API re-exports
- `src/options.rs` — `CodexOptions` configuration
- `src/messages.rs` — `CodexMessage` enum mapped from `ServerNotification`
- `src/approvals.rs` — `ApprovalHandler` trait + `AutoApproveHandler` / `DenyAllHandler`
- `src/hooks.rs` — `HookObserver` trait (read-only)
- `src/query.rs` — `query()`, `query_streaming()`, `QueryHandle`
- `src/client.rs` — `CodexClient` convenience wrapper
- `src/runtime.rs` — Internal startup boilerplate (Config -> InProcessClientStartArgs)

## Quick start

```rust
use codex_sdk::{query, CodexOptions, CodexMessage};

let mut handle = query("List files", CodexOptions::default()).await?;
while let Some(msg) = handle.messages.recv().await {
    match msg? {
        CodexMessage::AgentMessageDelta(d) => print!("{}", d.delta),
        CodexMessage::TurnCompleted(_) => break,
        _ => {}
    }
}
```

## Building

This crate is a **standalone workspace** (excluded from the parent moatless workspace) because the codex dependency graph requires its own `[patch.crates-io]` entries and a pinned Rust toolchain (1.93+).

Requires the codex repo to be checked out at `~/repos/stuffs/codex/`.

```bash
cd crates/codex-sdk
cargo build
cargo build --examples
```

The `Cargo.lock` was seeded from the codex workspace to ensure compatible dependency resolution. Do not delete it.

## Testing

```bash
# Unit tests (no API key required, ~24 tests)
cd crates/codex-sdk
cargo test

# Clippy
cargo clippy -- -D warnings

# E2E tests (requires OPENAI_API_KEY or codex login, costs ~$0.01-0.05 per test)
CODEX_SDK_E2E=1 cargo test --ignored -- --nocapture

# Single E2E test
CODEX_SDK_E2E=1 cargo test --ignored test_simple_query -- --nocapture
```

### Unit tests (no runtime)
- `src/error.rs` — 5 tests: SdkError Display formatting
- `src/options.rs` — 1 test: CodexOptions::default() verification
- `src/approvals.rs` — 4 tests: AutoApproveHandler and DenyAllHandler decisions
- `src/messages.rs` — 14 tests: CodexMessage::from(ServerNotification) mapping for all variant categories

### E2E tests (`tests/e2e.rs`)
All gated behind `CODEX_SDK_E2E=1` + `#[ignore]`:
- `test_simple_query` — Basic message flow: AgentMessageDelta → TurnCompleted
- `test_thread_id_is_valid` — Thread ID non-empty
- `test_cancel_stops_session` — Cancel causes channel close
- `test_multi_turn_via_codex_client` — CodexClient multi-turn conversation with context retention
- `test_approval_handler_is_invoked` — Custom CapturingApprovalHandler records invocations
- `test_deny_handler_blocks_execution` — DenyAllHandler prevents file writes
- `test_hook_observer_receives_events` — CapturingHookObserver wiring validation
- `test_ephemeral_thread` — Ephemeral thread flag propagation

## Examples

- `examples/simple_query.rs` — One-shot query
- `examples/multi_turn.rs` — Multi-turn conversation
- `examples/approvals.rs` — Custom approval handler
