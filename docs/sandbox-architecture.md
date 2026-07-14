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
   - [Preview Server](#preview-server)
   - [Browser Automation](#browser-automation)
   - [Workspace Provisioning](#workspace-provisioning)
7. [Skills System](#skills-system)
8. [Event System](#event-system)
9. [Communication with Backend](#communication-with-backend)
10. [Docker Image](#docker-image)

---

## Overview

The sandbox is an isolated per-task execution environment that runs the Claude Code agent and supporting services. It provides file operations, git integration, preview server management, and browser automation—all scoped to a single task's workspace.

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
│   ├── server                           # Server management script
│   └── docker-entrypoint-sandbox.sh     # Container entrypoint
│
├── opt/moatless/
│   ├── repo/                            # Workspace (git clone target, may be volume-mounted)
│   ├── data/                            # Runtime data
│   │   ├── events/                      # Persisted events
│   │   ├── screenshots/                 # Browser screenshots
│   │   ├── preview-server.config.json   # Preview server configuration
│   │   ├── preview-server.status.json   # Current server status
│   │   ├── preview-server.pid           # Server process ID
│   │   └── preview-server.log           # Server stdout/stderr
│   ├── .claude/                         # Claude Code configuration
│   │   └── skills -> /usr/share/moatless-skills  # Symlinked at startup
│   └── bin/
│       └── preview-server               # Shell script for server lifecycle
│
├── usr/share/moatless-skills/           # Skills baked into image
│   ├── preview-server/
│   │   └── SKILL.md
│   ├── sandbox-browser/
│   │   ├── SKILL.md
│   │   ├── references/
│   │   └── scripts/
│   │       └── browser-client.ts
│   └── skill-rules.json
```

**Volume Mount Consideration**: The workspace directory (`/opt/moatless/repo`) may be volume-mounted for persistent storage. The sandbox binary lives at `/usr/local/bin/sandbox` to avoid being overwritten by workspace volume mounts.

---

## Startup Sequence

The container entrypoint (`docker-entrypoint-sandbox.sh`) orchestrates parallel startup of multiple components:

```
Container Start
│
├─ 1. Create directories
│     /opt/moatless/repo, /opt/moatless/.claude, /opt/moatless/data
│
├─ 2. Symlink skills
│     /usr/share/moatless-skills → /opt/moatless/.claude/skills
│
├─ 3. Configure git
│     Set GITHUB_TOKEN credential helper, user.name, user.email
│
├─ 4. Clone repository (if GIT_CLONE_URL set and .git/ absent)
│     gh repo clone → checkout GIT_BRANCH
│
├─ 5. Write preview config (if REPO_CONFIG has previewConfig)
│     Parse REPO_CONFIG JSON → /opt/moatless/data/preview-server.config.json
│
├─ 6. Start sandbox application (background)
│     /usr/local/bin/sandbox &
│
├─ 7. Run setup commands (from REPO_CONFIG.codeExecution.setupCommands)
│     Execute sequentially in workspace directory
│
├─ 8. Auto-start preview server (if config exists with serverCommands)
│     /opt/moatless/bin/preview-server start &
│
└─ 9. Wait for sandbox process
      wait $SANDBOX_PID
```

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
├─ Initialize PreviewProcessMonitor (watch status.json)
│     • Watches filesystem for preview server status changes
│     • Emits server events to Redis
│
├─ Create Hono app with routes
│   ├─ Workspace routes
│   ├─ File routes
│   ├─ Git routes
│   ├─ Agent routes (HTTP fallback, secondary execution path)
│   ├─ Preview routes
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
| `WORKSPACE_DIR` | No | `/opt/moatless/repo` | Workspace directory path |
| `DATA_DIR` | No | `/opt/moatless/data` | Data directory for runtime artifacts |
| `LOG_LEVEL` | No | `info` | Pino log level |
| `GIT_CLONE_URL` | No | — | Repository URL to clone on startup |
| `GIT_BRANCH` | No | — | Branch to checkout after clone |
| `GITHUB_TOKEN` | No | — | Token for private repository access |
| `REPO_CONFIG` | No | — | JSON-encoded repository configuration |
| `CLAUDE_CODE_OAUTH_TOKEN` | No | — | OAuth token for Claude Code agent |
| `CLAUDE_BIN_DIR` | No | `/usr/bin` | Directory containing Claude CLI binary |
| `CLAUDE_CONFIG_DIR` | No | `/opt/moatless/.claude` | Claude configuration directory |
| `BACKEND_GRPC_URL` | No | — | Backend gRPC endpoint for events/messages |
| `GRPC_PORT` | No | `50052` | gRPC server port for SandboxService |

### REPO_CONFIG Structure

The `REPO_CONFIG` environment variable contains a JSON-encoded configuration object:

```json
{
  "systemPrompt": "Optional additional system prompt for the agent",
  "codeExecution": {
    "setupCommands": ["npm install", "npm run build"],
    "dockerImage": "aorwall/moatless-vibe-sandbox-js:latest"
  },
  "previewConfig": {
    "serverStartCommands": ["npm", "run", "dev"],
    "port": 3000,
    "startupTimeoutSeconds": 60
  }
}
```

### Preview Server Config File

Generated from `REPO_CONFIG.previewConfig` and written to `/opt/moatless/data/preview-server.config.json`:

```json
{
  "serverCommands": ["npm", "run", "dev"],
  "port": 3000,
  "startupTimeoutSeconds": 60,
  "workspacePath": "/opt/moatless/repo"
}
```

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
| `GetStatus` | Combined agent + servers snapshot (JSON) |
| `CheckHealth` | Health check |
| `SearchFiles` | Search workspace files by query |
| `GetServerStatus` | Single server status (JSON) |
| `StreamServerLogs` | Stream server logs (server-streaming RPC) |

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

### Preview Server

The preview server feature uses a three-component architecture: a shell script manages the process, a monitor watches for state changes, and a proxy forwards requests with script injection.

#### Architecture

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                        PREVIEW SERVER ARCHITECTURE                           │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  Agent (via preview-server skill)                                           │
│       │                                                                      │
│       ▼                                                                      │
│  /opt/moatless/bin/preview-server start|stop|restart|status|logs            │
│       │                                                                      │
│       ├─► Reads: preview-server.config.json                                 │
│       ├─► Writes: preview-server.status.json (state transitions)            │
│       ├─► Writes: preview-server.pid                                        │
│       └─► Writes: preview-server.log (stdout/stderr)                        │
│                                                                              │
│  PreviewProcessMonitor (watches status.json)                                │
│       │                                                                      │
│       ├─► Detects status changes                                            │
│       ├─► Verifies PID is alive                                             │
│       └─► Emits server events to Redis → Backend → SSE → Frontend          │
│                                                                              │
│  handleProxyRequest() (serves /preview/*)                                   │
│       │                                                                      │
│       ├─► Proxies HTTP to localhost:<port>                                   │
│       ├─► Injects inspector script into HTML responses                      │
│       └─► Strips CSP/X-Frame-Options headers for iframe embedding           │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

#### Shell Script: `/opt/moatless/bin/preview-server`

**Commands**:

| Command | Description |
|---------|-------------|
| `start` | Read config, start server process, poll for readiness, write status |
| `stop` | Send SIGTERM (then SIGKILL after timeout), write stopped status |
| `restart` | Stop then start |
| `status` | Read and output status.json |
| `logs [-n N] [-f]` | Tail log file, optionally follow |

**Start Flow**:
1. Read `preview-server.config.json`
2. Replace `{{PORT}}` placeholder in server commands
3. cd to workspace directory
4. Start: `PORT=<port> bash -c "<commands>"` in background
5. Write `starting` status
6. Poll `http://localhost:<port>/` until responsive (tries IPv6 and IPv4)
7. Write `started` or `failed` status with port, PID, error info

#### Filesystem Artifacts

| File | Content |
|------|---------|
| `preview-server.config.json` | `{ serverCommands, port, startupTimeoutSeconds, workspacePath }` |
| `preview-server.status.json` | `{ status, pid, internalUrl, error, timestamp }` |
| `preview-server.pid` | Process ID (plain text) |
| `preview-server.log` | Server stdout/stderr output |

#### Status Lifecycle

```
stopped → starting → started
                  └─→ failed → stopped (on retry)
started → stopped (on stop or crash)
```

#### Process Monitor

**File**: `src/features/preview/services/preview-process-monitor.ts`

`PreviewProcessMonitor` watches the status file and emits Redis events:

```typescript
class PreviewProcessMonitor {
  getStatus(): Promise<PreviewServerStatus>     // Read status + verify PID alive
  isRunning(): Promise<boolean>                  // Check PID alive
  getConfiguredPort(): number                    // From config file (default 3000)
  getLogs(lines?: number): Promise<string>       // Tail log file
  streamLogs(): AsyncGenerator<LogChunk>         // Watch + stream log file
  startWatching(): void                          // Begin file watch loop
  stopWatching(): void                           // Stop watching
}
```

#### Proxy

**File**: `src/features/preview/runtime/preview-proxy.ts`

`handleProxyRequest(req, path, serverId, port)`:
- HTML requests (`GET /` or `/index.html`): Fetch, inject inspector script, return modified HTML
- All other requests: Raw pipe via `node:http` (preserves content-encoding)

The inspector script enables communication between the preview iframe and the parent application for live interaction tracking.

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

1. **Git Clone**: If `GIT_CLONE_URL` is set and `.git/` doesn't exist, clone the repository
2. **Branch Checkout**: Switch to `GIT_BRANCH` if specified
3. **Skip**: If `.git/` already exists, skip cloning (handles pre-provisioned volumes)

The provisioning is performed by the entrypoint script before the sandbox application starts, ensuring the workspace is ready when the HTTP server begins accepting requests.

---

## Skills System

Skills provide Claude Code agents with domain-specific knowledge and tooling guidance. They are baked into the Docker image and symlinked to the Claude config directory at startup.

### Location

- **Image path**: `/usr/share/moatless-skills/`
- **Symlinked to**: `$CLAUDE_CONFIG_DIR/skills`
- **Activation rules**: `skill-rules.json`

### Available Skills

#### preview-server

**Activation triggers**: Keywords like "preview server", "dev server", "start server", "server logs"; patterns like `(start|stop|restart).*server`

**Provides**:
- Shell script commands for managing the preview server
- Filesystem artifact paths and formats
- Config file format reference
- Troubleshooting guide
- Rules: Always use the shell script, never start servers manually

#### sandbox-browser

**Activation triggers**: Keywords like "browser automation", "playwright", "screenshot"; patterns like `(take|get).*screenshot`

**Provides**:
- `BrowserClient` TypeScript API (`/opt/moatless/.claude/skills/sandbox-browser/scripts/browser-client.ts`)
- CDP access patterns
- Common automation examples (navigation, form filling, screenshots)
- Import path and usage instructions

### skill-rules.json

Defines activation conditions for each skill:

```json
[
  {
    "name": "preview-server",
    "keywords": ["preview server", "dev server", ...],
    "intentPatterns": ["(start|stop|restart).*server", ...],
    "priority": "HIGH"
  },
  {
    "name": "sandbox-browser",
    "keywords": ["browser automation", "playwright", ...],
    "intentPatterns": ["(take|get).*screenshot", ...],
    "priority": "MEDIUM"
  }
]
```

---

## Event System

**File**: `src/events.rs` (`GrpcEventPublisher`)

The `GrpcEventPublisher` publishes structured events to the backend via gRPC `PublishEvent` RPC. The backend then writes events to Redis Streams internally for SSE fan-out and event handler processing.

### Event Structure (gRPC)

Events are sent as `PublishEventRequest` messages with fields:
- `task_id`: Task this event belongs to
- `event_type`: "task", "taskTurn", or "environment"
- `sub_type`: e.g. "agent_started", "message", "agent_completed"
- `agent_type`: Optional agent type identifier
- `turn_number`: Optional turn number
- `is_error`: Whether this is an error event
- `payload_json`: JSON string payload (preserved for backend consumer compatibility)

### Transport

Events are published via gRPC `PublishEvent` to the backend's `BackendSandboxService`. The backend writes them to Redis Streams at key `task-events:{taskId}`, where existing SSE consumers and event handlers process them unchanged.

### Event Types Emitted

| Type | SubType | Trigger |
|------|---------|---------|
| `agent` | `started` | Agent begins execution |
| `agent` | `message` | Agent produces a message (includes content, agentSessionId, contextTokens) |
| `agent` | `completed` | Agent finishes successfully (includes result, plan if applicable) |
| `agent` | `error` | Agent encounters an error |
| `server` | `starting` | Preview server start initiated |
| `server` | `started` | Preview server HTTP ready |
| `server` | `stopped` | Preview server process terminated |
| `server` | `failed` | Preview server failed to start or crashed |

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
  ├─ GetStatus() ─────────────────────────►  │ (combined status snapshot)
  ├─ CheckHealth() ───────────────────────►  │ (health check)
  ├─ SearchFiles(query, limit) ───────────►  │ (search workspace files)
  ├─ GetServerStatus(name) ───────────────►  │ (server status)
  └─ StreamServerLogs(name, lines) ───────►  │ (streaming server logs)
```

### 2. BackendSandboxService (Sandbox → Backend, gRPC port 50051)

The sandbox calls the backend's gRPC server to publish events and fetch messages:

```
Sandbox (gRPC Client)                      Backend (gRPC Server :50051)
  │                                          │
  ├─ PublishEvent(task_id, type, payload) ─►  │ → writes to Redis → SSE → Frontend
  └─ GetPendingMessages(task_id, token) ──►  │ → returns queued user messages
```

**Event flow**: Sandbox publishes events via gRPC `PublishEvent`. The backend writes them to Redis Streams (internally), where existing SSE consumers and event handlers process them — unchanged from before.

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
  ├─ Redis XADD (internal)                         │
  ├─ SSE consumers → broadcast to frontend         │
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
| Playwright | Latest | Browser automation |
| Chromium | Shared | At `/opt/playwright-browsers` |

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
- `K8S_SANDBOX_CARGO_TARGET_ON_BUILD_CACHE=true` (default off) sets
  `CARGO_TARGET_DIR=/opt/moatless/build-cache/target` so incremental compiles
  survive container restarts. Raise `K8S_SANDBOX_BUILD_CACHE_SIZE` (default
  `1536Mi`) to fit the repo's target dir first — overflowing the emptyDir
  limit evicts the pod.

### User

Runs as non-root user `moatless` (UID 1000) with home at `/home/moatless`.

### Key Paths

| Path | Purpose |
|------|---------|
| `/usr/local/bin/sandbox` | Rust sandbox binary |
| `/opt/moatless/repo` | Workspace directory (git clone target) |
| `/opt/moatless/data` | Runtime data (events, screenshots, preview artifacts) |
| `/opt/moatless/.claude` | Claude Code configuration |
| `/opt/moatless/bin/preview-server` | Preview server shell script |
| `/usr/share/moatless-skills` | Skills (baked into image) |
| `/opt/playwright-browsers` | Shared Playwright browser binaries |
| `/usr/local/bin/docker-entrypoint-sandbox.sh` | Container entrypoint |
