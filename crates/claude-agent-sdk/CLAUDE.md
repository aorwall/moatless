# claude-agent-sdk

Rust SDK for spawning and communicating with the Claude Code CLI over NDJSON stdin/stdout.

## Commands

```bash
cargo build -p claude-agent-sdk
cargo clippy -p claude-agent-sdk -- -D warnings
cargo test -p claude-agent-sdk
```

## Architecture

```
src/
├── lib.rs                  # Public re-exports
├── error.rs                # SdkError enum (11 variants)
├── query.rs                # query() + query_streaming() + QueryHandle
├── client.rs               # ClaudeClient (multi-turn sessions)
├── types/
│   ├── messages.rs         # SdkMessage enum (16 variants, custom deserialization)
│   ├── content.rs          # ContentBlock, ApiMessage, Usage, ModelUsage
│   ├── options.rs          # ClaudeOptions (35 fields), SystemPrompt, SandboxSettings
│   ├── control.rs          # RawStdoutMessage, control request/response envelopes
│   ├── permissions.rs      # PermissionMode, PermissionHandler trait, PermissionUpdate
│   ├── hooks.rs            # HookEvent (13 variants), HookHandler trait, HookOutput
│   ├── mcp.rs              # McpServerConfig (Stdio/Sse/Http), McpServerStatus
│   └── agents.rs           # AgentDefinition, AgentModel
├── transport/
│   └── subprocess.rs       # SubprocessTransport: spawn, NDJSON read/write, lifecycle
└── protocol/
    ├── orchestrator.rs     # Background reader task, message routing, permission dispatch
    └── request_tracker.rs  # Control request ID correlation with oneshot channels
```

## Key Design Decisions

- **SdkMessage deserialization**: Custom `from_value()` instead of `#[serde(tag)]` because `"system"` type has 8 subtypes and `"result"` branches on success/error
- **Stdin writes**: `Arc<tokio::sync::Mutex<ChildStdin>>` — tokio mutex since writes happen in async context
- **Message channel**: Bounded `mpsc` (256 capacity) from orchestrator to consumer for backpressure
- **Cancellation**: `tokio_util::sync::CancellationToken` checked in `tokio::select!` loops
- **Error handling**: `thiserror` only (library crate, no `anyhow`)

## Reference

Types mirror the TypeScript SDK at `node_modules/@anthropic-ai/claude-agent-sdk/sdk.d.ts` (1777 lines). The existing TS consumer lives at `apps/sandbox/src/features/agents/claude-code/`.

## Dependencies

All workspace-common: tokio, serde, serde_json, thiserror, tracing, uuid, futures, async-trait, tokio-util.
