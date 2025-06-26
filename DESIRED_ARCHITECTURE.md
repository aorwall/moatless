# Moatless Desired Architecture

## Overview

The Moatless system provides an AI-powered development environment where agents can execute tasks within isolated workspaces. The architecture emphasizes simplicity, clear state management, and proper separation of concerns.

## Core Components

### 1. API Layer

```
┌─────────────────────────────────────────────────────────────┐
│                         API Routes                          │
├─────────────────────┬───────────────────────────────────────┤
│   /tasks/*          │        /environments/*              │
│   - CRUD operations │        - List/Get                   │
│   - Send messages   │        - Create/Delete              │
│   - Workspace ops   │        - File search                │
│   - Event streaming │                                     │
└─────────────────────┴───────────────────────────────────────┘
```

**Task Routes** (`/api/v1/tasks/*`)
- Task CRUD operations
- Message sending to trigger agents
- Agent status monitoring (`GET /:taskId/agent-status`)
- Workspace operations (files, git, commands, server)
- Event streaming via SSE

**Environment Routes** (`/api/v1/environments/*`)
- Environment configuration management
- Repository and execution settings

### 2. Service Layer

```
┌─────────────────────────────────────────────────────────────────┐
│                      Service Layer                              │
├──────────────┬────────────────┬────────────────┬───────────────┤
│ TaskService  │ WorkspaceService│  AgentService  │ EventService  │
│              │                 │                │ (Agent events │
│              │                 │                │ only)         │
├──────────────┴────────────────┼────────────────┴───────────────┤
│      PreviewEnvironmentService │                                │
│    (Server lifecycle mgmt)     │                                │
└────────────────────────────────┴────────────────────────────────┘
                        │
                        ▼
┌─────────────────────────────────────────────────────────────────┐
│                   StorageBackend                                │
│              (Direct file system access)                        │
└─────────────────────────────────────────────────────────────────┘
```

**TaskService**
- Manages task metadata and iterations
- Delegates workspace operations to WorkspaceService
- Handles message sending via AgentService
- Uses StorageBackend directly (no wrapper)

**WorkspaceService**
- Single source of truth for workspace operations
- Manages workspace lifecycle and state
- Provides file, git, and command operations
- Delegates server operations to PreviewEnvironmentService
- Tracks background operations in state

**AgentService**
- Manages agent process lifecycle
- Handles agent execution and monitoring
- Coordinates between workspace and task contexts
- Provides unified agent execution interface

**PreviewEnvironmentService**
- Manages preview environment server lifecycle
- Persists server state to disk
- Handles process recovery on startup
- Ensures one server per task
- Provides server health monitoring

**EventService**
- Handles only agent-generated events
- Provides SSE streaming (no WebSocket)
- Reads Claude Code events from disk when needed

### 3. Task Lifecycle

```
┌─────────────────────────────────────────────────────────────┐
│                    Task Lifecycle States                    │
├─────────────────────────────────────────────────────────────┤
│  OPEN → RUNNING → {OPEN|ERROR} → MERGED                    │
│                      ↓                                      │
│                    CLOSED                                   │
└─────────────────────────────────────────────────────────────┘
```

**Task State Model**
- **OPEN**: Task created and ready for agent execution (default state)
- **RUNNING**: Agent actively executing an iteration (set when agent starts)
- **ERROR**: Last iteration failed or workspace setup failed
- **MERGED**: Task changes have been merged to the repository
- **CLOSED**: Task has been closed (workspace torn down)

**State Transitions**
- Task creation → **OPEN**
- Agent starts → **RUNNING**
- Agent completes successfully → **OPEN**
- Agent fails → **ERROR**
- Merge operation → **MERGED**
- Close task → **CLOSED** (unless already **MERGED**)

**State Management**
- TaskManager (Python) sets initial state and handles completion events
- AgentService (TypeScript) manages running state during execution
- Both services coordinate to ensure consistent state
- When task is in RUNNING state, always check AgentService to verify agent is still running

