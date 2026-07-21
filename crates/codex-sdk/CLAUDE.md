# codex-sdk

Rust SDK for the [Codex](https://github.com/openai/codex) coding agent, mirroring the `claude-agent-sdk` interface. Uses Codex's **in-process app-server runtime** (not subprocess spawning) for full protocol access: streaming deltas, approval handling, hook observation.

> Not used by the sandbox runtime — the sandbox runs Codex via the `codex exec` CLI in `sandbox/src/agent/codex_exec.rs`. This crate is a standalone in-process SDK.

## Relationship to claude-agent-sdk

Mirrors the Claude Agent SDK's interface: `query()`/`query_streaming()` match; `CodexClient` ≈ `ClaudeClient`, `CodexMessage` ≈ `SdkMessage`, `CodexOptions` ≈ `ClaudeOptions`, `ApprovalHandler` ≈ `PermissionHandler`, `HookObserver` ≈ `HookHandler` (read-only — Codex hooks run server-side).

**No Codex equivalent**: `SdkMcpServer`/in-process MCP tools, `AgentDefinition`/sub-agents, runtime `set_model()`/`set_permission_mode()`, `max_budget_usd`.

**Codex-specific**: `QueryHandle::steer()` (inject context mid-turn), rich sandbox policies (file/network isolation), three separate approval types (command, file change, permissions), thread management (list/resume/fork).

## Architecture
- **Transport**: in-process `InProcessAppServerClient` (from `codex-app-server-client`)
- **Orchestrator**: background tokio task maps `ServerNotification` → `CodexMessage`, routes `ServerRequest` (approvals) to `ApprovalHandler`
- **Dependencies**: path deps to the codex repo at `~/repos/stuffs/codex/codex-rs/`

## Building
**Standalone workspace** (excluded from the parent moatless workspace) — the codex dependency graph needs its own `[patch.crates-io]` entries and a pinned Rust toolchain (1.93+). Requires the codex repo checked out at `~/repos/stuffs/codex/`. `Cargo.lock` was seeded from the codex workspace for compatible resolution — **do not delete it**.

```bash
cd crates/codex-sdk
cargo build
cargo build --examples
cargo clippy -- -D warnings
```

## Testing
```bash
cargo test                                                    # unit tests, no API key (~24 tests)
CODEX_SDK_E2E=1 cargo test --ignored -- --nocapture           # E2E (needs OPENAI_API_KEY/codex login, ~$0.01-0.05/test)
CODEX_SDK_E2E=1 cargo test --ignored test_simple_query -- --nocapture   # single E2E
CODEX_SDK_E2E=1 cargo test --test smoke -- --ignored --nocapture        # smoke: roundtrip + multi-turn + resume (~30s, ~$0.02)
```
E2E tests live in `tests/e2e.rs`, all gated behind `CODEX_SDK_E2E=1` + `#[ignore]`. The smoke resume test uses a custom 8MB-stack tokio runtime — the in-process app-server's deep call chains overflow the default 2MB stack in debug builds.

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
Examples: `examples/simple_query.rs`, `multi_turn.rs`, `approvals.rs`.
