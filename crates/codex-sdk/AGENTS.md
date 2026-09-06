# codex-sdk

The sandbox runtime does not use this crate. The sandbox runs Codex through the
`codex app-server` CLI in `sandbox/src/agent/codex_app_server/`. This crate is
a standalone in-process SDK.

## How it maps onto claude-agent-sdk

`query()` and `query_streaming()` match. The rest maps like this:

| codex-sdk | claude-agent-sdk |
|---|---|
| `CodexClient` | `ClaudeClient` |
| `CodexMessage` | `SdkMessage` |
| `CodexOptions` | `ClaudeOptions` |
| `ApprovalHandler` | `PermissionHandler` |
| `HookObserver` (read-only, because Codex hooks run server-side) | `HookHandler` |

Codex has no equivalent of `SdkMcpServer` and its in-process MCP tools,
`AgentDefinition` and sub-agents, the runtime `set_model()` and
`set_permission_mode()`, or `max_budget_usd`.

Codex adds four things of its own:

- `QueryHandle::steer()`, to inject context mid-turn.
- Rich sandbox policies, for file and network isolation.
- Three separate approval types: command, file change, permissions.
- Thread management: list, resume, fork.

## How the crate is wired

- **Transport**: the in-process `InProcessAppServerClient`, from
  `codex-app-server-client`.
- **Orchestrator**: a background tokio task that maps a `ServerNotification` to a
  `CodexMessage` and routes a `ServerRequest` approval to `ApprovalHandler`.
- **Dependencies**: path deps to the codex repo at
  `~/repos/stuffs/codex/codex-rs/`.

## Building against the codex repo

This is a **standalone workspace**, excluded from the parent moatless
workspace. The codex dependency graph needs its own `[patch.crates-io]` entries
and a pinned Rust toolchain (1.93+). Check out the codex repo at
`~/repos/stuffs/codex/` first. **Do not delete `Cargo.lock`**: it was seeded
from the codex workspace for compatible resolution.

```bash
cd crates/codex-sdk
cargo build
cargo build --examples
cargo clippy -- -D warnings
```

## Running the tests

```bash
cargo test                                                    # unit tests, no API key (~24 tests)
CODEX_SDK_E2E=1 cargo test --ignored -- --nocapture           # E2E (needs OPENAI_API_KEY/codex login, ~$0.01-0.05/test)
CODEX_SDK_E2E=1 cargo test --ignored test_simple_query -- --nocapture   # single E2E
CODEX_SDK_E2E=1 cargo test --test smoke -- --ignored --nocapture        # smoke: roundtrip + multi-turn + resume (~30s, ~$0.02)
```

E2E tests live in `tests/e2e.rs`, each gated behind `CODEX_SDK_E2E=1` and
`#[ignore]`. The smoke resume test uses a custom 8MB-stack tokio runtime. The
in-process app-server's deep call chains overflow the default 2MB stack in a
debug build.

## Writing a first query

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

More examples: `examples/simple_query.rs`, `multi_turn.rs`, `approvals.rs`.
