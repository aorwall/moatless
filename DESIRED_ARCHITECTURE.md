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
┌─────────────────────────────────────────────────────────────┐
│                      Service Layer                          │
├──────────────┬────────────────┬────────────────┬───────────┤
│ TaskService  │ WorkspaceService│  AgentService  │EventService│
│              │                 │                │ (Agent    │
│              │                 │                │ events    │
│              │                 │                │ only)     │
└──────────────┴────────────────┴────────────────┴───────────┘
                        │
                        ▼
┌─────────────────────────────────────────────────────────────┐
│                   StorageBackend                            │
│              (Direct file system access)                    │
└─────────────────────────────────────────────────────────────┘
```

**TaskService**
- Manages task metadata and iterations
- Delegates workspace operations to WorkspaceService
- Handles message sending via AgentService
- Uses StorageBackend directly (no wrapper)

**WorkspaceService**
- Single source of truth for workspace operations
- Manages workspace lifecycle and state
- Provides file, git, command, and server operations
- Tracks background operations in state

**AgentService**
- Manages agent process lifecycle
- Handles agent execution and monitoring
- Coordinates between workspace and task contexts
- Provides unified agent execution interface

**EventService**
- Handles only agent-generated events
- Provides SSE streaming (no WebSocket)
- Reads Claude Code events from disk when needed

### 3. Workspace Architecture

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

### 4. Agent Execution Flow

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

### 5. Error Handling Strategy

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

### 6. Storage Architecture

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

### 7. Agent Service Architecture

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

### 8. Workspace Operations & Capabilities

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

### 9. Event System

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

### 10. Key Design Principles

1. **State-Driven Operations**
   - All operations check workspace state first
   - State derived from runtime conditions
   - Clear rules for operation availability

2. **Single Responsibility**
   - TaskService: Task metadata and coordination
   - WorkspaceService: Workspace operations and state
   - AgentService: Agent process management
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

## Benefits of This Architecture

1. **Simplicity**: Reduced from 5+ layers to 3 clear layers
2. **Maintainability**: Clear separation of concerns
3. **Reliability**: Proper state management prevents race conditions
4. **Performance**: Direct storage access, minimal overhead
5. **Debuggability**: Clear flow, comprehensive error handling
6. **Flexibility**: Easy to add new workspace operations

## Current Implementation Status

### ✅ **Fully Implemented**

1. **AgentService Architecture**
   - ✅ Centralized agent process lifecycle management
   - ✅ Real-time agent status tracking with session management
   - ✅ Proper separation from WorkspaceService
   - ✅ Event routing and error handling

2. **API Layer Complete**
   - ✅ Agent status endpoint (`GET /:taskId/agent-status`)
   - ✅ Agent stop endpoint (`POST /:taskId/stop`)
   - ✅ All task and environment routes functional
   - ✅ SSE event streaming for real-time updates
   - ✅ Simplified API routes with centralized validation

3. **Service Layer Refactoring**
   - ✅ TaskService delegates agent operations to AgentService
   - ✅ WorkspaceService handles workspace operations only
   - ✅ Direct StorageBackend access (no wrapper services)
   - ✅ Proper dependency injection
   - ✅ Capability-based workspace operations

4. **Core Functionality**
   - ✅ Agent execution with session tracking
   - ✅ Workspace state management with capability checks
   - ✅ Background task tracking
   - ✅ Error isolation and handling
   - ✅ Centralized task validation in WorkspaceService

### 🔄 **Areas for Future Enhancement**

1. **State Model Improvements**
   - Runtime state derivation optimization
   - Enhanced workspace state transitions
   - Better error state handling

2. **Event System Optimization**
   - Remove non-agent events (currently mixed)
   - Optimize event storage patterns
   - Enhanced SSE connection management

3. **Performance & Monitoring**
   - Agent execution metrics
   - Workspace operation performance tracking
   - Better resource cleanup strategies

### 🎯 **Architecture Benefits Realized**

- **Simplicity**: Clean 3-layer architecture implemented
- **Maintainability**: Clear service boundaries and responsibilities
- **Reliability**: Proper state management prevents race conditions
- **Performance**: Direct storage access with minimal overhead
- **Debuggability**: Clear execution flow and comprehensive error handling
- **Scalability**: Easy to extend with new workspace operations and agent types

This architecture successfully provides a clean, maintainable system that properly handles the complexity of AI-powered development environments while keeping the codebase simple and understandable.