### 4. Workspace Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    Workspace States                         │
├─────────────────────────────────────────────────────────────┤
│  NOT_SETUP → SETTING_UP → IDLE ⟷ RUNNING                   │
└─────────────────────────────────────────────────────────────┘
```

**State Model**
- **NOT_SETUP**: Workspace repository/worktree doesn't exist
- **SETTING_UP**: Creating worktree or running setup scripts
- **IDLE**: Ready for operations
- **RUNNING**: Agent currently executing

States are derived at runtime based on:
- Directory existence
- Setup script status
- Agent execution status

### 5. Agent Execution Flow

```
Client Request
    │
    ▼
TaskService.sendMessage()
    │
    ├─→ Create iteration
    ├─→ Update task metadata
    │
    ▼
AgentService.executeAgent()
    │
    ├─→ Validate workspace state (via WorkspaceService)
    ├─→ Create agent instance with workspace context
    ├─→ Monitor agent execution
    │
    ▼
Agent.run() [Async Generator]
    │
    ├─→ Emit events (handled by AgentService)
    ├─→ Capture session ID
    └─→ Handle completion/errors
```

**AgentService Responsibilities**
- **Process Management**: Create, monitor, and cleanup agent processes
- **State Coordination**: Ensure workspace is ready before agent execution
- **Context Injection**: Provide workspace and task context to agents
- **Error Handling**: Capture and handle agent failures gracefully
- **Event Routing**: Route agent events to EventService for streaming
- **Status Reporting**: Provide real-time agent execution status via API

**Simplified Flow**
- Clear separation: TaskService → AgentService → Agent
- AgentService coordinates with WorkspaceService for state validation
- Direct async iteration over agent events
- Session ID captured from first system event

### 6. Error Handling Strategy

```
┌─────────────────────────────────────────┐
│           Error Types                   │
├─────────────────────────────────────────┤
│  Critical    │  Non-Critical           │
├──────────────┼─────────────────────────┤
│ • Abort flow │ • Log to console        │
│ • Save to    │ • Continue operation    │
│   iteration  │ • No flow interruption  │
│ • Return     │                         │
│   error      │                         │
└──────────────┴─────────────────────────┘
```

**Critical Errors**
- Workspace not ready for operation
- Agent already running
- Environment not found
- Task not found

**Non-Critical Errors**
- Transient network issues
- Non-blocking setup warnings
- Optional feature failures

### 7. Storage Architecture

```
data/
├── environments/
│   └── {envId}.json
├── tasks/
│   └── {taskId}.json
├── events/
│   └── {taskId}/
│       └── events.json
└── workspaces/
    └── {taskId}/
        └── (git worktree)
```

**Direct Storage Access**
- Services use `StorageBackend` directly
- No intermediate wrapper services
- Consistent read/write patterns
- JSON-based persistence

### 8. Agent Service Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                     AgentService                            │
├─────────────────────────────────────────────────────────────┤
│  Agent Process Management                                   │
│  ├─→ createAgent(taskId, environment, message)            │
│  ├─→ executeAgent(agent, workspaceContext)                │
│  ├─→ monitorExecution(agent)                              │
│  └─→ cleanupAgent(agent)                                  │
│                                                             │
│  State Coordination                                         │
│  ├─→ validateWorkspaceReady(workspaceService)             │
│  ├─→ ensureEnvironmentSetup(environment)                  │
│  └─→ handleWorkspaceStateTransitions()                    │
│                                                             │
│  Event Management                                           │
│  ├─→ captureAgentEvents(eventStream)                      │
│  ├─→ routeToEventService(events)                          │
│  └─→ handleSessionTracking(sessionId)                     │
└─────────────────────────────────────────────────────────────┘
```

**AgentService Integration Points**

**With TaskService:**
- Receives agent execution requests with task context
- Updates task iterations with agent results
- Handles task-specific agent configuration

**With WorkspaceService:**
- Validates workspace state before agent execution
- Coordinates workspace setup if needed
- Provides workspace context to agents
- Monitors workspace changes during execution

