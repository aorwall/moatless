# claude-agent-sdk

Rust SDK for spawning and communicating with the Claude Code CLI over NDJSON stdin/stdout. Two backends: **subprocess** (`--print`/NDJSON) and **tmux** (drives the TUI, hook-driven).

## Commands

```bash
cargo build -p claude-agent-sdk
cargo test -p claude-agent-sdk
# Clippy: run the floor line from the root AGENTS.md — it covers this crate.

# E2E — subprocess backend (requires `claude` CLI + API key, ~$0.01-0.05/test)
CLAUDE_SDK_E2E=1 cargo test -p claude-agent-sdk -- --ignored --nocapture
# E2E — tmux backend (additionally requires `tmux` on PATH)
CLAUDE_TMUX_E2E=1 cargo test -p claude-agent-sdk -- --ignored --nocapture
# Both backends in one run
CLAUDE_SDK_E2E=1 CLAUDE_TMUX_E2E=1 cargo test -p claude-agent-sdk -- --ignored --nocapture

# Smoke — version assertion + roundtrip + per-turn --resume (~$0.005, ~30s)
CLAUDE_SDK_E2E=1 cargo test -p claude-agent-sdk --test smoke -- --ignored --nocapture
# Smoke in Docker against the pinned Claude CLI (catches version-pin drift)
./crates/claude-agent-sdk/scripts/test-docker.sh smoke
```

### E2E test naming (`tests/e2e.rs`)
- `test_<name>_subprocess` / `test_<name>_tmux` — paired tests sharing a `run_<name>(Backend)` body; capability differences gate on `Backend::has_cost_data()`, `has_real_system_init()`, `channel_stays_open_after_result()`.
- `test_<name>` (no suffix) — subprocess-only, because it exercises a capability tmux lacks (permission handler, hook handler, SDK MCP servers, real System init, real cost data).

The smoke test (`tests/smoke.rs`) asserts the on-PATH `claude --version` matches the pin in `docker/Dockerfile.sandbox-base`, then runs a roundtrip and a 2-turn `--resume <session_id>` (mirroring the sandbox's per-turn dispatch in `crates/sandbox/src/agent/options.rs`). Use as a fast pre-release check.

## Module layout
`query.rs` (`query()` + `query_streaming()` + `QueryHandle`), `client.rs` (`ClaudeClient` multi-turn), `error.rs` (`SdkError`), `mcp/` (in-process `SdkMcpServer` + tools), `api/` (messages, content, options, control, permissions, hooks, mcp, agents), `transport/` (`Transport`/`StdinWriter` traits + `SubprocessTransport` + mocks), `protocol/` (orchestrator: background reader, message/MCP routing, permission/hook dispatch; `request_tracker` for control-request ID correlation), `tmux/` (TUI backend — see below).

## Tmux backend lifecycle (hook-driven)

The TUI lives behind a detached tmux session. Claude's hook system is the
readiness and turn-completion signal.

1. `HookHarness::create` mkfifos `$TMPDIR/claude-sdk-tmux-<pid>-<uuid>/events.fifo` and writes a relay shell script.
2. `--settings '<json>'` registers `SessionStart` + `Stop` hooks that pipe payloads into the FIFO (one `"<event>\t<payload>\n"` line each).
3. A `spawn_blocking` reader opens the FIFO `O_RDWR | O_NONBLOCK` (never sees EOF when the relay disconnects between events) and forwards parsed lines via unbounded mpsc.
4. `spawn_hook_listener` drains that channel and republishes higher-level `HookState` (Pending / SessionStarted / TurnEnded) on a `watch`.
5. `mod.rs` awaits `SessionStarted` (`init_timeout`, default 60s) before pasting the prompt — no hard-coded sleeps. A `trust_dialog_watcher` runs in parallel: polls `tmux capture-pane`, strips CSI, presses Enter if both "trust" and "folder" appear.
6. `stop_fallback_loop` watches for `Stop` events; each gives up to 2s (40 × 50ms) for the tail loop to emit a `Result` from the JSONL `turn_duration` record, else synthesizes `ResultSuccess` from the Stop payload's `last_assistant_message`.

`Stop`'s `transcript_path` also drives JSONL discovery — important for
`--continue`, where claude appends to the existing transcript and no new file
appears. Snapshot-diff stays as a defensive fallback when the Stop payload omits
the path.

## Key design decisions
- **`SdkMessage` deserialization**: custom `from_value()` instead of `#[serde(tag)]` — `"system"` has 8 subtypes and `"result"` branches on success/error.
- **Stdin writes**: `Arc<dyn StdinWriter>` (impl `ChildStdinWriter` over `Arc<Mutex<ChildStdin>>`) — enables mock testing.
- **Transport abstraction**: `Transport` (read) + `StdinWriter` (write) traits; `MockTransport`/`MockStdinWriter` drive the orchestrator tests.
- **Message channel**: bounded mpsc (256) orchestrator → consumer for backpressure.
- **Cancellation**: `tokio_util::sync::CancellationToken` in `tokio::select!` loops.
- **Errors**: `thiserror` only (library crate, no `anyhow`).
- **SDK MCP servers**: orchestrator routes `McpMessage` control requests to the right server's `handle_message()`.
- **Hooks**: orchestrator invokes `HookHandler` for `HookCallback` control requests; auto-responds empty-success if no handler registered.

## Reference
- Claude Code CLI reference: https://code.claude.com/docs/en/cli-reference — authoritative for supported flags.
- Types mirror the TypeScript SDK at `node_modules/@anthropic-ai/claude-agent-sdk/sdk.d.ts`.
