# Moatless Vibe Backend Architecture

## Table of Contents
1. [System Overview](#system-overview)
2. [Core Features](#core-features)
3. [Request Flow](#request-flow)
4. [Tasks Feature](#tasks-feature)
5. [Workspace Feature](#workspace-feature)
6. [Local vs Kubernetes Execution](#local-vs-kubernetes-execution)
7. [Agents Feature](#agents-feature)
8. [Preview Feature](#preview-feature)
9. [Events Feature](#events-feature)
10. [Complete Flow Example](#complete-flow-example)

---

## System Overview

The backend is a **Hono-based Node.js application** running on Bun, organized into feature modules with a layered architecture pattern.

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                              CLIENT (Frontend)                               │
│                    React + TanStack Query + TanStack Router                  │
└─────────────────────────────────────────────────────────────────────────────┘
                                      │
                                      ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                              API GATEWAY                                     │
│                         Hono Routes + OpenAPI                                │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐    │
│  │    Tasks     │  │  Workspace   │  │    Agents    │  │   Preview    │    │
│  │   Routes     │  │   Routes     │  │   Routes     │  │   Routes     │    │
│  └──────────────┘  └──────────────┘  └──────────────┘  └──────────────┘    │
│                                                                              │
├─────────────────────────────────────────────────────────────────────────────┤
│                            SERVICE LAYER                                     │
│                                                                              │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐    │
│  │ TaskService  │  │  Workspace   │  │    Agent     │  │   Preview    │    │
│  │ Iteration    │  │  Runtime     │  │  Controller  │  │ Orchestrator │    │
│  │ Coordinator  │  │  Manager     │  │  MCP Service │  │   Runtime    │    │
│  └──────────────┘  └──────────────┘  └──────────────┘  └──────────────┘    │
│                                                                              │
│                         ┌──────────────────────┐                            │
│                         │    Event Service     │◄─── SSE Broadcasting       │
│                         │  (Central Event Hub) │                            │
│                         └──────────────────────┘                            │
│                                                                              │
├─────────────────────────────────────────────────────────────────────────────┤
│                          REPOSITORY LAYER                                    │
│                                                                              │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐    │
│  │    Task      │  │  Workspace   │  │   Command    │  │    Event     │    │
│  │ Repository   │  │  Runtime     │  │  Repository  │  │  Repository  │    │
│  │              │  │  Repository  │  │              │  │              │    │
│  └──────────────┘  └──────────────┘  └──────────────┘  └──────────────┘    │
│                                                                              │
├─────────────────────────────────────────────────────────────────────────────┤
│                           DATABASE LAYER                                     │
│                     SQLite (dev) / PostgreSQL (prod)                         │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## Core Features

### Feature Responsibilities

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           FEATURE DOMAINS                                    │
├─────────────────┬───────────────────────────────────────────────────────────┤
│                 │                                                           │
│     TASKS       │  • Task lifecycle (create, update, delete)                │
│                 │  • Iterations (execution cycles)                          │
│                 │  • Message history                                        │
│                 │  • Coordinates agents + workspaces                        │
│                 │                                                           │
├─────────────────┼───────────────────────────────────────────────────────────┤
│                 │                                                           │
│   WORKSPACE     │  • Execution environment provisioning                     │
│                 │  • Runtime state management                               │
│                 │  • Local vs Kubernetes abstraction                        │
│                 │  • File operations via FileGateway                        │
│                 │                                                           │
├─────────────────┼───────────────────────────────────────────────────────────┤
│                 │                                                           │
│     AGENTS      │  • AI agent lifecycle (start/stop)                        │
│                 │  • MCP protocol server                                    │
│                 │  • Tool execution                                         │
│                 │  • Message persistence                                    │
│                 │                                                           │
├─────────────────┼───────────────────────────────────────────────────────────┤
│                 │                                                           │
│    PREVIEW      │  • Preview server lifecycle                               │
│                 │  • HTTP proxy management                                  │
│                 │  • Playwright browser automation                          │
│                 │  • Inspector script injection                             │
│                 │                                                           │
├─────────────────┼───────────────────────────────────────────────────────────┤
│                 │                                                           │
│     EVENTS      │  • Real-time event streaming (SSE)                        │
│                 │  • Event persistence                                      │
│                 │  • Pub/sub to all features                                │
│                 │                                                           │
└─────────────────┴───────────────────────────────────────────────────────────┘
```

---

## Request Flow

### High-Level Request Processing

```
                                 ┌─────────────────┐
                                 │     Client      │
                                 │   (Frontend)    │
                                 └────────┬────────┘
                                          │
                         ┌────────────────┴────────────────┐
                         │                                 │
                         ▼                                 ▼
                ┌───────────────┐                 ┌───────────────┐
                │  REST API     │                 │    SSE        │
                │  (JSON)       │                 │  (Events)     │
                └───────┬───────┘                 └───────┬───────┘
                        │                                 │
                        └────────────────┬────────────────┘
                                         │
                                         ▼
                             ┌───────────────────────┐
                             │    Hono Router        │
                             │  + Auth Middleware    │
                             └───────────┬───────────┘
                                         │
                     ┌───────────────────┼───────────────────┐
                     │                   │                   │
                     ▼                   ▼                   ▼
            ┌──────────────┐    ┌──────────────┐    ┌──────────────┐
            │    Task      │    │   Workspace  │    │   Preview    │
            │   Service    │    │   Service    │    │   Service    │
            └──────────────┘    └──────────────┘    └──────────────┘
```

---

## Tasks Feature

### Data Model

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                              TASK DATA MODEL                                 │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │                              Task                                    │    │
│  ├─────────────────────────────────────────────────────────────────────┤    │
│  │  taskId: string (PK)                                                │    │
│  │  status: 'created' | 'running' | 'completed' | 'failed' | 'closed'  │    │
│  │  repositoryId: string                                               │    │
│  │  name, description: string                                          │    │
│  │  createdAt, lastMessageAt: Date                                     │    │
│  │  numTurns, totalCostUsd: number                                     │    │
│  └──────────────────────────────┬──────────────────────────────────────┘    │
│                                 │                                            │
│                                 │ 1:N                                        │
│                                 ▼                                            │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │                         TaskIteration                                │    │
│  ├─────────────────────────────────────────────────────────────────────┤    │
│  │  taskId + iterationNumber (PK)                                      │    │
│  │  status: 'active' | 'completed' | 'error' | 'stopped'               │    │
│  │  initialMessage: string                                             │    │
│  │  result: string                                                     │    │
│  │  costUsd, contextTokens: number                                     │    │
│  │  gitDiff, gitStatus, gitChanges: JSON                               │    │
│  │  screenshotFileId, siteUrl: string                                  │    │
│  └──────────────────────────────┬──────────────────────────────────────┘    │
│                                 │                                            │
│                                 │ 1:N                                        │
│                                 ▼                                            │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │                         TaskMessage                                  │    │
│  ├─────────────────────────────────────────────────────────────────────┤    │
│  │  messageId: string (PK)                                             │    │
│  │  taskId: string (FK)                                                │    │
│  │  role: 'user' | 'assistant'                                         │    │
│  │  content: string                                                    │    │
│  │  iterationNumber: number                                            │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Task & Iteration Lifecycle

Tasks contain multiple **iterations** - each iteration represents one user message → agent execution cycle. The iteration is where agent outcomes (completion, error, interruption) are handled. Task status is derived from the latest iteration.

#### TaskIteration Lifecycle (per execution cycle)

```
                         User sends message
                                │
                                ▼
                        ┌──────────────┐
                        │    active    │
                        └──────┬───────┘
                               │
           ┌───────────────────┼───────────────────┐
           │                   │                   │
           ▼                   ▼                   ▼
   ┌──────────────┐    ┌──────────────┐    ┌──────────────┐
   │  completed   │    │    error     │    │   stopped    │
   │              │    │              │    │ (interrupted)│
   │ Agent done   │    │ Agent failed │    │  User stop   │
   └──────────────┘    └──────────────┘    └──────────────┘
```

#### Task Status (derived from latest iteration)

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                                                                              │
│   ┌─────────┐      ┌─────────┐      ┌───────────────────┐      ┌────────┐  │
│   │ created │─────►│ running │─────►│ completed / failed│─────►│ closed │  │
│   └─────────┘      └────┬────┘      └─────────┬─────────┘      └────────┘  │
│                         │                     │                            │
│                         │    User sends       │                            │
│                         │    new message      │                            │
│                         │◄────────────────────┘                            │
│                         │   (starts new iteration)                         │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘

Status Derivation Rules:
• Task has active iteration        → running
• Latest iteration completed       → completed
• Latest iteration error           → failed
• Latest iteration stopped         → completed (can continue)
• User explicitly closes task      → closed
```

### Task Services Architecture

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                          TASKS SERVICE LAYER                                 │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │                         TaskService                                  │    │
│  │                    (Primary API Surface)                             │    │
│  ├─────────────────────────────────────────────────────────────────────┤    │
│  │  createTask()     - Initialize task + optional first iteration      │    │
│  │  getTask()        - Fetch with access control                       │    │
│  │  listTasks()      - Paginated list with filters                     │    │
│  │  deleteTask()     - Orchestrate full cleanup                        │    │
│  │  sendMessage()    - Create iteration + trigger agent                │    │
│  │  sendCommand()    - Execute slash command                           │    │
│  └──────────────────────────────┬──────────────────────────────────────┘    │
│                                 │                                            │
│                    ┌────────────┴────────────┐                              │
│                    │                         │                              │
│                    ▼                         ▼                              │
│  ┌──────────────────────────┐  ┌──────────────────────────────────────┐    │
│  │ TaskExecutionCoordinator │  │       TaskIterationService           │    │
│  ├──────────────────────────┤  ├──────────────────────────────────────┤    │
│  │ • Workspace provisioning │  │ • createIteration()                  │    │
│  │ • Agent execution        │  │ • applyResult()                      │    │
│  │ • Background processing  │  │ • applyError()                       │    │
│  │ • Timeout handling       │  │ • performPostProcessing()            │    │
│  └──────────────────────────┘  │   (git metadata, screenshots)        │    │
│                                └──────────────────────────────────────┘    │
│                                                                              │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │                      TaskEventProjector                              │    │
│  ├─────────────────────────────────────────────────────────────────────┤    │
│  │  Subscribes to EventService (all events)                            │    │
│  │  • Handles agent:completed → updates iteration with result          │    │
│  │  • Handles agent:error → marks iteration failed                     │    │
│  │  • Handles agent:message → saves message to repository              │    │
│  │  • Emits iteration:completed and task status change events          │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## Workspace Feature

### Workspace Runtime State

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                      WorkspaceRuntimeSnapshot                                │
│                    (Central State Object - DB Persisted)                     │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  taskId: "task-abc123"                                                      │
│  repositoryId: "repo-xyz789"                                                │
│  configRevision: "v5"  ◄─── Optimistic locking                              │
│                                                                              │
│  ┌───────────────────────────────────────────────────────────────────┐      │
│  │                    executionTarget                                 │      │
│  │  ┌─────────────────────────┐    ┌─────────────────────────────┐   │      │
│  │  │   LocalExecutionTarget  │ OR │ KubernetesExecutionTarget   │   │      │
│  │  ├─────────────────────────┤    ├─────────────────────────────┤   │      │
│  │  │ mode: 'local'           │    │ mode: 'kubernetes'          │   │      │
│  │  │ workspacePath: string   │    │ podName: string             │   │      │
│  │  │ isAvailable: boolean    │    │ podNamespace: string        │   │      │
│  │  │                         │    │ podIp: string               │   │      │
│  │  │                         │    │ podPhase: string            │   │      │
│  │  │                         │    │ workerPort: 8001            │   │      │
│  │  └─────────────────────────┘    └─────────────────────────────┘   │      │
│  └───────────────────────────────────────────────────────────────────┘      │
│                                                                              │
│  ┌─────────────────────────────┐  ┌─────────────────────────────────┐      │
│  │        AgentState           │  │         PreviewState            │      │
│  ├─────────────────────────────┤  ├─────────────────────────────────┤      │
│  │ state: 'idle' | 'running'   │  │ status: 'stopped' | 'starting'  │      │
│  │ sessionId?: string          │  │         | 'started' | 'failed'  │      │
│  │ agentType?: string          │  │ endpoint?: string               │      │
│  │ startedAt?: Date            │  │ proxyUrl?: string               │      │
│  │                             │  │ port?: number                   │      │
│  │                             │  │ dirtyContext?: boolean          │      │
│  └─────────────────────────────┘  └─────────────────────────────────┘      │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Provisioning Modes

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                        WORKSPACE PROVISIONING MODES                          │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │                         DIRECT MODE                                  │    │
│  ├─────────────────────────────────────────────────────────────────────┤    │
│  │                                                                      │    │
│  │    Repository Path ──────────────────────────────► Workspace        │    │
│  │    /repos/my-app                                   /repos/my-app    │    │
│  │                                                                      │    │
│  │    • Uses repository path directly (no copy)                        │    │
│  │    • Fastest, minimal overhead                                      │    │
│  │    • Good for: quick iterations, local dev                          │    │
│  │                                                                      │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
│                                                                              │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │                        WORKTREE MODE                                 │    │
│  ├─────────────────────────────────────────────────────────────────────┤    │
│  │                                                                      │    │
│  │    Repository ─────┬─────────────────────────────► Workspace        │    │
│  │    /repos/my-app   │  git worktree add           /workspaces/task-1 │    │
│  │         │          │                                                │    │
│  │         │          └─► Creates isolated branch                      │    │
│  │         │              Shares .git objects                          │    │
│  │         │                                                           │    │
│  │    • Space-efficient (shares git objects)                           │    │
│  │    • Isolated branch per task                                       │    │
│  │    • Good for: parallel tasks on same repo                          │    │
│  │                                                                      │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
│                                                                              │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │                         CLONE MODE                                   │    │
│  ├─────────────────────────────────────────────────────────────────────┤    │
│  │                                                                      │    │
│  │    Remote URL ─────────────────────────────────► Workspace          │    │
│  │    github.com/org/repo    git clone             /workspaces/task-1  │    │
│  │                                                                      │    │
│  │    • Full independent clone                                         │    │
│  │    • Works with remote repositories                                 │    │
│  │    • PAT removed from git config after clone (security)             │    │
│  │    • Good for: Kubernetes pods, CI/CD                               │    │
│  │                                                                      │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## Local vs Kubernetes Execution

### Architecture Comparison

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           LOCAL EXECUTION                                    │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  ┌──────────────┐     ┌──────────────────────────────────────────────┐     │
│  │   Backend    │     │              Local Machine                    │     │
│  │   Server     │     │  ┌────────────────────────────────────────┐  │     │
│  │              │     │  │           Workspace Directory           │  │     │
│  │  ┌────────┐  │     │  │        /data/workspaces/task-123        │  │     │
│  │  │ Local  │──┼─────┼──│                                         │  │     │
│  │  │ Env    │  │     │  │  ┌──────────┐  ┌──────────┐            │  │     │
│  │  │ Driver │  │     │  │  │  Agent   │  │ Preview  │            │  │     │
│  │  └────────┘  │     │  │  │ Process  │  │  Server  │            │  │     │
│  │              │     │  │  │ (child)  │  │ (child)  │            │  │     │
│  │              │     │  │  └──────────┘  └──────────┘            │  │     │
│  │              │     │  └────────────────────────────────────────┘  │     │
│  └──────────────┘     └──────────────────────────────────────────────┘     │
│                                                                              │
│  • Direct filesystem access                                                 │
│  • In-process agent execution (child process)                              │
│  • Local preview server with localhost proxy                               │
│  • Simple state: just check if path exists                                 │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘


┌─────────────────────────────────────────────────────────────────────────────┐
│                         KUBERNETES EXECUTION                                 │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  ┌──────────────┐     ┌──────────────────────────────────────────────┐     │
│  │   Backend    │     │              Kubernetes Cluster               │     │
│  │   Server     │     │                                               │     │
│  │              │     │  ┌────────────────────────────────────────┐  │     │
│  │  ┌────────┐  │     │  │           Worker Pod                    │  │     │
│  │  │  K8s   │──┼─HTTP─┼─│        workspace-abc123                 │  │     │
│  │  │  Env   │  │     │  │                                         │  │     │
│  │  │ Driver │  │     │  │  ┌──────────┐  ┌──────────┐            │  │     │
│  │  └────────┘  │     │  │  │  Worker  │  │ Preview  │            │  │     │
│  │              │     │  │  │   App    │  │  Server  │            │  │     │
│  │              │     │  │  │ :8001    │  │  :3000   │            │  │     │
│  │              │     │  │  └──────────┘  └──────────┘            │  │     │
│  │              │     │  │                                         │  │     │
│  │              │     │  │  Workspace: /repo                       │  │     │
│  │              │     │  └────────────────────────────────────────┘  │     │
│  └──────────────┘     └──────────────────────────────────────────────┘     │
│                                                                              │
│  • HTTP communication to worker pod                                         │
│  • RemoteAgentProxy for agent execution                                    │
│  • Pod clones repository on startup                                        │
│  • Complex state: pod phases (Pending → Running → Failed)                  │
│  • Hostname-based proxy (task-123.proxy.domain.com)                        │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Driver Interface

The `BaseEnvironmentDriver` interface provides a unified abstraction for all environment operations.
Both local and Kubernetes drivers implement this interface, allowing transparent routing based on
the task's execution target.

**Per-Task Driver Instances**: Drivers are created per-task with `taskId` and context in the constructor.
This eliminates the need for workspacePath parameters on methods - context is stored as instance variables.
Drivers are created and cached by `WorkspaceRuntimeManager` and included in the `WorkspaceHandle` returned
by `ensureWorkspace()`. This ensures all workspace operations go through a single entry point.

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                      BaseEnvironmentDriver (Interface)                       │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│   INSTANCE PROPERTY                                                          │
│   ─────────────────                                                          │
│   readonly taskId: string              ← Task context stored on instance    │
│                                                                              │
│   COMMAND EXECUTION                                                          │
│   ─────────────────                                                          │
│   runCommand(command, options)         → AsyncGenerator<LogEvent>           │
│                                                                              │
│   AGENT LIFECYCLE                                                            │
│   ───────────────                                                            │
│   createAgent(agentType, options)      → BaseAgent                          │
│   stopAgent()                          → Promise<void>                      │
│                                                                              │
│   FILE OPERATIONS                                                            │
│   ───────────────                                                            │
│   readFile(filePath)                   → Promise<string>                    │
│   writeFile(filePath, content)         → Promise<void>                      │
│   listFiles(path?, recursive?)         → Promise<string[]>                  │
│   searchFiles(request)                 → Promise<SearchResult>              │
│                                                                              │
│   GIT OPERATIONS                                                             │
│   ──────────────                                                             │
│   getGitStatus()                       → Promise<GitStatus>                 │
│   getGitChanges()                      → Promise<GitChanges>                │
│   pushChanges(options)                 → Promise<PushResult>                │
│   createGitCheckpoint(message)         → Promise<CheckpointResult>          │
│                                                                              │
│   EXTENDED GIT OPERATIONS (for file tree and diff views)                     │
│   ───────────────────────────────────────────────────────                    │
│   getFileTreeWithChanges(rootPath?)    → Promise<FileTreeWithChanges>       │
│   getGitDiff()                         → Promise<GitDiffResult>             │
│   readFileWithChanges(filePath)        → Promise<FileWithChangesResult>     │
│   getCurrentCommitHash()               → Promise<string | null>             │
│                                                                              │
│   PREVIEW SERVER (Optional, K8s only)                                        │
│   ───────────────────────────────────                                        │
│   startPreviewServer?(config)          → Promise<PreviewStatus>             │
│   stopPreviewServer?()                 → Promise<boolean>                   │
│   getPreviewServerStatus?()            → Promise<PreviewStatus>             │
│   getPreviewServerLogs?(lines?)        → Promise<string>                    │
│                                                                              │
└───────────────────────────────────────┬─────────────────────────────────────┘
                                        │
                ┌───────────────────────┴───────────────────────┐
                │                                               │
                ▼                                               ▼
┌───────────────────────────────────────┐   ┌───────────────────────────────────────┐
│     LocalEnvironmentDriver            │   │     KubernetesEnvironmentDriver       │
├───────────────────────────────────────┤   ├───────────────────────────────────────┤
│                                       │   │                                       │
│  constructor(taskId, workspacePath)   │   │  constructor(taskId, target,          │
│                                       │   │              eventService)            │
│  runCommand():                        │   │                                       │
│    ShellCommand.execute()             │   │  runCommand():                        │
│                                       │   │    POST /command → pod:8001           │
│  createAgent():                       │   │                                       │
│    new ClaudeCodeAgent()              │   │  createAgent():                       │
│    Run in-process (child process)     │   │    new RemoteAgentProxy()             │
│                                       │   │    POST /execute-agent → pod          │
│  readFile/writeFile/listFiles:        │   │                                       │
│    Direct fs.* calls                  │   │  readFile/writeFile/listFiles:        │
│                                       │   │    POST /read-file, /write-file, etc. │
│  getGitStatus/getGitChanges:          │   │                                       │
│    Local git utilities                │   │  getGitStatus/getGitChanges:          │
│                                       │   │    GET /git-status, /git-changes      │
│  Preview: PreviewRuntimeService       │   │                                       │
│                                       │   │  Preview: HTTP to pod /server/*       │
└───────────────────────────────────────┘   └───────────────────────────────────────┘
```

### WorkspaceHandle (Driver Access)

The `WorkspaceRuntimeManager.ensureWorkspace(taskId)` method returns a `WorkspaceHandle` that includes
the environment driver. This provides a **single entry point** for all workspace operations:

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                          WorkspaceHandle                                     │
│                  (Returned by ensureWorkspace())                             │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  taskId: string                                                              │
│  repositoryId: string                                                        │
│  workspacePath: string                                                       │
│  mode: 'local' | 'kubernetes'                                                │
│  preview: { status, url, proxyUrl }                                          │
│  agent: { state, sessionId, agentType }                                      │
│  driver: BaseEnvironmentDriver  ← Included in handle                        │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

**Usage Pattern:**
```typescript
// Single entry point - get handle with driver included
const handle = await runtimeManager.ensureWorkspace(taskId)

// Use driver for file/git/agent operations
const status = await handle.driver.getGitStatus()
await handle.driver.writeFile('config.json', content)
const agent = await handle.driver.createAgent('claude-code', options)
```

### FileGateway (Backward-Compatible Wrapper)

The `FileGateway` service provides a taskId-only API for file operations. It acts as a thin wrapper
that delegates to `WorkspaceRuntimeManager` for driver access:

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           FileGateway                                        │
│                (Backward-Compatible File Operations API)                     │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  getDriver(taskId)           → Promise<BaseEnvironmentDriver>               │
│    └─► runtimeManager.ensureWorkspace(taskId).driver                        │
│                                                                              │
│  FILE OPERATIONS (delegate to driver)                                        │
│  ────────────────────────────────────                                        │
│  readFile(taskId, filePath)        → driver.readFile(filePath)              │
│  writeFile(taskId, filePath, ...)  → driver.writeFile(filePath, ...)        │
│  listFiles(taskId, ...)            → driver.listFiles(...)                  │
│  searchFiles(taskId, request)      → driver.searchFiles(request)            │
│  getGitStatus(taskId)              → driver.getGitStatus()                  │
│  getGitChanges(taskId)             → driver.getGitChanges()                 │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

**Key Design Points**:
- **Single Entry Point**: All driver access goes through `ensureWorkspace()` → `WorkspaceHandle.driver`
- **Driver Caching**: Drivers are cached per-task in `WorkspaceRuntimeManager`
- **Context in Driver**: Each driver stores taskId + workspacePath/target, not passed to methods
- **Cache Invalidation**: Occurs on workspace reprovisioning (e.g., pod recreated)

This ensures all file operations work correctly regardless of whether the task is running
locally or in a Kubernetes pod.

### Execution Flow Comparison

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                        LOCAL AGENT EXECUTION                                 │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  AgentController.startAgent(taskId, message)                                │
│       │                                                                      │
│       ▼                                                                      │
│  LocalEnvironmentDriver.createAgent()                                       │
│       │                                                                      │
│       ▼                                                                      │
│  new ClaudeCodeAgent(options)                                               │
│       │                                                                      │
│       ▼                                                                      │
│  agent.run(message) ─────────────────────────────┐                          │
│       │                                          │                          │
│       │  Events emitted directly                 │                          │
│       │                                          ▼                          │
│       │                              ┌──────────────────────┐               │
│       │                              │    EventService      │               │
│       │                              │   (broadcasts SSE)   │               │
│       │                              └──────────────────────┘               │
│       │                                                                      │
│       ▼                                                                      │
│  agent completes → update workspace state                                   │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘


┌─────────────────────────────────────────────────────────────────────────────┐
│                      KUBERNETES AGENT EXECUTION                              │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  AgentController.startAgent(taskId, message)                                │
│       │                                                                      │
│       ▼                                                                      │
│  KubernetesEnvironmentDriver.createAgent()                                  │
│       │                                                                      │
│       ▼                                                                      │
│  new RemoteAgentProxy(podIp, workerPort)                                    │
│       │                                                                      │
│       ├──────────────────────────────────────────────────┐                  │
│       │                                                  │                  │
│       ▼                                                  ▼                  │
│  POST /execute-agent ──────────────► Worker Pod         Subscribe to        │
│       to pod:8001                         │             EventService        │
│                                           │                  │              │
│                                           ▼                  │              │
│                                    Agent runs in pod         │              │
│                                           │                  │              │
│                                           │                  │              │
│                                    POST /emit-event ────────►│              │
│                                    to main backend           │              │
│                                           │                  │              │
│       ┌───────────────────────────────────┘                  │              │
│       │                                                      ▼              │
│       ▼                                          ┌──────────────────────┐   │
│  Poll /agent-status (500ms)                      │    EventService      │   │
│  until completed                                 │   (broadcasts SSE)   │   │
│       │                                          └──────────────────────┘   │
│       ▼                                                                      │
│  agent completes → update workspace state                                   │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## Agents Feature

### Agent Architecture

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           AGENTS FEATURE                                     │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │                        AgentController                               │    │
│  │                    (Lifecycle Orchestrator)                          │    │
│  ├─────────────────────────────────────────────────────────────────────┤    │
│  │  startAgent(taskId, agentType, message, options)                    │    │
│  │    1. Ensure workspace provisioned                                  │    │
│  │    2. Select driver (local/k8s)                                     │    │
│  │    3. Update agent state → 'running'                                │    │
│  │    4. Create agent via driver                                       │    │
│  │    5. Execute and stream events                                     │    │
│  │    6. Update agent state → 'idle' on completion                     │    │
│  │                                                                      │    │
│  │  stopAgent(taskId)                                                  │    │
│  │    1. Call driver.stopAgent()                                       │    │
│  │    2. Update agent state → 'idle'                                   │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
│                                                                              │
│  ┌──────────────────────────────┐  ┌──────────────────────────────────┐    │
│  │       Agent Types            │  │       MessageService             │    │
│  ├──────────────────────────────┤  ├──────────────────────────────────┤    │
│  │                              │  │                                  │    │
│  │  BaseAgent (abstract)        │  │  • Persist user messages         │    │
│  │      │                       │  │  • Persist agent responses       │    │
│  │      ├── ClaudeCodeAgent     │  │  • Fetch message history         │    │
│  │      │   (real AI agent)     │  │  • Fallback to Claude session    │    │
│  │      │                       │  │    replay if DB empty            │    │
│  │      └── TestAgent           │  │                                  │    │
│  │          (mock for tests)    │  │                                  │    │
│  │                              │  │                                  │    │
│  └──────────────────────────────┘  └──────────────────────────────────┘    │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

### MCP (Model Context Protocol) Flow

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                          MCP TOOL EXECUTION                                  │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  ┌──────────────┐                                                           │
│  │    Agent     │  Calls MCP tool (e.g., "take_screenshot")                 │
│  │  (Claude)    │                                                           │
│  └──────┬───────┘                                                           │
│         │                                                                    │
│         │  JSON-RPC request                                                 │
│         ▼                                                                    │
│  ┌──────────────────────────────────────────────────────────────────────┐   │
│  │                          McpService                                   │   │
│  ├──────────────────────────────────────────────────────────────────────┤   │
│  │  handleRequestWithTaskContext(taskId, request)                       │   │
│  │    1. Get task owner ID                                              │   │
│  │    2. Create synthetic session (for access control)                  │   │
│  │    3. Route request:                                                 │   │
│  │       • initialize → server capabilities                             │   │
│  │       • tools/list → available tools                                 │   │
│  │       • tools/call → delegate to ToolService                         │   │
│  └──────────────────────────────┬───────────────────────────────────────┘   │
│                                 │                                            │
│                                 ▼                                            │
│  ┌──────────────────────────────────────────────────────────────────────┐   │
│  │                          ToolService                                  │   │
│  ├──────────────────────────────────────────────────────────────────────┤   │
│  │                                                                       │   │
│  │  Available Tools:                                                    │   │
│  │  ┌────────────────────┐  ┌────────────────────┐  ┌────────────────┐ │   │
│  │  │ updateTaskMetadata │  │      install       │  │   Playwright   │ │   │
│  │  │                    │  │                    │  │     Tools      │ │   │
│  │  │ Updates task name, │  │ Runs setup         │  │                │ │   │
│  │  │ description via    │  │ commands for       │  │ • navigate     │ │   │
│  │  │ TaskService        │  │ preview server     │  │ • click        │ │   │
│  │  │                    │  │                    │  │ • screenshot   │ │   │
│  │  │                    │  │                    │  │ • fillForm     │ │   │
│  │  └────────────────────┘  └────────────────────┘  └────────────────┘ │   │
│  │                                                                       │   │
│  └──────────────────────────────────────────────────────────────────────┘   │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## Preview Feature

### Preview Server Lifecycle

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                        PREVIEW SERVER STATES                                 │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│                         ┌──────────────┐                                    │
│                         │   stopped    │◄─────────────────────┐             │
│                         └──────┬───────┘                      │             │
│                                │                              │             │
│                         User starts server                    │             │
│                                │                         stop() called      │
│                                ▼                              │             │
│                         ┌──────────────┐                      │             │
│                    ┌────│   starting   │────┐                 │             │
│                    │    └──────────────┘    │                 │             │
│                    │                        │                 │             │
│              HTTP ready              Timeout/Error            │             │
│                    │                        │                 │             │
│                    ▼                        ▼                 │             │
│             ┌──────────────┐         ┌──────────────┐        │             │
│             │   started    │         │    failed    │────────┘             │
│             └──────┬───────┘         └──────────────┘                      │
│                    │                        ▲                               │
│                    │                        │                               │
│              Process crash ─────────────────┘                               │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Preview Architecture

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                          PREVIEW FEATURE                                     │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │                    PreviewOrchestratorService                        │    │
│  │                      (High-Level Coordination)                       │    │
│  ├─────────────────────────────────────────────────────────────────────┤    │
│  │  start(taskId)   - Provision workspace, start server                │    │
│  │  stop(taskId)    - Stop running server                              │    │
│  │  restart(taskId) - Stop then start                                  │    │
│  │  getStatus()     - Current server status                            │    │
│  │  streamLogs()    - SSE log streaming                                │    │
│  └──────────────────────────────┬──────────────────────────────────────┘    │
│                                 │                                            │
│              ┌──────────────────┼──────────────────┐                        │
│              │                  │                  │                        │
│              ▼                  ▼                  ▼                        │
│  ┌──────────────────┐ ┌──────────────────┐ ┌──────────────────┐            │
│  │  PreviewServer   │ │ PreviewRuntime   │ │  Playwright      │            │
│  │    (Process)     │ │    Service       │ │  Interaction     │            │
│  ├──────────────────┤ ├──────────────────┤ ├──────────────────┤            │
│  │ • Shell commands │ │ • Active server  │ │ • Browser mgmt   │            │
│  │ • HTTP polling   │ │   registry       │ │ • Screenshots    │            │
│  │ • Proxy creation │ │ • Lifecycle sink │ │ • Navigation     │            │
│  │ • Log streaming  │ │ • Event emission │ │ • DOM interaction│            │
│  └──────────────────┘ └──────────────────┘ └──────────────────┘            │
│                                                                              │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │                         PROXY LAYER                                  │    │
│  ├────────────────────────────────┬────────────────────────────────────┤    │
│  │      PreviewProxyService       │       HostnameProxyService         │    │
│  │       (Per-Task Proxy)         │     (Hostname-Based Routing)       │    │
│  ├────────────────────────────────┼────────────────────────────────────┤    │
│  │  localhost:9001 → :3000        │  task-123.proxy.domain → :3000     │    │
│  │                                │                                    │    │
│  │  • Allocates local port        │  • Subdomain extraction            │    │
│  │  • Script injection            │  • Multi-tenant routing            │    │
│  │  • Auth checking               │  • K8s pod resolution              │    │
│  └────────────────────────────────┴────────────────────────────────────┘    │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Local vs Kubernetes Preview

**Design Decision**: Preview methods are optional in `BaseEnvironmentDriver` and only implemented
in `KubernetesEnvironmentDriver`. This is intentional because:

- **Local mode**: Uses `PreviewRuntimeService` for complex lifecycle management including:
  - PreviewServer instance management with event subscriptions
  - Playwright integration for browser automation
  - Code change detection (dirty context tracking)
  - SSE event emission for real-time updates

- **K8s mode**: Uses `KubernetesEnvironmentDriver` for simple HTTP forwarding to the worker pod

The `PreviewOrchestratorService` routes between these two implementations based on execution target.

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                      PREVIEW ROUTING (PreviewOrchestratorService)            │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  start(taskId)                                                              │
│       │                                                                      │
│       ├─► isKubernetesTask(taskId)?                                         │
│       │       │                                                              │
│       │       ├─► YES: KubernetesEnvironmentDriver.startPreviewServer()     │
│       │       │           (HTTP to pod /server/start)                        │
│       │       │                                                              │
│       │       └─► NO:  PreviewRuntimeService.start()                        │
│       │                   (Complex local lifecycle management)               │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                         LOCAL PREVIEW                                        │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  Client ──► localhost:9001 ──► PreviewProxyService ──► localhost:3000      │
│                                        │                                     │
│                                        │ Inject inspector script             │
│                                        ▼                                     │
│                                   HTML Response                              │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘


┌─────────────────────────────────────────────────────────────────────────────┐
│                       KUBERNETES PREVIEW                                     │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  Client ──► task-123.proxy.moatless.ai ──► HostnameProxyService            │
│                                                    │                         │
│                                                    │ Resolve subdomain       │
│                                                    ▼                         │
│                                           RegistryTargetResolver             │
│                                                    │                         │
│                                                    │ Get pod IP from         │
│                                                    │ workspace snapshot      │
│                                                    ▼                         │
│                                           pod-ip:3000 (inside K8s)          │
│                                                    │                         │
│                                                    │ Inject inspector script │
│                                                    ▼                         │
│                                              HTML Response                   │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## Events Feature

### Event System Architecture

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                          EVENT SYSTEM                                        │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│                         EVENT SOURCES                                        │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐    │
│  │    Agent     │  │    Task      │  │  Workspace   │  │   Preview    │    │
│  │   Events     │  │   Events     │  │   Events     │  │   Events     │    │
│  └──────┬───────┘  └──────┬───────┘  └──────┬───────┘  └──────┬───────┘    │
│         │                 │                 │                 │             │
│         └─────────────────┴─────────────────┴─────────────────┘             │
│                                   │                                          │
│                                   ▼                                          │
│                      ┌───────────────────────┐                              │
│                      │     EventService      │                              │
│                      │  (Central Event Hub)  │                              │
│                      │   extends EventEmitter│                              │
│                      └───────────┬───────────┘                              │
│                                  │                                           │
│              ┌───────────────────┼───────────────────┐                      │
│              │                   │                   │                      │
│              ▼                   ▼                   ▼                      │
│  ┌───────────────────┐ ┌─────────────────┐ ┌─────────────────────┐         │
│  │    Database       │ │  In-Memory      │ │   SSE Broadcast     │         │
│  │   (PostgreSQL)    │ │    Cache        │ │                     │         │
│  ├───────────────────┤ ├─────────────────┤ ├─────────────────────┤         │
│  │ Persists events   │ │ Fast lookups    │ │ Real-time to        │         │
│  │ for recovery      │ │ for recent      │ │ connected clients   │         │
│  │                   │ │ events          │ │                     │         │
│  └───────────────────┘ └─────────────────┘ └─────────────────────┘         │
│                                                                              │
│                         EVENT CONSUMERS                                      │
│  ┌───────────────────────────────────────────────────────────────────────┐  │
│  │                      TaskEventProjector                                │  │
│  │  Subscribes to: agent:completed, agent:error, agent:message           │  │
│  │  Actions: Update iteration, save messages, emit task events           │  │
│  └───────────────────────────────────────────────────────────────────────┘  │
│                                                                              │
│  ┌───────────────────────────────────────────────────────────────────────┐  │
│  │                      PreviewRuntimeService                             │  │
│  │  Subscribes to: agent events                                          │  │
│  │  Actions: Set dirtyContext flag when code modified                    │  │
│  └───────────────────────────────────────────────────────────────────────┘  │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Event Types

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           EVENT TYPES                                        │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  ┌─────────────────┐  ┌─────────────────┐  ┌─────────────────┐             │
│  │   TaskEvent     │  │ TaskIteration   │  │   AgentEvent    │             │
│  │                 │  │     Event       │  │                 │             │
│  ├─────────────────┤  ├─────────────────┤  ├─────────────────┤             │
│  │ • created       │  │ • created       │  │ • started       │             │
│  │ • open          │  │ • active        │  │ • stopped       │             │
│  │ • closed        │  │ • completed     │  │ • message       │             │
│  │ • updated       │  │ • error         │  │ • completed     │             │
│  │                 │  │ • updated       │  │ • error         │             │
│  └─────────────────┘  └─────────────────┘  └─────────────────┘             │
│                                                                              │
│  ┌─────────────────┐  ┌─────────────────┐  ┌─────────────────┐             │
│  │ WorkspaceEvent  │  │  PreviewEvent   │  │  ServerEvent    │             │
│  ├─────────────────┤  ├─────────────────┤  ├─────────────────┤             │
│  │ • provisioning  │  │ • started       │  │ • started       │             │
│  │ • ready         │  │ • stopped       │  │ • stopped       │             │
│  │ • error         │  │ • failed        │  │ • error         │             │
│  │                 │  │ • screenshot    │  │                 │             │
│  └─────────────────┘  └─────────────────┘  └─────────────────┘             │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

### SSE Connection Flow

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                          SSE EVENT STREAMING                                 │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  Client                                      Server                          │
│    │                                           │                             │
│    │  GET /events/stream                       │                             │
│    │──────────────────────────────────────────►│                             │
│    │                                           │                             │
│    │  HTTP 200 (text/event-stream)             │                             │
│    │◄──────────────────────────────────────────│                             │
│    │                                           │                             │
│    │                    ┌──────────────────────┤                             │
│    │                    │  SSEConnectionManager│                             │
│    │                    │  registers connection│                             │
│    │                    └──────────────────────┤                             │
│    │                                           │                             │
│    │  {"type":"ping"}                          │  (every 30s heartbeat)      │
│    │◄──────────────────────────────────────────│                             │
│    │                                           │                             │
│    │  {"type":"agent","subType":"message",...} │  (agent emits event)        │
│    │◄──────────────────────────────────────────│                             │
│    │                                           │                             │
│    │  {"type":"taskIteration",...}             │  (iteration complete)       │
│    │◄──────────────────────────────────────────│                             │
│    │                                           │                             │
│    │                    Connection closed       │                             │
│    │────────────────────────────X              │                             │
│    │                                           │                             │
│    │                    ┌──────────────────────┤                             │
│    │                    │  Cleanup handlers    │                             │
│    │                    │  remove subscription │                             │
│    │                    └──────────────────────┤                             │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## Complete Flow Example

### User Sends Message to Task

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                    COMPLETE MESSAGE FLOW                                     │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  1. CLIENT REQUEST                                                          │
│  ─────────────────                                                          │
│  POST /api/v1/tasks/task-123/messages                                       │
│  { "content": "Add a dark mode toggle" }                                    │
│       │                                                                      │
│       ▼                                                                      │
│  2. TASK SERVICE                                                            │
│  ────────────────                                                           │
│  TaskService.sendMessage()                                                  │
│       │                                                                      │
│       ├─► Load task from repository                                         │
│       │                                                                      │
│       ├─► TaskIterationService.createIteration()                            │
│       │       │                                                              │
│       │       └─► Emit "taskIteration:created" event ──────────► SSE        │
│       │                                                                      │
│       ├─► Save user message to TaskMessageRepository                        │
│       │                                                                      │
│       └─► TaskExecutionCoordinator.sendMessage() (background)               │
│               │                                                              │
│               ▼                                                              │
│  3. WORKSPACE PROVISIONING                                                  │
│  ─────────────────────────                                                  │
│  WorkspaceRuntimeManager.ensureWorkspace()                                  │
│       │                                                                      │
│       ├─► [LOCAL] Apply provision plan (direct/worktree/clone)              │
│       │       Save LocalExecutionTarget to snapshot                         │
│       │                                                                      │
│       └─► [K8S] Create pod, wait for ready                                  │
│               Save KubernetesExecutionTarget to snapshot                    │
│               │                                                              │
│               ▼                                                              │
│  4. AGENT EXECUTION                                                         │
│  ──────────────────                                                         │
│  AgentController.startAgent()                                               │
│       │                                                                      │
│       ├─► Update workspace: agentState → 'running'                          │
│       │                                                                      │
│       ├─► [LOCAL] LocalEnvironmentDriver.createAgent()                      │
│       │       └─► new ClaudeCodeAgent()                                     │
│       │           └─► agent.run() - executes in-process                     │
│       │                                                                      │
│       └─► [K8S] KubernetesEnvironmentDriver.createAgent()                   │
│               └─► new RemoteAgentProxy()                                    │
│                   └─► POST /execute-agent to pod:8001                       │
│                       Poll /agent-status until complete                     │
│               │                                                              │
│               ▼                                                              │
│  5. EVENT STREAMING                                                         │
│  ──────────────────                                                         │
│  Agent emits events during execution:                                       │
│       │                                                                      │
│       ├─► agent:message ──────► EventService ──────► SSE to client          │
│       │                              │                                       │
│       │                              └─► TaskEventProjector                 │
│       │                                      └─► MessageService.save()      │
│       │                                                                      │
│       ├─► agent:completed ────► EventService ──────► SSE to client          │
│       │                              │                                       │
│       │                              └─► TaskEventProjector                 │
│       │                                      ├─► applyResult()              │
│       │                                      ├─► capture git metadata       │
│       │                                      └─► emit iteration:completed   │
│       │                                                                      │
│       └─► [K8S] Worker pod: POST /emit-event to main backend               │
│               │                                                              │
│               ▼                                                              │
│  6. COMPLETION                                                              │
│  ─────────────                                                              │
│  Update workspace: agentState → 'idle'                                      │
│  Task status derived from iteration status                                  │
│  Client receives final taskIteration:completed event                        │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## Key File Locations

| Feature | Key Files |
|---------|-----------|
| **Tasks** | `apps/backend/src/features/tasks/services/task.service.ts`<br>`apps/backend/src/features/tasks/services/task-execution-coordinator.service.ts`<br>`apps/backend/src/features/tasks/services/task-event-projector.service.ts` |
| **Workspace** | `apps/backend/src/features/workspace/services/workspace-runtime-manager.service.ts`<br>`apps/backend/src/features/workspace/drivers/local-environment-driver.ts`<br>`apps/backend/src/features/workspace/drivers/kubernetes-environment-driver.ts` |
| **Agents** | `apps/backend/src/features/agents/services/agent-controller.service.ts`<br>`apps/backend/src/features/agents/services/mcp.service.ts`<br>`apps/backend/src/features/agents/services/tool.service.ts` |
| **Preview** | `apps/backend/src/features/preview/services/preview-orchestrator.service.ts`<br>`apps/backend/src/features/preview/runtime/preview-server.ts`<br>`apps/backend/src/features/preview/services/playwright-interaction.service.ts` |
| **Events** | `apps/backend/src/features/events/services/event.service.ts`<br>`apps/backend/src/features/events/utils/sse-manager.ts` |

---

## Configuration

### Environment Variables

```bash
# Execution mode
WORKSPACE_EXECUTION_MODE=local|kubernetes

# Kubernetes settings
K8S_NAMESPACE=moatless
K8S_WORKER_IMAGE=moatless/worker:latest
K8S_RESOURCE_LIMIT_CPU=2
K8S_RESOURCE_LIMIT_MEMORY=4Gi
K8S_POD_STARTUP_TIMEOUT=120

# Preview proxy
USE_HOSTNAME_ROUTING=true
PROXY_DOMAIN=proxy.moatless.ai
PROXY_PROTOCOL=https

# Database
DATABASE_PROVIDER=sqlite|postgres
DATABASE_URL=postgres://...
```