**With EventService:**
- Routes all agent events for streaming
- Maintains event ordering and session tracking
- Handles event persistence

**Agent Lifecycle Management**
1. **Creation**: Agent instance with environment and task context
2. **Validation**: Ensure workspace is in IDLE state
3. **Execution**: Run agent with proper monitoring
4. **Event Handling**: Capture and route all agent events
5. **Completion**: Handle success/failure and cleanup
6. **Error Recovery**: Manage agent failures and workspace recovery

**Agent Status API Integration**
The AgentService exposes agent status through the task API:

```
GET /api/v1/tasks/:taskId/agent-status
Response: {
  status: 'idle' | 'running' | 'preparing' | 'error',
  sessionId: string | null,
  startedAt: string | null,
  currentIteration: number | null,
  workspaceStatus: WorkspaceStatus,
  canExecute: boolean
}

POST /api/v1/tasks/:taskId/stop
Response: {
  status: 'stopped' | 'not_running',
  taskId: string
}
```

**Status Values:**
- **idle**: Agent available, workspace ready
- **running**: Agent currently executing
- **preparing**: Workspace setting up
- **error**: System in error state

**Benefits of AgentService**
- **Centralized Agent Logic**: Single point for all agent-related operations
- **State Safety**: Prevents agent execution in invalid workspace states
- **Event Consistency**: Ensures all agent events are properly captured
- **Error Isolation**: Agent failures don't affect other system components
- **Resource Management**: Proper cleanup of agent processes and resources
- **Real-time Status**: Provides accurate agent execution status for UI

### 9. Workspace Operations & Capabilities

All workspace operations go through WorkspaceService and use capability checks for validation:

**Capability-Based Operations**

The WorkspaceService provides boolean capability methods that handle both task validation and workspace state checking:

```typescript
// Capability check methods
async canListFiles(taskId: string): Promise<boolean>
async canReadWriteFiles(taskId: string): Promise<boolean> 
async canExecuteCommands(taskId: string): Promise<boolean>
async canManageServer(taskId: string): Promise<boolean>
async canAccessGit(taskId: string): Promise<boolean>
```

**Operation Capabilities Matrix:**

| Operation | NOT_SETUP | SETTING_UP | IDLE | RUNNING | Methods |
|-----------|-----------|------------|------|---------|---------|
| **List Files** (`canListFiles`) | ❌ | ✅ | ✅ | ✅ | `listFiles`, `searchFiles` |
| **Read/Write Files** (`canReadWriteFiles`) | ❌ | ✅ | ✅ | ✅ | `readFile`, `writeFile` |
| **Execute Commands** (`canExecuteCommands`) | ❌ | ❌ | ✅ | ❌ | `runCommand` |
| **Manage Server** (`canManageServer`) | ❌ | ❌ | ✅ | ✅ | `startServer`, `stopServer`, `getServerStatus`, `getServerLogs` |
| **Access Git** (`canAccessGit`) | ❌ | ✅ | ✅ | ✅ | `getGitDiff` |
| **Agent Execution** | ❌ | ❌ | ✅ | ❌ | Coordinated through AgentService |

**State Definitions:**
- **NOT_SETUP**: Workspace repository/worktree doesn't exist
- **SETTING_UP**: Creating worktree or running setup scripts  
- **IDLE**: Ready for operations, no agent running
- **RUNNING**: Agent currently executing

**Operation Rules:**
- **File Operations**: Available once workspace directory exists (SETTING_UP+)
- **Git Operations**: Available once workspace directory exists (SETTING_UP+)
- **Command Execution**: Only when workspace is ready and no agent running (IDLE only)
- **Server Management**: Available when workspace is operational (IDLE, RUNNING)
- **Agent Execution**: Only when workspace is ready and available (IDLE only)

**Centralized Task Validation**

