# Moatless Vibe Architecture

## Table of Contents
1. [System Overview](#system-overview)
2. [Core Applications](#core-applications)
3. [Sandbox Application](#sandbox-application)
4. [Backend Application](#backend-application)
5. [Request Flow](#request-flow)
6. [Tasks Feature](#tasks-feature)
7. [Workspace Feature](#workspace-feature)
8. [Docker vs Kubernetes Execution](#docker-vs-kubernetes-execution)
9. [Agents Feature](#agents-feature)
10. [Preview Feature](#preview-feature)
11. [Events Feature](#events-feature)
12. [Complete Flow Example](#complete-flow-example)

---

## System Overview

The system consists of three main applications organized in a monorepo:
- **Frontend**: React application for the user interface
- **Backend**: Hono-based API server for orchestration and state management
- **Sandbox**: Hono-based execution environment for agent and preview workloads

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                              CLIENT (Frontend)                               │
│                    React + TanStack Query + TanStack Router                  │
└─────────────────────────────────────────────────────────────────────────────┘
                                      │
                                      ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                           BACKEND APPLICATION                                │
│                         (Orchestration & State)                              │
├─────────────────────────────────────────────────────────────────────────────┤
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐    │
│  │    Tasks     │  │  Workspace   │  │   Preview    │  │   Agents     │    │
│  │   Service    │  │   Manager    │  │ Orchestrator │  │  Controller  │    │
│  └──────────────┘  └──────────────┘  └──────────────┘  └──────────────┘    │
│                              │                                               │
│                              │ HTTP API                                      │
│                              ▼                                               │
├─────────────────────────────────────────────────────────────────────────────┤
│                         DATABASE LAYER                                       │
│                          PostgreSQL                                          │
└─────────────────────────────────────────────────────────────────────────────┘
                                      │
                              HTTP Communication
                                      │
                                      ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                          SANDBOX APPLICATION                                 │
│                    (Per-Task Execution Environment)                          │
├─────────────────────────────────────────────────────────────────────────────┤
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐    │
│  │   Agents     │  │    Files     │  │     Git      │  │   Preview    │    │
│  │ (Claude SDK) │  │  Operations  │  │  Operations  │  │   Server     │    │
│  └──────────────┘  └──────────────┘  └──────────────┘  └──────────────┘    │
│                              │                                               │
│                              ▼                                               │
│                    ┌──────────────────┐                                     │
│                    │    Workspace     │                                     │
│                    │   (Filesystem)   │                                     │
│                    └──────────────────┘                                     │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## Core Applications

### Application Responsibilities

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                        APPLICATION DOMAINS                                   │
├─────────────────┬───────────────────────────────────────────────────────────┤
│                 │                                                           │
│    FRONTEND     │  • User interface (React)                                 │
│   apps/frontend │  • Task creation and management UI                        │
│                 │  • Real-time updates via SSE                              │
│                 │  • Preview iframe embedding                               │
│                 │  • File tree and diff visualization                       │
│                 │                                                           │
├─────────────────┼───────────────────────────────────────────────────────────┤
│                 │                                                           │
│    BACKEND      │  • Task lifecycle orchestration                           │
│   apps/backend  │  • Workspace provisioning coordination                    │
│                 │  • Event streaming (SSE)                                  │
│                 │  • State persistence (PostgreSQL)                         │
│                 │  • Docker container / Kubernetes pod management           │
│                 │  • MCP server for agent tools                             │
│                 │                                                           │
├─────────────────┼───────────────────────────────────────────────────────────┤
│                 │                                                           │
│    SANDBOX      │  • Agent execution (Claude Code SDK)                      │
│   apps/sandbox  │  • File operations (read, write, list, search)            │
│                 │  • Git operations (status, diff, changes)                 │
│                 │  • Preview server lifecycle                               │
│                 │  • HTTP proxy with script injection                       │
│                 │  • Workspace provisioning (git clone)                     │
│                 │                                                           │
└─────────────────┴───────────────────────────────────────────────────────────┘
```

---

## Sandbox Application

The sandbox is a standalone Hono-based HTTP server that provides an isolated execution environment for each task. It runs on port 8001 by default and handles all workspace-local operations.

### Sandbox Architecture

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                          SANDBOX APPLICATION                                 │
│                       apps/sandbox/src/index.ts                              │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  STARTUP SEQUENCE                                                            │
│  ────────────────                                                            │
│  1. Load configuration from environment variables                            │
│  2. Provision workspace (clone git repo if configured)                       │
│  3. Start HTTP server on configured port                                     │
│                                                                              │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │                           HTTP API                                   │    │
│  │                        (Hono Routes)                                 │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
│                                                                              │
│  ┌──────────────────┐  ┌──────────────────┐  ┌──────────────────┐          │
│  │  Agent Routes    │  │   File Routes    │  │   Git Routes     │          │
│  ├──────────────────┤  ├──────────────────┤  ├──────────────────┤          │
│  │ POST /execute-   │  │ POST /read-file  │  │ GET /git-status  │          │
│  │      agent       │  │ POST /write-file │  │ GET /git-changes │          │
│  │ POST /stop-agent │  │ POST /list-files │  │ GET /git-diff    │          │
│  │ GET /agent-status│  │ POST /search-    │  │ GET /current-    │          │
│  │                  │  │      files       │  │     commit-hash  │          │
│  │                  │  │ GET /file-tree-  │  │                  │          │
│  │                  │  │     with-changes │  │                  │          │
│  └──────────────────┘  └──────────────────┘  └──────────────────┘          │
│                                                                              │
│  ┌──────────────────┐  ┌──────────────────┐  ┌──────────────────┐          │
│  │  Preview Routes  │  │ Workspace Routes │  │  Browser Routes  │          │
│  ├──────────────────┤  ├──────────────────┤  ├──────────────────┤          │
│  │ GET /server/     │  │ GET /workspace/  │  │ GET /browser/    │          │
│  │     status       │  │      status      │  │     start        │          │
│  │ GET /server/logs │  │ GET /health      │  │ GET /browser/    │          │
│  │ GET /server/     │  │                  │  │     stop         │          │
│  │     logs/stream  │  │                  │  │ POST /browser/   │          │
│  │ ALL /preview/*   │  │                  │  │     screenshot   │          │
│  └──────────────────┘  └──────────────────┘  └──────────────────┘          │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Sandbox Configuration

The sandbox is configured via environment variables:

```bash
# Required
TASK_ID=task-abc123                    # Task ID this sandbox is scoped to

# Workspace
WORKSPACE_DIR=/opt/moatless/repo       # Workspace directory path
DATA_DIR=/opt/moatless/data            # Data directory for storage

# Git (optional - for auto-provisioning)
GIT_CLONE_URL=https://github.com/...   # Repository to clone on startup
GIT_BRANCH=main                        # Branch to checkout

# Network
PORT=8001                              # HTTP server port

# Repository configuration (JSON - includes preview config, setup commands)
REPO_CONFIG='{"previewConfig":{...},"codeExecution":{"setupCommands":[...]}}'

# Claude Code (for agent execution)
CLAUDE_CODE_OAUTH_TOKEN=...            # OAuth token for Claude Code
CLAUDE_CONFIG_DIR=/opt/moatless/.claude  # Claude config directory

# Event streaming
REDIS_URL=redis://localhost:6379       # Redis for event transport
```

### Sandbox Features

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                          SANDBOX FEATURES                                    │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │                         AGENT EXECUTION                              │    │
│  ├─────────────────────────────────────────────────────────────────────┤    │
│  │  • ClaudeCodeAgent wraps @anthropic-ai/claude-agent-sdk              │    │
│  │  • Emits all events directly to Redis via EventService               │    │
│  │  • Supports resume sessions for conversation continuity              │    │
│  │  • MCP tools integration via HTTP endpoint                           │    │
│  │  • Plan mode support for structured task execution                   │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
│                                                                              │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │                         FILE OPERATIONS                              │    │
│  ├─────────────────────────────────────────────────────────────────────┤    │
│  │  • Read/write files within workspace                                 │    │
│  │  • List files with optional recursion                                │    │
│  │  • Search files by name pattern                                      │    │
│  │  • File tree with git change annotations                             │    │
│  │  • Security: Path traversal prevention                               │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
│                                                                              │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │                         GIT OPERATIONS                               │    │
│  ├─────────────────────────────────────────────────────────────────────┤    │
│  │  • Git status (branch, uncommitted changes, ahead/behind)            │    │
│  │  • Git changes (additions, deletions, modifications)                 │    │
│  │  • Git diff (detailed file-level changes)                            │    │
│  │  • Current commit hash                                               │    │
│  │  • File content with git change info (old vs new)                    │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
│                                                                              │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │                        PREVIEW SERVER                                │    │
│  ├─────────────────────────────────────────────────────────────────────┤    │
│  │  • Monitor preview server process via filesystem artifacts           │    │
│  │  • Shell script management (/opt/moatless/bin/preview-server)        │    │
│  │  • HTTP proxy with inspector script injection                        │    │
│  │  • Log streaming via SSE                                             │    │
│  │  • Agent starts/stops server via skills (preview-server skill)       │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
│                                                                              │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │                      BROWSER AUTOMATION                              │    │
│  ├─────────────────────────────────────────────────────────────────────┤    │
│  │  • Playwright + Chromium via CDP debugging port                      │    │
│  │  • Screenshot capture and management                                 │    │
│  │  • Agent interacts via skills (sandbox-browser skill)                │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
│                                                                              │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │                      WORKSPACE PROVISIONING                          │    │
│  ├─────────────────────────────────────────────────────────────────────┤    │
│  │  • Auto-clone git repository on startup                              │    │
│  │  • Branch checkout support                                           │    │
│  │  • Skip clone if .git already exists                                 │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## Backend Application

The backend orchestrates tasks and communicates with sandbox instances via HTTP.

### Backend Architecture

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                          BACKEND APPLICATION                                 │
│                        apps/backend/src/index.ts                             │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │                         API GATEWAY                                  │    │
│  │                    Hono Routes + OpenAPI                             │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
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
│                           PostgreSQL                                         │
└─────────────────────────────────────────────────────────────────────────────┘
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
                             │   Backend (Hono)      │
                             │  + Auth Middleware    │
                             └───────────┬───────────┘
                                         │
                     ┌───────────────────┼───────────────────┐
                     │                   │                   │
                     ▼                   ▼                   ▼
            ┌──────────────┐    ┌──────────────┐    ┌──────────────┐
            │    Task      │    │   Workspace  │    │   Preview    │
            │   Service    │    │   Manager    │    │ Orchestrator │
            └──────┬───────┘    └──────┬───────┘    └──────┬───────┘
                   │                   │                   │
                   └───────────────────┼───────────────────┘
                                       │
                               HTTP to Sandbox
                                       │
                                       ▼
                             ┌───────────────────────┐
                             │  Sandbox Application  │
                             │   (Execution Env)     │
                             └───────────────────────┘
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

Tasks contain multiple **iterations** - each iteration represents one user message → agent execution cycle.

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
│  snapshotRevision: "v5"  ◄─── Optimistic locking                            │
│                                                                              │
│  ┌───────────────────────────────────────────────────────────────────┐      │
│  │                    executionTarget                                 │      │
│  │  ┌─────────────────────────┐    ┌─────────────────────────────┐   │      │
│  │  │   DockerExecutionTarget │ OR │ KubernetesExecutionTarget   │   │      │
│  │  ├─────────────────────────┤    ├─────────────────────────────┤   │      │
│  │  │ mode: 'docker'          │    │ mode: 'kubernetes'          │   │      │
│  │  │ containerName: string   │    │ podName: string             │   │      │
│  │  │ containerStatus: string │    │ podNamespace: string        │   │      │
│  │  │ hostPort: number        │    │ podIp: string               │   │      │
│  │  │ workerPort: 8001        │    │ podPhase: string            │   │      │
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

---

## Docker vs Kubernetes Execution

Both Docker and Kubernetes modes run sandbox containers that communicate with the backend via HTTP. The key difference is how containers are orchestrated and accessed.

### Architecture Comparison

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           DOCKER EXECUTION                                   │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  ┌──────────────┐     ┌──────────────────────────────────────────────┐     │
│  │   Backend    │     │              Local Docker Host                │     │
│  │   Server     │     │                                               │     │
│  │              │     │  ┌────────────────────────────────────────┐  │     │
│  │  ┌────────┐  │     │  │         Sandbox Container              │  │     │
│  │  │ Docker │──┼─HTTP─┼─│        workspace-abc123                 │  │     │
│  │  │  Env   │  │     │  │                                         │  │     │
│  │  │ Driver │  │     │  │  ┌──────────┐  ┌──────────┐            │  │     │
│  │  └────────┘  │     │  │  │ Sandbox  │  │ Preview  │            │  │     │
│  │              │     │  │  │   App    │  │  Server  │            │  │     │
│  │              │     │  │  │ :8001    │  │  :3000   │            │  │     │
│  │              │     │  │  └──────────┘  └──────────┘            │  │     │
│  │              │     │  │                                         │  │     │
│  │              │     │  │  Workspace: /opt/moatless/repo     │  │     │
│  │              │     │  └────────────────────────────────────────┘  │     │
│  └──────────────┘     └──────────────────────────────────────────────┘     │
│                                                                              │
│  • HTTP communication via localhost port mapping                            │
│  • Each container gets unique host port (e.g., localhost:9001)             │
│  • Container clones repository on startup                                   │
│  • Preview accessed via localhost:proxyPort                                 │
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
│  │  ┌────────┐  │     │  │           Sandbox Pod                  │  │     │
│  │  │  K8s   │──┼─HTTP─┼─│        workspace-abc123                 │  │     │
│  │  │  Env   │  │     │  │                                         │  │     │
│  │  │ Driver │  │     │  │  ┌──────────┐  ┌──────────┐            │  │     │
│  │  └────────┘  │     │  │  │ Sandbox  │  │ Preview  │            │  │     │
│  │              │     │  │  │   App    │  │  Server  │            │  │     │
│  │              │     │  │  │ :8001    │  │  :3000   │            │  │     │
│  │              │     │  │  └──────────┘  └──────────┘            │  │     │
│  │              │     │  │                                         │  │     │
│  │              │     │  │  Workspace: /opt/moatless/repo     │  │     │
│  │              │     │  └────────────────────────────────────────┘  │     │
│  └──────────────┘     └──────────────────────────────────────────────┘     │
│                                                                              │
│  • HTTP communication to pod via internal cluster IP                        │
│  • RemoteAgentProxy for agent execution                                    │
│  • Pod clones repository on startup                                        │
│  • Hostname-based proxy (task-123.proxy.domain.com)                        │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Environment Driver Interface

All workspaces use the same `RemoteEnvironmentDriver` which communicates with sandbox containers/pods via HTTP. The provisioners create drivers with different URL resolution strategies.

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
│   getFileTreeWithChanges(rootPath?)    → Promise<FileTreeWithChanges>       │
│   getGitDiff()                         → Promise<GitDiffResult>             │
│   getCurrentCommitHash()               → Promise<string | null>             │
│                                                                              │
│   PREVIEW SERVER                                                             │
│   ──────────────                                                             │
│   getPreviewServerStatus()             → Promise<PreviewStatus>             │
│   getPreviewServerLogs(lines?)         → Promise<string>                    │
│   streamPreviewServerLogs(lines?)      → AsyncGenerator<LogEvent>           │
│                                                                              │
└───────────────────────────────────────┬─────────────────────────────────────┘
                                        │
                                        │ Both provisioners create
                                        │
                                        ▼
                ┌───────────────────────────────────────────────────────────────┐
                │                   RemoteEnvironmentDriver                     │
                │                   (Single implementation)                     │
                ├───────────────────────────────────────────────────────────────┤
                │                                                               │
                │  Unified HTTP driver for all remote workspaces                │
                │                                                               │
                │  Constructor receives URL provider:                           │
                │    getWorkerUrl: () => Promise<string>                        │
                │                                                               │
                │  Docker provisioner creates driver with:                      │
                │    getWorkerUrl: () => `http://localhost:${hostPort}`         │
                │                                                               │
                │  Kubernetes provisioner creates driver with:                  │
                │    getWorkerUrl: async () => {                                │
                │      const podIp = await kubernetesService.getPodIp(taskId)   │
                │      return `http://${podIp}:${workerPort}`                   │
                │    }                                                          │
                │                                                               │
                │  HTTP Methods:                                                │
                │  ─────────────                                                │
                │  runCommand():     POST /command → sandbox                    │
                │  createAgent():    POST /execute-agent → sandbox              │
                │  readFile():       POST /read-file → sandbox                  │
                │  writeFile():      POST /write-file → sandbox                 │
                │  Preview:          HTTP to sandbox /server/*                  │
                │                                                               │
                └───────────────────────────────────────────────────────────────┘

                                    Architecture:
                ┌───────────────────────────────────────────────────────────────┐
                │                                                               │
                │  Routes → WorkspaceRuntimeManager → Provisioner → Driver      │
                │                                                               │
                │  ┌─────────────────────────────────────────────────────────┐  │
                │  │              WorkspaceRuntimeManager                    │  │
                │  │  • Provisions workspaces via WorkspaceProvisioner       │  │
                │  │  • Manages runtime state (snapshots, locks)             │  │
                │  │  • Creates provision plans from repository config       │  │
                │  │  • Returns WorkspaceHandle with driver                  │  │
                │  └─────────────────────────────────────────────────────────┘  │
                │                          │                                    │
                │                          ▼                                    │
                │  ┌─────────────────────────────────────────────────────────┐  │
                │  │              WorkspaceProvisioner                       │  │
                │  ├──────────────────────┬──────────────────────────────────┤  │
                │  │ DockerWorkspace      │ KubernetesWorkspace              │  │
                │  │ Provisioner          │ Provisioner                      │  │
                │  ├──────────────────────┼──────────────────────────────────┤  │
                │  │ • Creates containers │ • Creates K8s pods               │  │
                │  │ • Port allocation    │ • Pod IP resolution              │  │
                │  │ • Container health   │ • Pod health checks              │  │
                │  │ • Creates driver     │ • Creates driver                 │  │
                │  │   with localhost URL │   with async pod IP URL          │  │
                │  └──────────────────────┴──────────────────────────────────┘  │
                │                                                               │
                └───────────────────────────────────────────────────────────────┘
```

### Agent Execution Flow

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                      REMOTE AGENT EXECUTION                                  │
│                   (Docker and Kubernetes modes)                              │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  AgentController.startAgent(taskId, message)                                │
│       │                                                                      │
│       ▼                                                                      │
│  RemoteEnvironmentDriver.createAgent()                                      │
│       │                                                                      │
│       ▼                                                                      │
│  new RemoteAgentProxy(workerUrl)                                            │
│       │                                                                      │
│       ▼                                                                      │
│  POST /execute-agent ──────────────► Sandbox Container                      │
│       to workerUrl                        │                                 │
│                                           ▼                                 │
│                                   ClaudeCodeAgent                           │
│                                   runs in sandbox                           │
│                                           │                                 │
│                                           │ Events emitted                  │
│                                           ▼ directly to Redis               │
│                                   EventService.emitEvent()                  │
│                                           │                                 │
│                                           ▼                                 │
│                               ┌──────────────────────┐                      │
│                               │    EventService      │                      │
│                               │  (Redis Pub/Sub)     │                      │
│                               └──────────┬───────────┘                      │
│                                          │                                  │
│                          ┌───────────────┼───────────────┐                  │
│                          ▼               ▼               ▼                  │
│                 TaskIteration      SSE Broadcast    MessageService           │
│                   Service          to clients       (saves messages)         │
│                                                                              │
│  Meanwhile: AgentController drains the stream                               │
│  (for await loop drives execution, no event processing)                     │
│       │                                                                      │
│       ▼                                                                      │
│  agent completes → update workspace state → 'idle'                          │
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
│  BACKEND (Orchestration)                                                     │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │                        AgentController                               │    │
│  ├─────────────────────────────────────────────────────────────────────┤    │
│  │  startAgent(taskId, agentType, message, options)                    │    │
│  │    1. Ensure workspace provisioned                                  │    │
│  │    2. Get driver from provisioner (docker/k8s)                      │    │
│  │    3. Update agent state → 'running'                                │    │
│  │    4. Create agent via driver                                       │    │
│  │    5. Drain agent stream (events emitted by sandbox via Redis)      │    │
│  │    6. Update agent state → 'idle' on completion                     │    │
│  │                                                                      │    │
│  │  TaskIterationService (Event Subscription)                          │    │
│  │    Subscribes to agent events from Redis:                           │    │
│  │    • agent:started  → update runtime state                          │    │
│  │    • agent:message  → capture sessionId, contextTokens              │    │
│  │    • agent:completed → apply result, post-processing, metrics       │    │
│  │    • agent:error    → mark iteration failed, emit error events      │    │
│  │                                                                      │    │
│  │  stopAgent(taskId)                                                  │    │
│  │    1. Call driver.stopAgent()                                       │    │
│  │    2. Update agent state → 'idle'                                   │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
│                                                                              │
│  SANDBOX (Execution)                                                         │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │                        ClaudeCodeAgent                               │    │
│  ├─────────────────────────────────────────────────────────────────────┤    │
│  │  • Wraps @anthropic-ai/claude-agent-sdk                             │    │
│  │  • Emits ALL events directly to Redis via EventService              │    │
│  │  • Supports plan mode (ExitPlanMode detection)                      │    │
│  │  • Configurable allowed tools                                       │    │
│  │  • MCP server integration for custom tools                          │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
│                                                                              │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │                      MCP Tool Integration                            │    │
│  ├─────────────────────────────────────────────────────────────────────┤    │
│  │  Agent calls MCP tools via HTTP:                                    │    │
│  │  POST http://localhost:8000/api/v1/tasks/{taskId}/mcp               │    │
│  │                                                                      │    │
│  │  Available Tools:                                                   │    │
│  │  • updateTaskMetadata - Update task name/description                │    │
│  │                                                                      │    │
│  │  Note: Preview server and browser are managed via Claude Code       │    │
│  │  skills (preview-server, sandbox-browser), not MCP tools.           │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## Preview Feature

### Preview Server Lifecycle

The preview server is managed by a shell script (`/opt/moatless/bin/preview-server`) invoked by the Claude Code agent via the `preview-server` skill. The sandbox monitors the server process via filesystem artifacts.

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                        PREVIEW SERVER STATES                                 │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│                         ┌──────────────┐                                    │
│                         │   stopped    │◄─────────────────────┐             │
│                         └──────┬───────┘                      │             │
│                                │                              │             │
│                   Agent runs shell script:                    │             │
│              /opt/moatless/bin/preview-server start           │             │
│                                │                     script: stop           │
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
│  Filesystem Artifacts:                                                       │
│    /opt/moatless/data/preview-server.status.json  ← status + port + PID    │
│    /opt/moatless/data/preview-server.pid          ← process ID             │
│    /opt/moatless/data/preview-server.log          ← server output          │
│    /opt/moatless/data/preview-server.config.json  ← server commands + port │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Preview Architecture

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                          PREVIEW FEATURE                                     │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  BACKEND (Orchestration)                                                     │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │                    PreviewOrchestratorService                        │    │
│  ├─────────────────────────────────────────────────────────────────────┤    │
│  │  getStatus(taskId)       - Read status from sandbox                 │    │
│  │  getLogs(taskId)         - Fetch server logs                        │    │
│  │  streamLogs(taskId)      - SSE log streaming                        │    │
│  │  getBrowserScreenshots() - List captured screenshots                │    │
│  │                                                                      │    │
│  │  Note: start/stop/restart removed — agent manages lifecycle         │    │
│  │  via /opt/moatless/bin/preview-server shell script                  │    │
│  └──────────────────────────────┬──────────────────────────────────────┘    │
│                                 │                                            │
│                                 │ HTTP via driver                            │
│                                 ▼                                            │
│  SANDBOX (Execution)                                                         │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │                     PreviewProcessMonitor                            │    │
│  ├─────────────────────────────────────────────────────────────────────┤    │
│  │  • Watches filesystem for status.json changes                       │    │
│  │  • Emits server events to Redis on status transitions               │    │
│  │  • Verifies PID is alive for accurate status                        │    │
│  │  • Provides log tailing and streaming                               │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
│                                                                              │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │                     handleProxyRequest()                             │    │
│  ├─────────────────────────────────────────────────────────────────────┤    │
│  │  • Stateless HTTP proxy from /preview/* to localhost:port            │    │
│  │  • Injects inspector script into HTML responses                     │    │
│  │  • Strips CSP/X-Frame-Options for iframe embedding                  │    │
│  │  • Raw pipes non-HTML requests (preserves encoding)                 │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
│                                                                              │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │                Shell Script: /opt/moatless/bin/preview-server        │    │
│  ├─────────────────────────────────────────────────────────────────────┤    │
│  │  Commands: start | stop | restart | status | logs [-n N] [-f]       │    │
│  │  • Reads config from preview-server.config.json                     │    │
│  │  • Replaces {{PORT}} placeholder in server commands                 │    │
│  │  • Polls HTTP endpoint for readiness                                │    │
│  │  • Writes status.json with state, PID, port, error                  │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Preview Routing (Docker vs Kubernetes)

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                         DOCKER PREVIEW                                       │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  Client ──► localhost:proxyPort ──► sandbox:8001/preview/*                  │
│                                              │                               │
│                                              │ handleProxyRequest()          │
│                                              │ Injects inspector script      │
│                                              ▼                               │
│                                         HTML Response                        │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘


┌─────────────────────────────────────────────────────────────────────────────┐
│                       KUBERNETES PREVIEW                                     │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  Client ──► task-123.proxy.moatless.xyz ──► HostnameProxyService            │
│                                                    │                         │
│                                                    │ Resolve subdomain       │
│                                                    ▼                         │
│                                           RegistryTargetResolver             │
│                                                    │                         │
│                                                    │ Get pod IP from         │
│                                                    │ workspace snapshot      │
│                                                    ▼                         │
│                                           pod-ip:8001/preview/*              │
│                                                    │                         │
│                                                    │ handleProxyRequest()    │
│                                                    │ Injects inspector script│
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
│                                                                              │
│  SANDBOX (Agent/Preview Events)      BACKEND (Task/Workspace Events)        │
│  ┌──────────────────────┐          ┌──────────────────────────────────┐    │
│  │ Agent events emitted │          │ Task, Workspace, Preview         │    │
│  │ directly to Redis    │          │ Services emit events             │    │
│  └──────────┬───────────┘          └────────────────┬─────────────────┘    │
│             │                                       │                      │
│             │ Redis Pub/Sub                         │                      │
│             │                                       │                      │
│             └───────────────────┬───────────────────┘                      │
│                                 │                                          │
│                                 ▼                                          │
│                      ┌───────────────────────┐                              │
│                      │     EventService      │                              │
│                      │  (Central Event Hub)  │                              │
│                      │   Redis Pub/Sub       │                              │
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
│  │                    TaskIterationService                                 │  │
│  │  Subscribes to: agent:started, agent:message, agent:completed,        │  │
│  │                  agent:error                                            │  │
│  │  Actions:                                                              │  │
│  │  • started: Update agent runtime state                                 │  │
│  │  • message: Capture sessionId, track contextTokens                    │  │
│  │  • completed: Apply result, post-processing, metrics, emit events     │  │
│  │  • error: Mark iteration failed, capture state, emit error events     │  │
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
│  2. TASK SERVICE (Backend)                                                  │
│  ─────────────────────────                                                  │
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
│  3. WORKSPACE PROVISIONING (Backend)                                        │
│  ───────────────────────────────────                                        │
│  WorkspaceRuntimeManager.ensureWorkspace()                                  │
│       │                                                                      │
│       ├─► Provisioner creates sandbox container/pod with env vars:          │
│       │         TASK_ID, GIT_CLONE_URL, GIT_BRANCH                   │
│       │                                                                      │
│       └─► Wait for container/pod ready / sandbox HTTP healthy               │
│               │                                                              │
│               ▼                                                              │
│  4. SANDBOX STARTUP (Sandbox Container)                                           │
│  ────────────────────────────────                                           │
│  provisionWorkspace()                                                       │
│       │                                                                      │
│       ├─► Clone git repository                                              │
│       ├─► Checkout branch                                                   │
│       └─► Start HTTP server on :8001                                        │
│               │                                                              │
│               ▼                                                              │
│  5. AGENT EXECUTION (Backend → Sandbox)                                     │
│  ──────────────────────────────────────                                     │
│  AgentController.startAgent()                                               │
│       │                                                                      │
│       ├─► Update workspace: agentState → 'running'                          │
│       │                                                                      │
│       └─► RemoteEnvironmentDriver.createAgent()                             │
│               │                                                              │
│               └─► RemoteAgentProxy                                          │
│                       │                                                      │
│                       └─► POST /execute-agent to sandbox workerUrl          │
│                               │                                              │
│                               ▼                                              │
│  6. AGENT EXECUTION (Sandbox)                                               │
│  ────────────────────────────                                               │
│  ClaudeCodeAgent.run(message)                                               │
│       │                                                                      │
│       ├─► Call Claude Code SDK                                              │
│       ├─► Execute tools (Read, Write, Bash, etc.)                           │
│       ├─► Call MCP tools via HTTP (updateTaskMetadata, etc.)                │
│       │                                                                      │
│       └─► Emit ALL events directly to Redis via EventService                │
│               │                                                              │
│               ▼                                                              │
│  7. EVENT HANDLING (Backend - TaskIterationService)                          │
│  ─────────────────────────────────────────────────                          │
│  TaskIterationService subscribes to agent events from Redis:                │
│       │                                                                      │
│       ├─► agent:started ─────► Update agent runtime state                   │
│       │                                                                      │
│       ├─► agent:message ─────► Capture sessionId, contextTokens             │
│       │                         MessageService.save() (via subscription)     │
│       │                                                                      │
│       ├─► agent:completed ───► applyResult()                                │
│       │                         performPostProcessing()                      │
│       │                         Record Prometheus metrics                    │
│       │                         emit taskIteration:completed                 │
│       │                                                                      │
│       └─► agent:error ───────► Mark iteration failed                        │
│                                 performPostProcessing()                      │
│                                 emit taskIteration:error                     │
│               │                                                              │
│               ▼                                                              │
│  8. COMPLETION                                                              │
│  ─────────────                                                              │
│  AgentController finishes draining stream                                   │
│  Update workspace: agentState → 'idle'                                      │
│  Task status derived from iteration status                                  │
│  Client receives final taskIteration:completed event via SSE                │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## Key File Locations

| Component | Key Files |
|-----------|-----------|
| **Sandbox App** | `apps/sandbox/src/index.ts` (entry point)<br>`apps/sandbox/src/app.ts` (Hono app)<br>`apps/sandbox/src/features/agents/claude-code/claude-code-agent.ts`<br>`apps/sandbox/src/features/preview/services/preview-process-monitor.ts`<br>`apps/sandbox/src/features/preview/runtime/preview-proxy.ts`<br>`apps/sandbox/bin/preview-server` (shell script)<br>`apps/sandbox/src/features/files/services/file.service.ts`<br>`apps/sandbox/src/features/git/services/git.service.ts`<br>`apps/sandbox/src/features/browser/services/browser-server.ts` |
| **Backend Tasks** | `apps/backend/src/features/tasks/services/task.service.ts`<br>`apps/backend/src/features/tasks/services/task-execution-coordinator.service.ts`<br>`apps/backend/src/features/tasks/services/task-iteration.service.ts` |
| **Backend Workspace** | `apps/backend/src/features/workspace/services/workspace-runtime-manager.service.ts` (provisioning + state)<br>`apps/backend/src/features/workspace/drivers/remote-environment-driver.ts` (HTTP client)<br>`apps/backend/src/features/workspace/drivers/remote-agent-proxy.ts`<br>`apps/backend/src/features/workspace/provisioners/docker-workspace-provisioner.ts`<br>`apps/backend/src/features/workspace/provisioners/kubernetes-workspace-provisioner.ts` |
| **Backend Agents** | `apps/backend/src/features/agents/services/agent-controller.service.ts`<br>`apps/backend/src/features/agents/services/mcp.service.ts` |
| **Backend Preview** | `apps/backend/src/features/preview/services/preview-orchestrator.service.ts`<br>`apps/backend/src/features/preview/services/preview-runtime.service.ts` |
| **Backend Events** | `apps/backend/src/features/events/services/event.service.ts`<br>`apps/backend/src/features/events/utils/sse-manager.ts` |

---

## Configuration

### Environment Variables

```bash
# Backend - Execution mode (docker or kubernetes)
WORKSPACE_EXECUTION_MODE=docker|kubernetes  # Default: kubernetes

# Backend - Docker settings (when WORKSPACE_EXECUTION_MODE=docker)
DOCKER_SANDBOX_IMAGE=ghcr.io/aorwall/moatless-vibe-worker:latest

# Backend - Kubernetes settings (when WORKSPACE_EXECUTION_MODE=kubernetes)
K8S_NAMESPACE=moatless
K8S_WORKER_IMAGE=ghcr.io/aorwall/moatless-vibe-worker:latest
K8S_RESOURCE_LIMIT_CPU=2
K8S_RESOURCE_LIMIT_MEMORY=4Gi
K8S_POD_STARTUP_TIMEOUT=120

# Backend - Preview proxy
USE_HOSTNAME_ROUTING=true
PROXY_DOMAIN=proxy.moatless.xyz
PROXY_PROTOCOL=https

# Backend - Database
DATABASE_URL=postgres://...

# Sandbox - Required
TASK_ID=task-abc123
WORKSPACE_DIR=/opt/moatless/repo
PORT=8001
DATA_DIR=/opt/moatless/data

# Sandbox - Git provisioning
GIT_CLONE_URL=https://github.com/...
GIT_BRANCH=main

# Sandbox - Repository configuration (JSON)
REPO_CONFIG='{"previewConfig":{"serverStartCommands":["npm run dev"],"port":3000}}'

# Sandbox - Claude Code
CLAUDE_CODE_OAUTH_TOKEN=...
CLAUDE_CONFIG_DIR=/opt/moatless/.claude

# Sandbox - Redis (event streaming)
REDIS_URL=redis://localhost:6379
```
