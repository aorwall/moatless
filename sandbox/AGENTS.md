# moatless-sandbox

Rust sandbox runtime for Moatless Vibe sandbox pods.

## Commands

```bash
cargo build -p moatless-sandbox
cargo test -p moatless-sandbox
# Clippy: run the floor line from the root AGENTS.md — it covers this crate.

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
| DATA_DIR | /opt/moatless/data | Server artifact directory; mounted into every server sidecar, so no credential file belongs here |
| UPLOAD_DIR | `$WORKSPACE_DIR/uploads` | Where a message's attached uploads are written. Set by the backend from its `file_mount_path` |
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
- A message's attached uploads are written under `UPLOAD_DIR`, which sits inside `WORKSPACE_DIR` (`$WORKSPACE_DIR/uploads`). That placement is what makes them survive a restart: `workspace_sync` archives loose directories as well as repos, so a restored pod already holds the bytes and `files::uploads::ensure_upload_on_disk` skips the gRPC re-fetch. It is a sibling of the repo checkouts, not a path inside one, so it stays out of the user's `git status`. The backend owns the value — it renders the same path into the prompt's "Saved to:" line and passes it to the pod as `UPLOAD_DIR` — so do not give the sandbox an independent default that could drift from what the prompt advertises.
- Credential material on disk belongs in `RUNTIME_DIR` (`/run/moatless`, `Config::runtime_dir`) — the credential broker socket and the persisted user auth token. It is the only directory that is neither archived into the workspace snapshot nor mounted into the server sidecars, so a file there cannot ride a snapshot into a pod it was not minted for and cannot be read by the repo code those sidecars run. It is not configurable: `docker/moatless-git-cred.sh` hardcodes the broker socket beneath it. Not `WORKSPACE_DIR` (`workspace_sync` archives and restores it) and not `DATA_DIR` (every sidecar mounts it). `init::credentials::purge_exposed_user_tokens` sweeps both on every successful fetch. Note the sandbox and the agent subprocess share a container and uid, so this bounds exposure rather than making a file unreadable to the agent.
- No single store reliably holds the freshest backend auth token, so init tries them all. The persisted copy is rewritten on every credential refresh but lives only as long as `RUNTIME_DIR` does, while `USER_AUTH_TOKEN` and the bootstrap token are minted when the pod or container was created and then fixed. `Config::initial_auth_token_candidates` lists them, and on an `Unauthenticated` rejection of the initial `GetCredentials` init presents the next one; only once every candidate is refused is the failure permanent (exit 78), because at that point re-provisioning is the sole remaining fix. Do not collapse this to a fixed precedence — whichever store loses is a crash loop on that runtime.
- Claude CLI env defaults live in `agent::options::CLAUDE_ENV_DEFAULTS` and are applied with `or_insert`, under whatever the credential channel resolved for the task, so a global- or user-scoped `env`-kind secret of the same key wins. They must not move into `$CLAUDE_CONFIG_DIR/settings.json`: that file's `env` block outranks the process environment, so a default written there cannot be overridden at all. `init::claude_code_settings` therefore writes no `env` block. The same rule bars credentials from that file — it is neither `RUNTIME_DIR` nor readable-only by the agent.
- Each checked-out repo's root instruction file (`CLAUDE.md`/`AGENTS.md`) is inlined into the agent's system prompt by `agent::options::build_repo_instructions_section`, and excluded from both agents' own memory loading — Claude via `claudeMdExcludes` (`init::claude_code_settings`), Codex via `project_doc_max_bytes = 0` (`init::codex_config`). The section and the exclusion both key off `Config::repo_dirs()`; keep them in sync. Inlining is the default because being pointed at a path does not reliably get it read; `SANDBOX_FRONTLOAD_REPO_INSTRUCTIONS=false`, or `frontloadInstructions` in REPO_CONFIG, reverts to listing paths (`AgentConfig::frontload_instructions_enabled`). Read the file through `agent::options::read_capped`: the size cap has to bound the I/O itself, because the path is repo-controlled and resolves through symlinks, so reading first and trimming after would let a checkout choose how much memory prompt assembly spends.
- A delivery's `skills` are names, never bodies. `agent::skills::frontload_skills` resolves each against `init::home_skills::REPO_SKILL_ROOTS` under every `Config::repo_dirs()` entry, then the harness's own home skills dir (`$CLAUDE_CONFIG_DIR/skills` or `$CODEX_HOME/skills`), and prepends `Activated <root>/<name>:` plus the `SKILL.md`. Both modules key off that one const — keep them in sync. A name that resolves nowhere is warned and skipped; the turn still runs. A delivery carrying only skills and no message text is enough to open a turn: `dispatcher` gates on `text.is_empty() && skills.is_empty()`, frontloads, then re-gates on the frontloaded text so a delivery whose every skill failed to resolve still opens nothing.
- AGENTS.md is canonical local guidance.
