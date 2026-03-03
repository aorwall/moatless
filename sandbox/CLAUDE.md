# moatless-sandbox

Rust sandbox runtime for Moatless Vibe sandbox pods. Replaces the TypeScript `apps/sandbox/`.

## Commands

```bash
cargo build -p moatless-sandbox
cargo clippy -p moatless-sandbox -- -D warnings
cargo test -p moatless-sandbox

# E2E tests (requires `claude` CLI + API key, costs ~$0.01-0.05 per test)
CLAUDE_SDK_E2E=1 cargo test -p moatless-sandbox --test agent_e2e -- --ignored --nocapture
```

## Architecture

```
src/
├── lib.rs                     # Library target: re-exports all modules for tests
├── main.rs                    # Entry point: config → services → HTTP /health + gRPC server
├── config.rs                  # Config from env vars (Config::from_env())
├── state.rs                   # SandboxState (shared across handlers)
├── error.rs                   # SandboxError enum + IntoResponse
├── events.rs                  # GrpcEventPublisher (publishes events to backend via gRPC)
├── backend_client.rs          # BackendGrpcClient: gRPC client wrapper for BackendSandboxService
├── grpc_server.rs             # SandboxService gRPC impl (receives commands from backend)
├── agent/
│   ├── mod.rs
│   ├── agent_state.rs         # AgentState: generic state machine (status, session, cancel, prompts)
│   ├── claude_code.rs         # Claude Code executor: wraps claude-agent-sdk query lifecycle
│   ├── options.rs             # Build ClaudeOptions from config + request
│   ├── permission_handler.rs  # InteractivePermissionHandler: AskUserQuestion/ExitPlanMode
│   ├── prompt.rs              # Prompt types: PendingPrompt, PromptResponse, PromptToolName
│   ├── consumer.rs            # process_pending_messages(): one-shot fetch via gRPC
│   └── types.rs               # Internal types (AgentRunRequest, AgentStatus, etc.)
├── server/
│   ├── mod.rs
│   ├── monitor.rs             # ServerProcessMonitor: reads PID/status/log files
│   └── registry.rs            # ServerRegistry: manages monitors
└── files/
    └── mod.rs
tests/
└── agent_e2e.rs               # E2E integration tests (gated behind CLAUDE_SDK_E2E=1)
```

## Communication

### gRPC (primary communication channel)

**Sandbox gRPC Server** (port 50052, `SandboxService`):
- Backend calls sandbox to execute/stop agents, notify messages, check health/status, search files, stream logs

**Sandbox gRPC Client** (`BackendSandboxServiceClient`):
- Sandbox publishes events to backend via `PublishEvent` (replaces Redis XADD)
- Sandbox fetches pending messages via `GetPendingMessages` (replaces HTTP polling)

### HTTP (health probes only)

| Method | Path | Description |
|--------|------|-------------|
| GET | `/health` | K8s liveness/readiness/startup probe |

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| PORT | 8001 | HTTP health probe port |
| GRPC_PORT | 50052 | gRPC server port |
| TASK_ID | (required) | Task identifier |
| WORKSPACE_DIR | /workspace | Working directory |
| DATA_DIR | /opt/moatless/data | Server artifact directory |
| UPLOAD_DIR | /mnt/uploads | Upload directory |
| LOG_LEVEL | info | Tracing log level |
| BACKEND_GRPC_URL | (optional) | Backend gRPC endpoint for events/messages |
| USER_AUTH_TOKEN | (optional) | Auth token for backend calls |
| REPO_CONFIG | (optional) | JSON repository configuration |
| SANDBOX_CLAUDE_EXECUTABLE_PATH | /opt/moatless/bin/claude | Claude CLI path |

## Key Patterns

- **gRPC communication**: All backend↔sandbox communication uses gRPC via `sandbox-proto` crate
- **State sharing**: `SandboxState` via axum `State` extractor (all fields Arc-wrapped)
- **Cancellation**: `CancellationToken` from tokio-util for graceful agent abort
- **Interactive prompts**: `tokio::sync::oneshot` for AskUserQuestion/ExitPlanMode blocking
- **Event publishing**: Fire-and-forget gRPC calls, failures logged but never propagated
- **Message fetching**: Direct invocation via `process_pending_messages()` — called from init, gRPC notify, and post-agent completion
