# Sandbox Architecture

## Table of Contents

1. [Overview](#overview)
2. [Container Layout](#container-layout)
3. [Startup Sequence](#startup-sequence)
4. [Configuration](#configuration)
5. [HTTP API](#http-api)
6. [Features](#features)
   - [Agent Execution](#agent-execution)
   - [File Operations](#file-operations)
   - [Git Operations](#git-operations)
   - [Scripts](#scripts)
   - [Browser Automation](#browser-automation)
   - [Workspace Provisioning](#workspace-provisioning)
7. [Skills System](#skills-system)
8. [Event System](#event-system)
9. [Communication with Backend](#communication-with-backend)
10. [Docker Image](#docker-image)

---

## Overview

The sandbox is an isolated per-task execution environment that runs the Claude Code agent and supporting services. It provides file operations, git integration, Script execution, and browser automation—all scoped to a single task's workspace.

- **Runtime**: Native Rust binary (axum HTTP + tonic gRPC server)
- **HTTP Port**: 8001 (health probes only)
- **gRPC Port**: 50052 (all backend↔sandbox communication)
- **Deployment**: Docker container (local dev) or Kubernetes pod (production)
- **Source**: `sandbox/` (Rust crate), proto definitions in `crates/sandbox-proto/`

Each sandbox instance is ephemeral and bound to exactly one task. The backend creates a new container/pod per task and communicates with it over gRPC.

For the control-plane side — how the backend provisions, watches, idle-stops,
recovers, and destroys sandboxes — see [Sandbox Lifecycle & Handling](./sandbox-lifecycle.md).

---

## Container Layout

```
/
├── usr/local/bin/
│   ├── sandbox                          # Rust sandbox binary
│   ├── claude                           # Claude Code CLI
│   └── docker-entrypoint-sandbox.sh     # Container entrypoint
│
├── opt/moatless/
│   ├── workspace/                       # Workspace root, may be volume-mounted
│   │   └── <primary-mount>/             # Primary repo checkout
│   ├── data/                            # Runtime data
│   │   ├── events/                      # Persisted events
│   │   └── screenshots/                 # Browser screenshots
│   ├── .claude/                         # Claude Code configuration
│   │   └── skills/                      # Staged image skills + the repo's own, copied at startup
│   ├── .codex/
│   │   └── skills/                      # Same set, for Codex
│
├── usr/share/moatless-skills/           # Skills staged by downstream images (empty in the base image)
│   └── <skill-name>/
│       └── SKILL.md
```

**Volume Mount Consideration**: The workspace directory (`/opt/moatless/workspace`) may be volume-mounted for persistent storage. The sandbox binary lives at `/usr/local/bin/sandbox` to avoid being overwritten by workspace volume mounts.

---

## Startup Sequence

The container entrypoint (`docker-entrypoint-sandbox.sh`) orchestrates parallel startup of multiple components:

```
Container Start
│
├─ 1. Create directories
│     /opt/moatless/workspace, /opt/moatless/.claude, /opt/moatless/data
│
├─ 2. Symlink skills
│     /usr/share/moatless-skills → /opt/moatless/.claude/skills
│
├─ 3. Configure git
│     Set GITHUB_TOKEN credential helper, user.name, user.email
│
├─ 4. Clone repositories (every entry in WORKSPACE_REPOS whose mount lacks .git/,
│     falling back to GIT_CLONE_URL alone)
│     gh repo clone → checkout the entry's branch
│
├─ 5. Start sandbox application (background)
│     /usr/local/bin/sandbox &
│
├─ 6. Run setup commands (from REPO_CONFIG.codeExecution.setupCommands)
│     Execute sequentially in workspace directory
│
└─ 7. Wait for sandbox process
      wait $SANDBOX_PID
```

A `runOnStart` Script is not started by this entrypoint — `crates/t3code/src/startup_scripts.rs`
starts it once the backend observes the Task's Sandbox report Ready.

The sandbox application (`index.ts`) startup:

```
main()
│
├─ Load configuration from environment
├─ Initialize SandboxEventService (Redis transport)
│     • Emits events to Redis Streams (task-events:{taskId})
│
├─ Initialize AgentRequestConsumer (Redis Streams consumer)
│     • Polls agent-requests:{taskId} for execution requests
│     • Primary agent execution path (replaces HTTP-based execution)
│     • Start consumer loop in background
│
├─ Create Hono app with routes
│   ├─ Workspace routes
│   ├─ File routes
│   ├─ Git routes
│   ├─ Agent routes (HTTP fallback, secondary execution path)
│   └─ Browser routes
│
└─ Start HTTP server on PORT
```

---

## Configuration

### Environment Variables

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `TASK_ID` | Yes | — | Task this sandbox is scoped to |
| `PORT` | No | `8001` | HTTP server port |
| `WORKSPACE_DIR` | No | `/opt/moatless/workspace` | Workspace root; repository checkouts are placed below this path |
| `DATA_DIR` | No | `/opt/moatless/data` | Data directory for runtime artifacts |
| `LOG_LEVEL` | No | `info` | Pino log level |
| `GIT_CLONE_URL` | No | — | Primary repository URL to clone on startup |
| `GIT_BRANCH` | No | — | Branch to checkout after clone (primary repo) |
| `WORKSPACE_REPOS` | No | — | JSON array of every repository the workspace places, primary first (see below) |
| `GITHUB_TOKEN` | No | — | Token for private repository access |
| `REPO_CONFIG` | No | — | JSON-encoded repository configuration |
| `CLAUDE_CODE_OAUTH_TOKEN` | No | — | OAuth token for Claude Code agent |
| `CLAUDE_BIN_DIR` | No | `/usr/bin` | Directory containing Claude CLI binary |
| `CLAUDE_CONFIG_DIR` | No | `/opt/moatless/.claude` | Claude configuration directory |
| `BACKEND_GRPC_URL` | No | — | Backend gRPC endpoint for events/messages |
| `GRPC_PORT` | No | `50052` | gRPC server port for SandboxService |

### WORKSPACE_REPOS Structure

A task's workspace may place several repositories in the same sandbox. The
backend serializes them, primary first, into `WORKSPACE_REPOS`:

```json
[
  { "cloneUrl": "https://github.com/acme/api.git", "branch": "main", "provider": "github", "mountName": "api", "isPrimary": true },
  { "cloneUrl": "https://gitlab.com/acme/web.git", "provider": "gitlab", "mountName": "web", "isPrimary": false }
]
```

Each entry is cloned into `$WORKSPACE_DIR/{mountName}`, using that entry's own
provider token and branch. The primary entry's directory is `repo_dir` — the
one git status, diffs, and PRs target. `GIT_CLONE_URL` / `GIT_BRANCH` /
`GIT_PROVIDER` mirror the primary entry; when `WORKSPACE_REPOS` is absent or
unparseable the sandbox falls back to cloning that single repository, so an
older backend and a newer sandbox image still agree.

### REPO_CONFIG Structure

The `REPO_CONFIG` environment variable contains a JSON-encoded configuration object:

```json
{
  "systemPrompt": "Optional additional system prompt for the agent",
  "codeExecution": {
    "setupCommands": ["npm install", "npm run build"],
    "dockerImage": "aorwall/moatless-vibe-sandbox-js:latest"
  }
}
```

A Script (`ScriptConfig`: name, command, optional label/icon/port/envVars/runOnStart)
is not passed through `REPO_CONFIG` — it is declared on the Workspace and run
on demand as a terminal session inside this sandbox; see [Scripts](#scripts).

---

## API

### HTTP (port 8001, health probes only)

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/health` | K8s liveness/readiness/startup probe |

### gRPC SandboxService (port 50052)

All operational communication uses gRPC. The backend calls these RPCs:

| RPC | Description |
|-----|-------------|
| `ExecuteAgent` | Start agent execution with content, model, system prompt |
| `StopAgent` | Stop running agent |
| `NotifyMessages` | Wake message polling consumer |
| `GetAgentStatus` | Agent status snapshot (JSON) |
| `SearchFilesByName` | Search workspace files by query |
| `ApplyTaskScript` / `GetTaskScripts` | Declare/read this task's Scripts |
| `TerminalOpen` / `TerminalAttach` | Open and stream a Script's terminal session |

---

## Features

### Agent Execution

The agent execution system consists of three main components:

**Files**:
- `src/features/agents/services/agent-request-consumer.ts` (queue consumer)
- `src/features/agents/services/agent.service.ts` (execution coordinator)
- `src/features/agents/claude-code/claude-code-agent.ts` (Claude Code SDK wrapper)

#### Architecture

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                      SANDBOX AGENT EXECUTION                                 │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │                   AgentRequestConsumer                               │    │
│  ├─────────────────────────────────────────────────────────────────────┤    │
│  │  PRIMARY EXECUTION PATH (Redis Streams queue)                       │    │
│  │                                                                      │    │
│  │  • Polls Redis Streams: agent-requests:{taskId}                     │    │
│  │  • Blocking read with 5s timeout for first message                  │    │
│  │  • Non-blocking reads with 100ms timeout to batch (up to 10 msgs)   │    │
│  │  • Consumer groups for reliable delivery                            │    │
│  │  • ACKs all messages after execution (success or failure)           │    │
│  │  • Recovers unACKed messages on restart                             │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
│                                      │                                       │
│                                      ▼                                       │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │                       AgentService                                   │    │
│  ├─────────────────────────────────────────────────────────────────────┤    │
│  │  High-level execution coordinator                                   │    │
│  │                                                                      │    │
│  │  executeRequest(request) → Single request execution                 │    │
│  │  executeBatchedRequest(requests) → Batched execution                │    │
│  │    • Combines content from ALL requests                             │    │
│  │    • Takes options from LAST request (resume, model, mode)          │    │
│  │    • Tracks session ID for resume support                           │    │
│  │  resolveResumeValue(resume) → Resolves to actual session ID         │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
│                                      │                                       │
│                                      ▼                                       │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │                      ClaudeCodeAgent                                 │    │
│  ├─────────────────────────────────────────────────────────────────────┤    │
│  │  Wraps @anthropic-ai/claude-agent-sdk                               │    │
│  │                                                                      │    │
│  │  run(request) → Single execution                                    │    │
│  │  runBatched(requests) → Batched execution                           │    │
│  │    • Creates batched SDK message stream                             │    │
│  │    • Yields SDKUserMessage for each request                         │    │
│  │                                                                      │    │
│  │  • Emits ALL events directly to Redis via SandboxEventService       │    │
│  │  • Supports plan mode (ExitPlanMode detection)                      │    │
│  │  • AskUserQuestion/ExitPlanMode pause and wait for user input       │    │
│  │  • Session resume via agentSessionId                                │    │
│  │  • Validates CLAUDE_CODE_OAUTH_TOKEN or ANTHROPIC_API_KEY           │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

#### Primary Execution Flow (Queue-Based)

```
AgentRequestConsumer (polling Redis)
│
├─ XREADGROUP blocking read (5s timeout)
│     Stream key: agent-requests:{taskId}
│     Consumer group: sandbox-{taskId}-consumers
│
├─ Accumulate requests (up to 10, 100ms timeout for batching)
│
├─ AgentService.executeBatchedRequest(requests)
│     │
│     ├─ Create batched request (combine content, use last options)
│     │
│     └─ ClaudeCodeAgent.runBatched(requests)
│           │
│           ├─ Validate authentication token
│           ├─ Build SDK options (token, tools, environment)
│           ├─ Call claude-agent-sdk query() function
│           │   ├─ SDK executes tools (Bash, Read, Write, etc.)
│           │   ├─ For each message → emit agent:message to Redis
│           │   ├─ If AskUserQuestion/ExitPlanMode tool:
│           │   │   └─ Pause, wait for /user-input-response
│           │   └─ On completion → emit agent:completed
│           │
│           └─ Return result
│
└─ ACK all messages (success or failure)
```

#### Secondary Execution Flow (HTTP Fallback)

```
POST /execute-agent (HTTP endpoint)
│
├─ AgentService.executeRequest(request)
│     │
│     └─ ClaudeCodeAgent.run(request)
│           (same execution as above, single request)
│
└─ Return immediately (agent runs in background)
```

#### Allowed Tools

The agent runs with these Claude Code tools enabled:
- `WebFetch`, `WebSearch` — Web access
- `Bash` — Shell commands
- `Read`, `Write`, `Edit` — File operations
- `Glob`, `Grep` — File search
- `Task` — Subagent tasks

#### User Input Handling

Two tools pause agent execution and wait for user response:
- `AskUserQuestion` — Presents choices to the user
- `ExitPlanMode` — Submits a plan for approval

The agent pauses via a pending Promise. The frontend displays the question, and when the user responds, the backend calls `POST /user-input-response` to resolve the promise and resume execution.

#### Session Resume

Agents support conversation continuity via `resume` session IDs:
- `resume: true` → Use last known session ID (tracked by AgentService)
- `resume: "session-id"` → Use specific session ID
- `resume: false/undefined` → Start new session

The backend's TaskEventHandler captures `agentSessionId` from `agent:message` events and stores it for future resume operations.

#### Request Batching

Multiple queued requests can be batched into a single SDK call:
- Content from all requests is combined
- Options (resume, model, mode) come from the LAST request
- Reduces overhead for rapid consecutive messages
- All messages ACKed together after batch completion

---

### File Operations

**File**: `src/features/files/services/file.service.ts`

The `FileService` provides CRUD operations on the workspace filesystem with path traversal prevention.

**Methods**:
- `readFile(filePath)` — Read file content (validates path within workspace)
- `writeFile(filePath, content)` — Write file, creating parent directories as needed
- `listFiles(relativePath?, recursive?)` — List files with optional recursion
- `searchFiles(query, options)` — Filename pattern search
- `getFileTreeWithChanges(rootPath?)` — Hierarchical tree structure with git annotations

**Security**: All paths are resolved and validated to ensure they remain within the configured workspace directory.

---

### Git Operations

**File**: `src/features/git/services/git.service.ts`

The `GitService` provides read-only git information for the workspace.

**Methods**:
- `getStatus()` — Branch name, commits ahead/behind origin, conflict detection, change summary
- `getChanges()` — Working directory changes: added, modified, deleted files with staging status
- `getDiff()` — Detailed file-level diff with hunks

**Utilities** (`src/features/git/utils/git.utils.ts`):
- `getOriginSyncStatus()` — Compare local vs origin
- `checkForConflicts()` — Detect merge conflicts
- `getGitDiffDetailed()` — Full diff with file metadata
- `getWorkingDirectoryChanges()` — Untracked, staged, unstaged changes

---

### Scripts

There is no dedicated preview-server process, shell script, or filesystem
status file in the sandbox. A **Script** (`ScriptConfig`: name, command,
optional label/icon/port/envVars/runOnStart) is a named command a Workspace
declares; it runs as a terminal session inside this same sandbox container —
`crates/t3code/src/scripts.rs::run` execs the command into a named terminal,
publishing the Script's `port` first when it has one.

A Script with a `port` earns a preview URL: the `SandboxPortPublisher` trait
publishes the port on the fly (Kubernetes: `crates/sandbox-k8s/src/edge.rs`
adds a `scr-{port}` Service port and a Traefik `IngressRoute`; Docker's
implementation always returns `None`, so a Docker sandbox's Script never gets
a preview URL). There is no separate proxy or inspector-script injection step
in this path.

A Script's own output lives in its terminal (read via `moat tasks scripts
run <name>` or the T3 UI terminal pane), not in the sandbox's own log stream
(`moat tasks logs`) or on the gRPC task-event feed.

---

### Browser Automation

**File**: `src/features/browser/services/browser-server.ts`

The `BrowserServer` manages a Chromium instance via Playwright for page interaction and screenshots.

**Singleton**: `getBrowserServer(options?)` — Get or create instance

**Chromium Launch Args**:
- `--no-sandbox`
- `--disable-setuid-sandbox`
- `--disable-dev-shm-usage`
- `--remote-debugging-port=9223`

**Methods**:
- `start()` — Launch Chromium with CDP debugging
- `stop()` — Close browser and cleanup
- `isRunning()` — Check connection status
- `listScreenshots()` — List screenshot metadata from data directory
- `readScreenshot(filename)` — Read screenshot buffer

**Screenshot Storage**: `${DATA_DIR}/screenshots/` — PNG files with JSON metadata sidecars

The agent interacts with the browser through the `sandbox-browser` skill, which provides a `BrowserClient` TypeScript API for page navigation, form filling, screenshots, and evaluation.

---

### Workspace Provisioning

**File**: `src/features/workspace/services/workspace-provisioner.ts`

Handles initial workspace setup on container start:

1. **Git Clone**: Clone every checkout declared in `WORKSPACE_REPOS` into its own `$WORKSPACE_DIR/{mountName}` directory (falling back to `GIT_CLONE_URL` alone when the variable is absent)
2. **Branch Checkout**: Switch each checkout to its own branch if specified
3. **Skip**: A mount that already contains `.git/` is left alone (handles pre-provisioned volumes), so a repo added to the workspace later is cloned on the next start without disturbing the existing checkouts

The provisioning is performed by the entrypoint script before the sandbox application starts, ensuring the workspace is ready when the HTTP server begins accepting requests.

---

## Skills System

Skills give agents domain-specific knowledge and tooling guidance. Sandbox init
**copies** two sets into the home-level skill directories both agents already
read — `$CLAUDE_CONFIG_DIR/skills` and `$CODEX_HOME/skills`. Copies, not
symlinks, so they survive the workspace volume mount.

### Sources

| Source | Installed by | Notes |
| --- | --- | --- |
| `/usr/share/moatless-skills/` | `init/dirs.rs` | Staged into the image by downstream Dockerfiles (`COPY`). Empty in the base image. |
| The primary repo's `.claude/skills/` and `.agents/skills/` | `init/home_skills.rs` | Both roots are scanned and merged, `.claude/skills` first. `is_dir()` follows symlinks, so a `.claude/skills` that points at `.agents/skills` is copied through. |

A name already present is never overwritten, so a staged image skill wins over a
repo skill of the same name, and the merge of the two repo roots copies each
skill once. Only skills are bridged. Instruction files are not: a repo that wants
both `AGENTS.md` and `CLAUDE.md` commits one as a symlink to the other. Init does
not create those links, so nothing the sandbox does can drift from the repo.

**Multi-repo limitation**: `install_repo_skills_to_home` takes `repo_dir`, which
is the workspace's *primary* checkout. Skills shipped by a secondary repo in a
multi-repo workspace are not installed.

### Selection

Skills are **model-invoked**: the agent picks one by reading the `description`
in each `SKILL.md` frontmatter. There is no activation-rules file and no
keyword-matching hook — write the `description` so it names the triggers.

---

## Event System

**File**: `src/events.rs` (`GrpcEventPublisher`)

The `GrpcEventPublisher` publishes structured events to the backend via gRPC `PublishEvent` RPC. The backend then records events on its Postgres task-event feed for SSE fan-out and event handler processing.

### Event Structure (gRPC)

Events are sent as `PublishEventRequest` messages with fields:
- `task_id`: Task this event belongs to
- `event_type`: `agent`, `message`, `resources`, `sandbox`, `task`, or `taskTurn` (`EventType`, `crates/types/src/event.rs`)
- `sub_type`: e.g. "agent_started", "message", "agent_completed"
- `agent_type`: Optional agent type identifier
- `turn_number`: Optional turn number
- `is_error`: Whether this is an error event
- `payload_json`: JSON string payload (preserved for backend consumer compatibility)

### Transport

Events are published via gRPC `PublishEvent` to the backend's `BackendSandboxService`. The backend records them in `task_event_changes`, where the SSE feed and the event dispatcher read them.

### Event Types Emitted

| Type | SubType | Trigger |
|------|---------|---------|
| `agent` | `started` | Agent begins execution |
| `agent` | `message` | Agent produces a message (includes content, agentSessionId, contextTokens) |
| `agent` | `completed` | Agent finishes successfully (includes result, plan if applicable) |
| `agent` | `error` | Agent encounters an error |

A Script's lifecycle is not carried on this event feed — it runs as a terminal
session, and its state is read from the terminal (`moat tasks scripts list`,
the T3 UI terminal pane), not from task events.

### Deduplication

Events include unique `eventId` values (UUIDs). The `BaseEventService` maintains an in-memory deduplication cache with 60s TTL to prevent infinite loops when the same event is received twice.

---

## Communication with Backend

The sandbox communicates with the backend entirely via gRPC, using two services defined in `crates/sandbox-proto/`:

### 1. SandboxService (Backend → Sandbox, gRPC port 50052)

The backend calls the sandbox's gRPC server for commands and queries:

```
Backend (gRPC Client)                      Sandbox (gRPC Server :50052)
  │                                          │
  ├─ ExecuteAgent(content, model) ────────►  │ (start agent execution)
  ├─ StopAgent() ─────────────────────────►  │ (stop running agent)
  ├─ NotifyMessages() ────────────────────►  │ (wake message consumer)
  ├─ GetAgentStatus() ────────────────────►  │ (agent status snapshot)
  ├─ SearchFilesByName(query, limit) ─────►  │ (search workspace files)
  ├─ ApplyTaskScript(script) ─────────────►  │ (declare/update a Script)
  └─ TerminalAttach(sessionId) ───────────►  │ (stream a Script's terminal)
```

### 2. BackendSandboxService (Sandbox → Backend, gRPC port 50051)

The sandbox calls the backend's gRPC server to publish events and fetch messages:

```
Sandbox (gRPC Client)                      Backend (gRPC Server :50051)
  │                                          │
  ├─ PublishEvent(task_id, type, payload) ─►  │ → records on the feed → SSE → Frontend
  └─ GetPendingMessages(task_id, token) ──►  │ → returns queued user messages
```

**Event flow**: Sandbox publishes events via gRPC `PublishEvent`. The backend records them on the Postgres task-event feed, where the SSE feed and the event dispatcher read them.

**Message flow**: Sandbox polls for pending messages via gRPC `GetPendingMessages` (with Notify-based wakeup via `NotifyMessages`), replacing the previous HTTP polling.

### Complete Agent Execution Flow

```
Backend                                          Sandbox
  │                                                │
  ├─ Persist user message to DB                    │
  ├─ NotifyMessages() (gRPC) ──────────────────►   │
  │                                                ├─ MessageConsumer wakes
  │                                                ├─ GetPendingMessages() (gRPC) ──►
  │  ◄─── returns pending messages ────────────────┤
  │                                                │
  │                                                ├─ Batch messages
  │                                                ├─ AgentExecutor.run_batched()
  │                                                │
  │  ◄─── PublishEvent(agent:started) ─────────────┤
  │  ◄─── PublishEvent(agent:message) ─────────────┤
  │  ◄─── PublishEvent(agent:completed) ───────────┤
  │                                                │
  ├─ Records on the task-event feed (internal)     │
  ├─ SSE feed → broadcast to frontend              │
  ├─ Event handler → DB writes, notifications      │
  │                                                │
```

---

## Docker Image

### Base Image (`Dockerfile.sandbox`)

Built on Ubuntu 24.04 with:

| Component | Version | Notes |
|-----------|---------|-------|
| Git | Latest | With LFS support |
| Node.js | 22.x | Via NodeSource |
| Claude Code CLI | Latest | npm global install |
| GitHub CLI (gh) | Latest | For repository operations |
| Git Delta | Latest | Better diff formatting |
| Python build deps | — | For pyenv support |
| Network tools | — | curl, wget, netcat, openssh |

### JavaScript Variant (`Dockerfile.sandbox-js`)

Extends base with:

| Component | Version | Notes |
|-----------|---------|-------|
| Bun | 1.3.6 | JavaScript runtime |
| pnpm, yarn | Latest | Via corepack |
| TypeScript, tsx | Latest | TypeScript execution |
| Vite+ (`vp`) | 0.2.7 | Unified JS toolchain at `VP_HOME=/opt/vite-plus` |
| Playwright | Pinned | One package tree per pin under `/opt/playwright-packages` |
| Chromium | Shared | One build per pin at `/opt/playwright-browsers` |

### Rust Toolchain

The base image (`Dockerfile.sandbox-base`) bakes a full Rust toolchain
(rustup, cargo, clippy, rustfmt) at `RUSTUP_HOME=/opt/rust` /
`CARGO_HOME=/opt/cargo` so agents never download a toolchain at runtime.

- The baked version (`ARG RUST_VERSION` in `Dockerfile.sandbox-base`) must
  match the workspace pin in `rust-toolchain.toml`; the `toolchain-drift`
  job in `ci.yml` fails on divergence. When bumping the workspace toolchain,
  bump the Dockerfile arg in the same PR (the base image rebuilds on
  Dockerfile changes).
- `K8S_SANDBOX_PERSIST_TOOLCHAIN=true` (default off) additionally persists
  cargo's registry and git caches across pod replacement via `workspace-data`
  subPath mounts at `/opt/cargo/registry` and `/opt/cargo/git`. Only these
  cache dirs ride the volume — mounting over all of `/opt/cargo` or
  `/opt/rust` would shadow the baked toolchain (K8s subPath mounts don't seed
  from image content). Budget PVC space when enabling: a warm registry is
  multiple Gi on top of the default 3Gi workspace.
**Sizing Rust-heavy repos**: a full `cargo build` of a multi-crate workspace
peaks around 5 GiB of RSS and grows `target/` past several Gi, so a sandbox on
the deployment defaults is prone to two evictions: node-pressure eviction
(the pod is a low-memory-request eviction target) and
`Usage of EmptyDir volume 'workspace-data' exceeds the limit` (the workspace
volume fills with build artifacts). Give such repos an explicit per-repo
`resources` block — via the `.moatless/workspaces.json` entry (git-synced) or
the workspace API — with a raised memory request/limit and a sized `storage`,
for example:

```json
"resources": {
  "requests": { "cpu": "1", "memory": "4Gi" },
  "limits":   { "cpu": "4", "memory": "8Gi" },
  "storage":  "20Gi"
}
```

The container's requests/limits are copied **verbatim** from this block
(`pod_builder.rs`). Each of `requests.cpu`, `requests.memory`, `limits.cpu` and
`limits.memory` overrides independently: a block that sets only `storage`
still gets the deployment `K8S_RESOURCE_*` defaults, so the container is never
BestEffort.

### User

Runs as non-root user `moatless` (UID 1000) with home at `/home/moatless`.

### Key Paths

| Path | Purpose |
|------|---------|
| `/usr/local/bin/sandbox` | Rust sandbox binary |
| `/opt/moatless/workspace` | Workspace root containing repo checkouts |
| `/opt/moatless/workspace/<primary-mount>` | Primary repo checkout; `<primary-mount>` is the workspace placement mount name or derived repo name |
| `/opt/moatless/data` | Runtime data (events, screenshots) |
| `/opt/moatless/.claude` | Claude Code configuration |
| `/usr/share/moatless-skills` | Skills (baked into image) |
| `/opt/playwright-browsers` | Shared Playwright browser binaries |
| `/opt/playwright-packages` | Playwright package tree per pin; keeps the browsers above from being pruned |
| `/usr/local/bin/playwright-cli` | Agent-facing Playwright wrapper; runs the baked package, so it reaches no npm registry |
| `/usr/local/bin/docker-entrypoint-sandbox.sh` | Container entrypoint |
