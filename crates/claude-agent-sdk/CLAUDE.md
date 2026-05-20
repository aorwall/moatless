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

# Smoke test — version assertion + single roundtrip + per-turn --resume (~$0.005, ~30s)
CLAUDE_SDK_E2E=1 cargo test -p claude-agent-sdk --test smoke -- --ignored --nocapture

# Smoke test in Docker against the pinned Claude CLI (catches version pin drift)
./crates/claude-agent-sdk/scripts/test-docker.sh smoke
```

E2E test naming convention (`tests/e2e.rs`):
- `test_<name>_subprocess` / `test_<name>_tmux` — paired tests that share a
  `run_<name>(Backend)` body; capability differences are gated on
  `Backend::has_cost_data()`, `has_real_system_init()`, and
  `channel_stays_open_after_result()`.
- `test_<name>` (no suffix) — subprocess-only because the test exercises a
  capability the tmux backend doesn't support (permission handler, hook
  handler, SDK MCP servers, real System init, real cost data).

Smoke test (`tests/smoke.rs`) is a separate integration binary that asserts the
on-PATH `claude --version` matches the pin in `docker/Dockerfile.sandbox-base`,
then runs a single roundtrip and a 2-turn `--resume <session_id>` (mirroring the
sandbox's per-turn dispatch model in `sandbox/src/agent/options.rs:151-163`).
Use it as a fast pre-release check.

## Architecture

```
src/
├── lib.rs                  # Public re-exports
├── error.rs                # SdkError enum (incl. tmux-specific: TmuxNotFound, TmuxSessionDied, HookFifoFailed, HookHarnessFailed, SessionStartTimeout, JsonlNotFound)
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
├── protocol/
│   ├── orchestrator.rs     # Background reader task, message/MCP routing, permission/hook dispatch
│   ├── orchestrator_tests.rs # 28 orchestrator tests (permissions, hooks, MCP, control, EOF)
│   └── request_tracker.rs  # Control request ID correlation with oneshot channels
└── tmux/
    ├── mod.rs              # query_tmux + query_tmux_streaming entry points; SessionStart wait
    ├── hooks.rs            # HookHarness: per-run FIFO + relay script + inline --settings JSON
    ├── session.rs          # TmuxSession spawn/kill (with SIGTERM→200ms→SIGKILL escalation)
    ├── tailer.rs           # Hook listener, JSONL tail loop, liveness loop, Stop fallback loop
    ├── translator.rs       # JSONL record → SdkMessage; turn_duration → ResultSuccess
    ├── trust.rs            # CSI stripping + workspace-trust dialog detection
    ├── writer.rs           # tmux paste-buffer + send-keys for prompt delivery
    └── args.rs             # `claude` CLI argv for TUI mode (no --print/--output-format)
```

## Tmux backend lifecycle (hook-driven)

The TUI lives behind a [tmux] detached session. We use Claude's hook system
to know when the TUI is ready and when each turn finishes — replacing the
older 500ms-sleep + snapshot-diff approach.

1. [`HookHarness::create`] mkfifos `$TMPDIR/claude-sdk-tmux-<pid>-<uuid>/events.fifo`
   and writes a tiny relay shell script.
2. `--settings '<json>'` registers `SessionStart` + `Stop` hooks that pipe their
   payloads into the FIFO (one line each: `"<event>\t<payload>\n"`).
3. A `spawn_blocking` reader opens the FIFO `O_RDWR | O_NONBLOCK` (so it never
   sees EOF when the relay script disconnects between events) and forwards
   parsed lines via an unbounded mpsc channel.
4. `spawn_hook_listener` drains that channel and republishes higher-level
   [`HookState`] (Pending / SessionStarted / TurnEnded) on a `tokio::sync::watch`.
5. `mod.rs` awaits `SessionStarted` (with `init_timeout`, default 60s) before
   pasting the prompt — no hard-coded sleeps. A `trust_dialog_watcher` runs in
   parallel during this wait window; it polls `tmux capture-pane`, strips CSI,
   and presses Enter if both "trust" and "folder" appear.
6. The supervisor's `stop_fallback_loop` watches for `Stop` events. Each Stop
   gives up to 2s (40 × 50ms) for the tail loop to emit a `Result` from the
   JSONL `turn_duration` record; if the budget elapses, it synthesizes a
   `ResultSuccess` from the Stop payload's `last_assistant_message`.

`Stop`'s `transcript_path` also drives JSONL discovery — important for
`--continue`, where claude appends to the existing transcript and snapshot-diff
would see no new file. Snapshot-diff against the pre-spawn JSONL listing stays
as a defensive fallback for older Claude versions whose Stop payload omits the
path.

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