All workspace operations now perform task validation within the WorkspaceService:
- Task existence checked in capability methods
- Throws `TaskNotFoundError` if task doesn't exist
- API routes simplified - no duplicate task validation
- Single source of truth for workspace access control

### 10. Preview Environment Service Architecture

```
┌─────────────────────────────────────────────────────────────┐
│               PreviewEnvironmentService                     │
├─────────────────────────────────────────────────────────────┤
│  Server Lifecycle Management                                │
│  ├─→ startServer(taskId)                                  │
│  ├─→ stopServer(taskId)                                   │
│  ├─→ getServerStatus(taskId)                              │
│  └─→ getServerLogs(taskId, lines)                         │
│                                                             │
│  Process Recovery & Validation                              │
│  ├─→ loadAndValidateExistingPreviewEnvironments()         │
│  ├─→ validateProcess(pid, command)                        │
│  ├─→ pingServer(port, timeout)                            │
│  └─→ killOrphanedProcess(pid, force)                      │
│                                                             │
│  State Persistence                                          │
│  ├─→ savePreviewEnvironmentMetadata(taskId, metadata)     │
│  ├─→ loadPreviewEnvironmentMetadata(taskId)               │
│  └─→ deletePreviewEnvironmentMetadata(taskId)             │
└─────────────────────────────────────────────────────────────┘
```

**PreviewEnvironmentService Integration Points**

**With WorkspaceService:**
- WorkspaceService delegates all server operations to PreviewEnvironmentService
- PreviewEnvironmentService accesses workspace configuration via WorkspaceService
- Maintains clear separation of concerns

**With StorageBackend:**
- Persists server metadata to `data/preview-environments/{taskId}.json`
- Validates task existence before operations
- Ensures data consistency

**Preview Environment Metadata Structure**
```typescript
interface PreviewEnvironmentMetadata {
  taskId: string
  pid: number
  port: number
  startTime: Date
  command: string
  workspacePath: string
  logFile: string
  portPlaceholder?: string
  processStartTime?: number
}
```

**Server Lifecycle Management**
1. **Start**: Create ServerHandler, start process, persist metadata
2. **Status**: Check handler or recover from metadata, validate process, ping server
3. **Stop**: Stop handler, kill process, clean metadata
4. **Recovery**: Load metadata on startup, validate processes, restore handlers

**Process Validation Strategy**
- Check if PID exists using platform-specific commands
- Verify process command matches expected command
- Ping server port to ensure responsiveness
- Clean up orphaned processes and metadata

**Benefits of PreviewEnvironmentService**
- **Persistence**: Server state survives application restarts
- **Single Server**: Enforces one server per task/workspace
- **Process Safety**: Validates PIDs before operations
- **Recovery**: Automatically recovers running servers on startup
- **Health Monitoring**: Ping validation ensures server responsiveness
- **Clean Architecture**: Follows AgentService pattern

### 11. Event System

```
Agent Events Only
    │
    ▼
EventService
    │
    ├─→ Store to disk
    ├─→ Emit to SSE subscribers
    │
    ▼
SSE Stream → Client
```

**Event Sources**
- Agent messages (from Claude Code SDK)
- Agent tool usage
- Agent completion/errors

**Removed Events**
- Workspace setup events
- Task operation events
- System status events

### 12. Key Design Principles

1. **State-Driven Operations**
   - All operations check workspace state first
   - State derived from runtime conditions
   - Clear rules for operation availability

2. **Single Responsibility**
   - TaskService: Task metadata and coordination
   - WorkspaceService: Workspace operations and state
   - AgentService: Agent process management
   - PreviewEnvironmentService: Preview environment server lifecycle management
   - EventService: Agent events only

3. **Direct Communication**
   - Minimal layers between components
   - No unnecessary wrappers or proxies
   - Clear, direct method calls

4. **Error Clarity**
   - Critical vs non-critical distinction
   - Errors saved to appropriate locations
   - Clear error messages for debugging

5. **Async by Design**
   - Workspace setup runs in background
   - State model handles async correctly
   - No race conditions with proper checks
