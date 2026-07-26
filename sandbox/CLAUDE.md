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
│   ├── agent_state.rs         # AgentState: status, session id, current run id, cancel, prompts
│   ├── handle/                # Agent actor: owns the CLI session and exposes send()
│   │   ├── actor.rs           # tokio::select! event loop; mints the per-run run_id
│   │   ├── session.rs         # start_session + tmux session fingerprint
│   │   ├── claude.rs          # Claude SdkMessage -> events
│   │   ├── codex.rs           # Codex event stream -> events
│   │   └── retry.rs           # Transient-error classification and backoff
│   ├── dispatcher.rs          # dispatch_pending_messages(): fetch, send, acknowledge
│   ├── options.rs             # Build ClaudeOptions from config + request
│   ├── permission_handler.rs  # InteractivePermissionHandler: AskUserQuestion/ExitPlanMode
│   ├── prompt.rs              # Prompt types: PendingPrompt, PromptResponse, PromptToolName
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
| WORKSPACE_REPOS | (optional) | JSON array of the workspace's repository checkouts, primary first: `[{cloneUrl, branch, provider, mountName, isPrimary}]`. Each is cloned into `$WORKSPACE_DIR/{mountName}` with its own provider token; the primary one is `repo_dir`. Absent/unparseable → clone `GIT_CLONE_URL` alone |
| SANDBOX_CLAUDE_EXECUTABLE_PATH | /opt/moatless/bin/claude | Claude CLI path |
| FRONTEND_URL | (optional) | Frontend URL for constructing task URLs in agent system prompt |
| SANDBOX_USE_TMUX | false | Use tmux-based Claude SDK transport instead of subprocess/NDJSON |
| SANDBOX_PERMISSION_MODE | bypassPermissions | Claude CLI permission mode for normal (non-plan) subprocess runs: `default`, `acceptEdits`, `bypassPermissions`, `plan`, `delegate`, `dontAsk`. Plan-mode requests always override with `plan`; ignored in tmux mode |

## Key Patterns

- **gRPC communication**: All backend↔sandbox communication uses gRPC via `sandbox-proto` crate
- **State sharing**: `SandboxState` via axum `State` extractor (all fields Arc-wrapped)
- **Cancellation**: `CancellationToken` from tokio-util for graceful agent abort
- **Interactive prompts**: `tokio::sync::oneshot` for AskUserQuestion/ExitPlanMode blocking
- **Event publishing**: Fire-and-forget gRPC calls, failures logged but never propagated
- **Message fetching**: Direct invocation via `dispatch_pending_messages()` — called from init, gRPC notify, and post-agent completion
- **Run identity**: the actor mints a `run_id` (UUID) whenever a run starts while
  the agent is idle, and holds it for the run's lifetime. Every event published
  from inside the run carries it, and so does the message acknowledgment. The
  backend maps `run_id` -> turn number, so a turn is exactly one agent run
  regardless of the order those two channels deliver in
