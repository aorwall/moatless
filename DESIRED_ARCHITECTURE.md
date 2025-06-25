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
- Workspace operations (files, git, commands, server)
- Event streaming via SSE

**Environment Routes** (`/api/v1/environments/*`)
- Environment configuration management
- Repository and execution settings

### 2. Service Layer

```
┌─────────────────────────────────────────────────────────────┐
│                      Service Layer                          │
├──────────────┬────────────────┬────────────────────────────┤
│ TaskService  │ WorkspaceService│    EventService           │
│              │                 │    (Agent events only)     │
└──────────────┴────────────────┴────────────────────────────┘
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
- Handles message sending and agent coordination
- Uses StorageBackend directly (no wrapper)

**WorkspaceService**
- Single source of truth for workspace operations
- Manages workspace lifecycle and state
- Provides unified `runAgent` method
- Tracks background operations in state

**EventService**
- Handles only agent-generated events
- Provides SSE streaming (no WebSocket)
- Reads Claude Code events from disk when needed

### 3. Workspace Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    Workspace States                         │
├─────────────────────────────────────────────────────────────┤
│  REPOSITORY_MISSING → SETTING_UP → IDLE ⟷ RUNNING          │
│                            ↓                                │
│                          ERROR                              │
└─────────────────────────────────────────────────────────────┘
```

**State Model**
- **REPOSITORY_MISSING**: Workspace repository/worktree doesn't exist
- **SETTING_UP**: Creating worktree or running setup scripts
- **IDLE**: Ready for operations
- **RUNNING**: Agent currently executing
- **ERROR**: Unrecoverable failure

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
WorkspaceService.runAgent()
    │
    ├─→ Check workspace state
    ├─→ Create agent instance
    ├─→ Execute agent
    │
    ▼
Agent.run() [Async Generator]
    │
    ├─→ Emit events
    ├─→ Capture session ID
    └─→ Handle completion/errors
```

**Simplified Flow**
- Only 3 layers: Route → Service → Agent
- Single `runAgent` method replaces multiple duplicate methods
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

### 7. Workspace Operations

All workspace operations go through WorkspaceService and respect state requirements:

**File Operations**
- Available: SETTING_UP (if directory exists), IDLE, RUNNING
- Methods: listFiles, readFile, writeFile, searchFiles

**Git Operations**
- Available: SETTING_UP (if directory exists), IDLE, RUNNING  
- Methods: getGitDiff

**Command Execution**
- Available: IDLE only
- Methods: runCommand

**Server Management**
- Start: IDLE, RUNNING
- Stop/Status/Logs: IDLE, RUNNING
- Methods: startServer, stopServer, getServerStatus, getServerLogs

**Agent Execution**
- Available: IDLE only
- Methods: runAgent

### 8. Event System

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

### 9. Key Design Principles

1. **State-Driven Operations**
   - All operations check workspace state first
   - State derived from runtime conditions
   - Clear rules for operation availability

2. **Single Responsibility**
   - TaskService: Task metadata only
   - WorkspaceService: All workspace operations
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

## Implementation Priority

1. **High Priority**
   - Remove TaskStorageService wrapper
   - Consolidate agent execution methods
   - Implement workspace state model
   - Add missing API endpoints

2. **Medium Priority**
   - Standardize error handling
   - Remove WebSocket support
   - Track background tasks in state

3. **Low Priority**
   - Remove non-agent events
   - Optimize storage patterns

This architecture provides a clean, maintainable system that properly handles the complexity of AI-powered development environments while keeping the codebase simple and understandable.