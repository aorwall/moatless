# moatless-sandbox

Rust sandbox runtime for Moatless Vibe sandbox pods.

## Commands

```bash
cargo build -p moatless-sandbox
cargo clippy -p moatless-sandbox -- -D warnings
cargo test -p moatless-sandbox

# E2E tests (requires `claude` CLI + API key, costs ~$0.01-0.05 per test)
CLAUDE_SDK_E2E=1 cargo test -p moatless-sandbox --test agent_e2e -- --ignored --nocapture
```

## Ownership Map

- `main.rs` wires config, credentials, supervisor, HTTP health, gRPC server, and init.
- `agent/coordinator.rs` owns harness-neutral run lifecycle, retry, run IDs, and event publication.
- `agent/harness/` owns Claude and Codex process/app-server adapters behind the Harness boundary.
- `agent/dispatcher.rs` fetches pending messages, submits them to the handle, and acknowledges only accepted input.
- `init/` owns checkout, credentials, repo skill install, transcript restore, workspace restore, and server startup.
- `outbox.rs` owns the sandbox-to-backend event buffer and delivery retry policy.
- `server/` owns configured preview/server process monitoring.

## Communication Rules

- Backend calls the sandbox gRPC server (`SandboxService`, port 50052) to execute/stop agents, notify messages, check health/status, search files, and stream logs.
- Sandbox calls backend gRPC (`BackendSandboxServiceClient`) to publish events, fetch/acknowledge messages, fetch credentials, and sync state archives.
- HTTP is for health probes only. Keep runtime control on gRPC.

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
| SANDBOX_HEADLESS | false | The task runs with nobody watching it (set by the backend for a headless Task). `AskUserQuestion`, `EnterPlanMode` and `ExitPlanMode` are disallowed, so a turn never parks on an answer that is not coming |

## Load-Bearing Rules

- `SandboxState` is shared through axum `State`; keep fields cheap to clone (`Arc` or internally shared).
- Long-lived tasks register with the root supervisor and use lifecycle cancellation.
- Event publishing goes through `EventOutbox`; do not bypass its bounded retry and drop accounting policy.
- `dispatch_pending_messages()` is the only pending-message fetch/submit/ack path; call it from init, gRPC notify, and post-agent completion.
- `CoordinatorService` mints a `run_id` (UUID) whenever a run
  starts while the agent is idle, and holds it for the run's lifetime. Every
  event published from inside the run carries it, and so does the message
  acknowledgment. The backend maps `run_id` -> turn number, so a turn is exactly
  one agent run regardless of delivery order.
- Interactive prompts use `tokio::sync::oneshot` for AskUserQuestion/ExitPlanMode blocking.
- `init/` does not write into a checkout under `WORKSPACE_DIR`. A file it creates there is untracked and lands in the user's `git status` and PR diff. Anything an agent must discover is staged in a home/config dir instead — `$CLAUDE_CONFIG_DIR`, `$CODEX_HOME`, `/etc/codex/skills` — and a repo that wants `CLAUDE.md` beside its `AGENTS.md`, or `.claude/skills` beside `.agents/skills`, commits that symlink itself. The clone, the configured setup commands, and `workspace_sync`'s restore are the only writers inside a checkout.
- Each checked-out repo's root instruction file (`CLAUDE.md`/`AGENTS.md`) is embedded in the agent's system prompt by `agent::options::build_repo_instructions_section` and excluded from both agents' own memory loading — Claude via `claudeMdExcludes` (`init::claude_code_settings`), Codex via `project_doc_max_bytes = 0` (`init::codex_config`). Embedding and exclusion both key off `Config::repo_dirs()`; keep them in sync so the root docs load exactly once.
- AGENTS.md is canonical local guidance.
