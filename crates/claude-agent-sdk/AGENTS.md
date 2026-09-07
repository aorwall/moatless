# claude-agent-sdk

Two backends. The **subprocess** backend uses `--print` and NDJSON. The **tmux**
backend drives the TUI and is hook-driven.

## Building and testing

```bash
cargo build -p claude-agent-sdk
cargo test -p claude-agent-sdk
# Clippy: run the floor line from the root AGENTS.md. It covers this crate.

# E2E, subprocess backend (requires the `claude` CLI + an API key, ~$0.01-0.05/test)
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

## Naming an E2E test (`tests/e2e.rs`)

- Name a paired test `test_<name>_subprocess` and `test_<name>_tmux`. Both share
  one `run_<name>(Backend)` body, and a capability difference gates on
  `Backend::has_cost_data()`, `has_real_system_init()` or
  `channel_stays_open_after_result()`.
- Name a subprocess-only test `test_<name>`, with no suffix. Use that form when
  the test exercises a capability tmux lacks. Those are the permission handler,
  the hook handler, SDK MCP servers, real System init, and real cost data.

The smoke test (`tests/smoke.rs`) asserts that the on-PATH `claude --version`
matches the pin in `docker/Dockerfile.sandbox-base`. It then runs a roundtrip
and a two-turn `--resume <session_id>`, which mirrors the sandbox's per-turn
dispatch in `crates/sandbox/src/agent/options.rs`. Use it as a fast pre-release
check.

## What each module owns

| Module | What it owns |
|---|---|
| `query.rs` | `query()`, `query_streaming()` and `QueryHandle` |
| `client.rs` | `ClaudeClient`, the multi-turn client |
| `error.rs` | `SdkError` |
| `mcp/` | The in-process `SdkMcpServer` and its tools |
| `api/` | Messages, content, options, control, permissions, hooks, mcp, agents |
| `transport/` | The `Transport` and `StdinWriter` traits, `SubprocessTransport`, and the mocks |
| `protocol/` | The orchestrator: background reader, message and MCP routing, permission and hook dispatch, and `request_tracker` for control-request ID correlation |
| `tmux/` | The TUI backend |
| `transcripts.rs` | Transcript paths, inventory and resume selection shared by both backends |

## How the tmux backend starts and ends a turn

The TUI lives behind a detached tmux session. Claude's hook system is the
readiness signal and the turn-completion signal.

1. `HookHarness::create` mkfifos
   `$TMPDIR/claude-sdk-tmux-<pid>-<uuid>/events.fifo` and writes a relay shell
   script.
2. `--settings '<json>'` registers `SessionStart` and `Stop` hooks that pipe
   their payloads into the FIFO, one `"<event>\t<payload>\n"` line each.
3. A `spawn_blocking` reader opens the FIFO `O_RDWR | O_NONBLOCK`, so it never
   sees EOF when the relay disconnects between events. It forwards each parsed
   line over an unbounded mpsc.
4. `spawn_hook_listener` drains that channel and republishes a higher-level
   `HookState` on a `watch`: Pending, SessionStarted or TurnEnded.
5. `tmux/startup.rs` awaits `SessionStarted` before `tmux/query.rs` pastes the prompt, under
   `init_timeout` (60s by default). No step sleeps for a fixed time. A
   `trust_dialog_watcher` runs in parallel: it polls `tmux capture-pane`, strips
   CSI, and presses Enter when both "trust" and "folder" appear.
6. `stop_fallback_loop` watches for a `Stop` event. Each one gives the tail loop
   up to 2s (40 × 50ms) to emit a `Result` from the JSONL `turn_duration`
   record. Past that it synthesizes `ResultSuccess` from the Stop payload's
   `last_assistant_message`.

`Stop`'s `transcript_path` also drives JSONL discovery. That matters for
`--continue`, where claude appends to the existing transcript and no new file
appears. Snapshot-diff stays as a defensive fallback for a Stop payload that
omits the path.

## Design decisions to preserve

- **`SdkMessage` deserialization** uses a custom `from_value()` instead of
  `#[serde(tag)]`. `"system"` has 8 subtypes, and `"result"` branches on success
  and error.
- **Stdin writes** go through `Arc<dyn StdinWriter>`, implemented as
  `ChildStdinWriter` over `Arc<Mutex<ChildStdin>>`. That enables mock testing.
- **The transport abstraction** splits reads (`Transport`) from writes
  (`StdinWriter`). `MockTransport` and `MockStdinWriter` drive the orchestrator
  tests.
- **The message channel** is a bounded mpsc of 256, from orchestrator to
  consumer, for backpressure.
- **Cancellation** uses `tokio_util::sync::CancellationToken` inside
  `tokio::select!` loops.
- **Errors** use `thiserror` only. This is a library crate, so it takes no
  `anyhow`.
- **SDK MCP servers**: the orchestrator routes an `McpMessage` control request
  to the right server's `handle_message()`.
- **Hooks**: the orchestrator invokes `HookHandler` for a `HookCallback` control
  request. It auto-responds with empty success when no handler is registered.

## Where the flags and the types come from

- The Claude Code CLI reference at https://code.claude.com/docs/en/cli-reference
  is authoritative for supported flags.
- The types mirror the TypeScript SDK at
  `node_modules/@anthropic-ai/claude-agent-sdk/sdk.d.ts`.
