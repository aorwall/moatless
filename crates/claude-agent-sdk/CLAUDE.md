# claude-agent-sdk

Rust SDK for spawning and communicating with the Claude Code CLI over NDJSON stdin/stdout.

## Commands

```bash
cargo build -p claude-agent-sdk
cargo clippy -p claude-agent-sdk -- -D warnings
cargo test -p claude-agent-sdk

# E2E tests — subprocess backend (requires `claude` CLI + API key, costs ~$0.01-0.05 per test)
CLAUDE_SDK_E2E=1 cargo test -p claude-agent-sdk -- --ignored --nocapture

# E2E tests — tmux backend (additionally requires `tmux` binary on PATH)
CLAUDE_TMUX_E2E=1 cargo test -p claude-agent-sdk -- --ignored --nocapture

# Both backends in one run
CLAUDE_SDK_E2E=1 CLAUDE_TMUX_E2E=1 cargo test -p claude-agent-sdk -- --ignored --nocapture
```

E2E test naming convention (`tests/e2e.rs`):
- `test_<name>_subprocess` / `test_<name>_tmux` — paired tests that share a
  `run_<name>(Backend)` body; capability differences are gated on
  `Backend::has_cost_data()`, `has_real_system_init()`, and
  `channel_stays_open_after_result()`.
- `test_<name>` (no suffix) — subprocess-only because the test exercises a
  capability the tmux backend doesn't support (permission handler, hook
  handler, SDK MCP servers, real System init, real cost data).

## Architecture

```
src/
├── lib.rs                  # Public re-exports
├── error.rs                # SdkError enum (11 variants)
├── query.rs                # query() + query_streaming() + QueryHandle
├── client.rs               # ClaudeClient (multi-turn sessions)
├── mcp/
│   ├── mod.rs              # Module root, re-exports
│   ├── server.rs           # SdkMcpServer: in-process MCP server with JSON-RPC dispatch
│   └── tool.rs             # SdkMcpTool, ToolHandler, ToolResult
├── types/
│   ├── messages.rs         # SdkMessage enum (16 variants, custom deserialization)
│   ├── content.rs          # ContentBlock, ApiMessage, Usage, ModelUsage
│   ├── options.rs          # ClaudeOptions (35+ fields), SystemPrompt, SandboxSettings
│   ├── control.rs          # RawStdoutMessage, control request/response envelopes
│   ├── permissions.rs      # PermissionMode, PermissionHandler trait, PermissionUpdate
│   ├── hooks.rs            # HookEvent (13 variants), HookHandler trait, HookOutput
│   ├── mcp.rs              # McpServerConfig (Stdio/Sse/Http), McpServerStatus
│   └── agents.rs           # AgentDefinition, AgentModel
├── transport/
│   ├── mod.rs              # Transport + StdinWriter traits, ChildStdinWriter, mock impls
│   └── subprocess.rs       # SubprocessTransport: spawn, NDJSON read/write, lifecycle
└── protocol/
    ├── orchestrator.rs     # Background reader task, message/MCP routing, permission/hook dispatch
    ├── orchestrator_tests.rs # 28 orchestrator tests (permissions, hooks, MCP, control, EOF)
    └── request_tracker.rs  # Control request ID correlation with oneshot channels
```

## Key Design Decisions

- **SdkMessage deserialization**: Custom `from_value()` instead of `#[serde(tag)]` because `"system"` type has 8 subtypes and `"result"` branches on success/error
- **Stdin writes**: `Arc<dyn StdinWriter>` trait (impl: `ChildStdinWriter` wrapping `Arc<Mutex<ChildStdin>>`) — enables mock testing
- **Transport abstraction**: `Transport` trait for reading, `StdinWriter` trait for writing — `MockTransport`/`MockStdinWriter` used in 28 orchestrator tests
- **Message channel**: Bounded `mpsc` (256 capacity) from orchestrator to consumer for backpressure
- **Cancellation**: `tokio_util::sync::CancellationToken` checked in `tokio::select!` loops
- **Error handling**: `thiserror` only (library crate, no `anyhow`)
- **SDK MCP Servers**: In-process tool hosting via `SdkMcpServer` — orchestrator routes `McpMessage` control requests to the correct server's `handle_message()` method
- **Hook system**: `HookHandler` trait invoked by orchestrator for `HookCallback` control requests; auto-responds with empty success if no handler registered

## Reference

- **Claude Code CLI reference**: https://code.claude.com/docs/en/cli-reference — authoritative source for supported flags, options, and usage patterns
- Types mirror the TypeScript SDK at `node_modules/@anthropic-ai/claude-agent-sdk/sdk.d.ts` (1777 lines). The existing TS consumer lives at `apps/sandbox/src/features/agents/claude-code/`.

## Dependencies

All workspace-common: tokio, serde, serde_json, thiserror, tracing, uuid, futures, async-trait, tokio-util.
