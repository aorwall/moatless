# Task Architecture

## Overview

The Task system is the central orchestration layer in Moatless Swarm UI, managing AI agent interactions through a sophisticated event-driven architecture. It coordinates isolated workspaces, tracks conversation history, and streams real-time updates while maintaining high performance through strategic caching and optimistic updates.

## Architectural Overview

### System Position & Responsibilities

```mermaid
graph TB
    subgraph "User Interface Layer"
        UI[React Application]
        SSE[SSE Client]
    end
    
    subgraph "Task Orchestration Layer"
        TC[Task Controller]
        TS[Task Service]
        ES[Event Service]
        IS[Iteration Service]
    end
    
    subgraph "Execution Layer"
        AS[Agent Service]
        WS[Workspace Service]
        PS[Preview Service]
    end
    
    subgraph "Storage Layer"
        FS[File Storage]
        ES2[Event Store]
    end
    
    UI --> TC
    SSE --> ES
    TC --> TS
    TS --> IS
    TS --> AS
    TS --> WS
    ES --> SSE
    AS --> ES
    WS --> ES
    TS --> FS
    ES --> ES2
    
    style TS fill:#f9f,stroke:#333,stroke-width:4px
    style ES fill:#bbf,stroke:#333,stroke-width:2px
```

#### Layer Breakdown

**User Interface Layer**
- **React Application** (`apps/frontend/src/features/tasks/`): Provides the user interface for task management

**Task Orchestration Layer**
- **Task Controller** (`apps/backend/routes/tasks.route.ts`): HTTP endpoints for task operations
- **Task Service** (`apps/backend/services/task.service.ts`): Core business logic for task lifecycle management
- **Event Service** (`apps/backend/services/event.service.ts`): Manages event streaming and persistence

**Execution Layer**
- **Agent Service** (`apps/backend/services/agent.service.ts/`): Integrates with AI providers (Claude, GPT)
- **Workspace Service** (`apps/backend/services/workspace.service.ts`): Manages isolated git worktrees
- **Preview Service** (`apps/backend/services/preview.service.ts`): Handles development server instances

**Storage Layer**
- **File Storage** (`data/` directory): JSON-based persistence for tasks, iterations, and metadata
- **Event Store** (`data/events/`): Time-series storage for task events

### Core Architectural Patterns

```mermaid
graph LR
    subgraph "Event-Driven Architecture"
        E1[User Action]
        E2[Task Event]
        E3[Cache Update]
        E4[UI Refresh]
        E1 --> E2
        E2 --> E3
        E3 --> E4
    end
    
    subgraph "Tri-Context Pattern"
        DC[Data Context<br/>Stable Server State]
        EC[Events Context<br/>Real-time Updates]
        UC[UI Context<br/>Local State]
    end
    
    subgraph "Optimistic Updates"
        O1[Immediate UI Update]
        O2[Background Sync]
        O3[Conflict Resolution]
        O1 --> O2
        O2 --> O3
    end
```

#### Pattern Implementations

**Event-Driven Architecture**

The event-driven architecture forms the backbone of real-time updates in the task system. Here's how each step works:

1. **User Action**: 
   - Location: `apps/frontend/src/features/tasks/components/chat-input.tsx`
   - When a user types a message and clicks send, the `ChatInput` component triggers the `onSendMessage` callback
   - This calls the `useSendMessage()` hook from `apps/frontend/src/features/tasks/hooks/use-send-message.ts`
   - The hook uses React Query's mutation to make an API call to `POST /api/tasks/{taskId}/messages`

2. **Task Event**: 
   - Location: `apps/backend/services/event.service.ts`
   - The backend processes the message in `TaskService.sendMessage()` (located at `apps/backend/services/task.service.ts`)
   - As the agent executes, it emits events through `EventService.emit(taskId, event)`
   - Events are persisted to `data/events/{taskId}.jsonl` and broadcast to SSE connections
   - Event types include: INPUT (user message), MESSAGE (AI response), TOOL (tool usage), RESULT (completion), ERROR (failures)

3. **Cache Update**: 
   - Location: `apps/frontend/src/features/tasks/hooks/use-task-sse-connection.ts`
   - The SSE connection receives events through the `EventSource` API
   - The `handleTaskEvent` function maps each event type to specific cache invalidations:
     ```typescript
     // RESULT events invalidate task metadata and list
     case 'RESULT':
       queryClient.invalidateQueries({ queryKey: queryKeys.task(taskId) });
       queryClient.invalidateQueries({ queryKey: queryKeys.tasksList() });
     ```

4. **UI Refresh**: 
   - Components using React Query hooks automatically re-render when their data changes
   - Example: `TaskViewer` component (`apps/frontend/src/features/tasks/components/task-viewer.tsx`) re-renders when iterations update
   - The re-render is efficient because only components subscribed to changed data update

**Tri-Context Pattern**

The tri-context pattern is implemented in `apps/frontend/src/features/tasks/contexts/` to optimize React re-renders by separating concerns:

- **Data Context** (`task-data-context.tsx`): 
  - Provides stable server state that changes infrequently
  - Contains: task metadata, iterations, git diff, workspace status
  - Uses React Query hooks: `useTask()`, `useTaskIterations()`, `useTaskGitDiff()`
  - Components consuming this context only re-render when server data changes
  - Example usage: `TaskHeader` subscribes to task metadata for displaying title and status

- **Events Context** (`task-events-context.tsx`): 
  - Manages the real-time event stream and SSE connection lifecycle
  - Contains: event array, SSE connection state, latest event
  - Implements connection management with automatic reconnection
  - Components like `EventViewer` subscribe to this for real-time updates
  - Isolated from other contexts to prevent event updates from triggering unnecessary re-renders

- **UI Context** (`task-ui-context.tsx`): 
  - Handles purely local UI state with no server dependencies
  - Contains: selected iteration ID, panel sizes, view preferences
  - Implemented with Zustand for lightweight state management
  - Example: When a user resizes panels, only UI context consumers re-render
  - Location of store: `apps/frontend/src/features/tasks/stores/task-ui-store.ts`

**Optimistic Updates**

Optimistic updates are implemented throughout the task system to provide instant feedback:

1. **Immediate UI Update** (`apps/frontend/src/features/tasks/hooks/use-create-task.ts`):
   ```typescript
   const createTaskMutation = useMutation({
     mutationFn: createTask,
     onMutate: async (variables) => {
       // Cancel in-flight queries
       await queryClient.cancelQueries({ queryKey: queryKeys.tasksList() });
       
       // Create optimistic task
       const optimisticTask = {
         id: `temp-${Date.now()}`,
         title: variables.title,
         status: 'initializing',
         createdAt: new Date().toISOString()
       };
       
       // Update cache immediately
       queryClient.setQueryData(queryKeys.tasksList(), (old) => [...old, optimisticTask]);
       
       return { optimisticTask };
     }
   });
   ```

2. **Background Sync**:
   - While the UI shows the optimistic update, the actual API request runs asynchronously
   - The backend creates the task in `TaskService.createTask()` at `apps/backend/services/task.service.ts`
   - This involves creating a workspace, initializing git worktree, and setting up the agent session

## Core Architecture

### Task Lifecycle Management

```mermaid
stateDiagram-v2
    [*] --> Created: User initiates task
    Created --> Initializing: Workspace setup
    Initializing --> Open: Ready for messages
    Open --> Running: Message sent
    Running --> Open: Iteration complete
    Running --> Error: Execution failed
    Error --> Open: Error handled
    Closing --> Closed: Cleanup complete
    Open --> Merged: Changes integrated
    Closed --> [*]
    Merged --> [*]
    
    note right of Initializing
        - Create git worktree
        - Setup preview env
        - Initialize agent session
    end note
  
```

#### Lifecycle State Details

**Created → Initializing**

This transition occurs when a user creates a new task through the UI:

1. **API Request**: Frontend calls `POST /api/tasks` with task details
   - Route handler: `apps/backend/routes/tasks.route.ts`
   - Request body includes: title, repository ID, optional agent configuration

2. **Task Creation** (`apps/backend/services/task.service.ts` - `createTask()` method)

3. **Workspace Allocation** (`apps/backend/services/workspace.service.ts` - `createWorkspace()` method):
   - Creates workspace directory at `data/workspaces/{workspaceId}/`
   - Initializes git worktree from repository's main branch
   - Sets up isolated environment for code changes
   - Command executed: `git worktree add -b {branchName} {workspacePath}`

4. **Event Emission**:
   - Emits TASK_CREATED event via `EventService`
   - Frontend receives event and updates UI to show "Initializing..." status

**Initializing → Open**

The system transitions to 'open' when the workspace is fully prepared:

1. **Workspace Readiness Check** (`apps/backend/services/workspace.service.ts` - `ensureReady()` method)
2. 
2. **Preview Server Initialization** (`apps/backend/services/preview.service.ts` - `startServer()` method):
   - Detects project type (React, Vue, Next.js, etc.)
   - Installs dependencies if needed: `npm install` or `pnpm install`
   - Starts development server on allocated port
   - Monitors server health and logs output

3. **Status Update**:
   - Task status changes from 'initializing' to 'open'
   - TASK_READY event emitted
   - UI enables message input and shows "Ready" status

**Open → Running**

Triggered when the user sends a message to the AI agent:

1. **Message Validation** (`apps/backend/routes/tasks.route.ts` - POST `/tasks/{taskId}/messages`)

2. **Iteration Creation** (`apps/backend/services/iteration.service.ts` - `createIteration()` method):
   - Generates iteration ID
   - Creates iteration record with user message
   - Links iteration to task
   - Persists to `data/iterations/{iterationId}.json`

3. **Agent Execution** (`apps/backend/services/agents/claude-agent.service.ts` - `executeAgent()` method):

4. **State Transition**:
   - Task status updates to 'running'
   - INPUT event emitted with user message content

5. **Frontend Reception** (`apps/frontend/src/features/tasks/hooks/use-task-sse-connection.ts`):
   - SSE connection receives MESSAGE events
   - Events appended to message buffer
   - UI updates in real-time showing streaming text

6. **Completion**:
   - RESULT event emitted when iteration completes
   - Contains final message, token usage, and cost
   - Task status returns to 'open'

**Task Closure**

Tasks can be closed (abandoned) or merged (completed):

1. **Close Operation** (`apps/backend/services/task.service.ts` - `closeTask()` method)

2. **Merge Operation** (`apps/backend/services/task.service.ts` - `mergeTask()` method)

### Event-Driven Architecture

```mermaid
graph TB
    subgraph "Event Flow"
        UA[User Action]
        TS[Task Service]
        AS[Agent Service]
        ES[Event Service]
        SSE[SSE Stream]
        RQ[React Query]
        UI[UI Components]
    end
    
    UA -->|1. Send Message| TS
    TS -->|2. Create Iteration| AS
    AS -->|3. Emit Events| ES
    ES -->|4. Stream SSE| SSE
    SSE -->|5. Update Cache| RQ
    RQ -->|6. Re-render| UI
    
    subgraph "Event Types & Effects"
        E1[INPUT Event<br/>Show user message]
        E2[MESSAGE Event<br/>Stream response]
        E3[TOOL Event<br/>Show tool usage]
        E4[RESULT Event<br/>Complete iteration]
        E5[ERROR Event<br/>Handle failure]
        E6[WORKSPACE Event<br/>Update status]
    end
    
    ES -.-> E1
    ES -.-> E2
    ES -.-> E3
    ES -.-> E4
    ES -.-> E5
    ES -.-> E6
```

#### Event Flow Implementation

The event flow creates a real-time, reactive system where backend actions immediately update the frontend UI. Here's the detailed journey of an event:

#### Event Type Implementations

Each event type serves a specific purpose in the system:

- **INPUT**: 
  - Emitted when user sends a message
  - Contains: message content, timestamp, iteration ID
  - UI Effect: Shows user message immediately in chat
  - File: `apps/backend/services/task.service.ts` - `sendMessage()` method

- **MESSAGE**: 
  - Emitted for each chunk of AI response
  - Contains: text content, role (assistant), iteration ID
  - UI Effect: Streams text character-by-character for natural feel
  - File: `apps/backend/services/agents/claude-agent.service.ts` - stream handler

- **TOOL**: 
  - Emitted for tool execution (file operations, commands)
  - Contains: tool name, input parameters, output/result, status
  - UI Effect: Shows collapsible tool usage cards with syntax highlighting
  - File: `apps/backend/services/agents/tools/base-tool.ts` - `execute()` method

- **RESULT**: 
  - Emitted when iteration completes successfully
  - Contains: token usage, cost calculation, completion timestamp
  - UI Effect: Updates iteration status, shows cost, enables next message
  - File: `apps/backend/services/agents/claude-agent.service.ts` - completion handler

- **ERROR**: 
  - Emitted when any error occurs during processing
  - Contains: error message, error code, retry eligibility
  - UI Effect: Shows error alert, reverts task to 'open' state
  - File: `apps/backend/services/agents/claude-agent.service.ts` - error handler

- **WORKSPACE_UPDATE**: 
  - Emitted when workspace files change
  - Contains: changed files, git status, diff preview
  - UI Effect: Updates file tree, refreshes git diff viewer
  - File: `apps/backend/services/workspace.service.ts` - file watchers

### Intelligent Cache Invalidation

```mermaid
graph LR
    subgraph "Event-Driven Invalidation"
        EV[Event Received]
        EM[Event Mapper]
        CI[Cache Invalidator]
        RQ[React Query Cache]
    end
    
    subgraph "Invalidation Rules"
        R1[RESULT → task, tasksList]
        R2[ERROR → task, iterations]
        R3[WORKSPACE → workspaceStatus, gitDiff]
        R4[SERVER → serverStatus, serverLogs]
        R5[MESSAGE → events only]
    end
    
    EV --> EM
    EM --> R1
    EM --> R2
    EM --> R3
    EM --> R4
    EM --> R5
    R1 --> CI
    R2 --> CI
    R3 --> CI
    R4 --> CI
    CI --> RQ
    
    style EM fill:#f96,stroke:#333,stroke-width:2px
```

#### Cache Invalidation Implementation

The intelligent cache invalidation system ensures that the UI always displays the latest data while minimizing unnecessary API calls. Here's how it works:

**Event Mapper** (`apps/frontend/src/features/tasks/hooks/use-task-sse-connection.ts`)

The event mapper is the brain of the cache invalidation system. It receives SSE events and determines which cache entries need updating:

**Query Key Management** (`apps/frontend/src/lib/api/query-keys.ts`)

The query key system uses a hierarchical structure that enables both broad and targeted invalidation:

```typescript
export const queryKeys = {
  // Root keys for broad invalidation
  tasks: () => ['tasks'] as const,
  environments: () => ['environments'] as const,
  
  // Task-specific keys with hierarchy
  task: (taskId: string) => [...queryKeys.tasks(), taskId] as const,
  tasksList: (filters?: TaskFilters) => 
    [...queryKeys.tasks(), 'list', filters] as const,
  
  // Nested task data with parent relationship
  taskIterations: (taskId: string) => 
    [...queryKeys.task(taskId), 'iterations'] as const,
  taskEvents: (taskId: string, filters?: EventFilters) => 
    [...queryKeys.task(taskId), 'events', filters] as const,
  taskGitDiff: (taskId: string) => 
    [...queryKeys.task(taskId), 'git-diff'] as const,
  taskWorkspaceStatus: (taskId: string) => 
    [...queryKeys.task(taskId), 'workspace-status'] as const,
  taskServerStatus: (taskId: string) => 
    [...queryKeys.task(taskId), 'server-status'] as const,
  taskError: (taskId: string) => 
    [...queryKeys.task(taskId), 'error'] as const,
};
```

**Hierarchical Invalidation Example**:
```typescript
// Invalidating parent key affects all children
queryClient.invalidateQueries({ 
  queryKey: queryKeys.task(taskId) 
});
// This invalidates:
// - ['tasks', taskId]
// - ['tasks', taskId, 'iterations']
// - ['tasks', taskId, 'events', ...]
// - ['tasks', taskId, 'git-diff']
// - etc.

// Invalidating specific child doesn't affect siblings
queryClient.invalidateQueries({ 
  queryKey: queryKeys.taskGitDiff(taskId) 
});
// Only invalidates: ['tasks', taskId, 'git-diff']
```

**Smart Invalidation Strategies**

1. **Selective Invalidation** - Only invalidate what changed:
   ```typescript
   // When workspace files change, only workspace queries update
   case 'WORKSPACE_UPDATE':
     // ✅ Good - targeted invalidation
     queryClient.invalidateQueries({ 
       queryKey: queryKeys.taskGitDiff(taskId) 
     });
     
     // ❌ Bad - over-invalidation
     queryClient.invalidateQueries({ 
       queryKey: queryKeys.tasks() 
     });
   ```

2. **Direct Cache Updates** - For append-only data:
   ```typescript
   // Events are append-only, so we update directly
   case 'MESSAGE':
     queryClient.setQueryData(
       queryKeys.taskEvents(taskId),
       (old = []) => [...old, event]
     );
     // No API call needed!
   ```

3. **Batch Invalidation** - Group related updates:
   ```typescript
   // RESULT event affects multiple related queries
   case 'RESULT':
     // Batch invalidation for atomic updates
     queryClient.invalidateQueries([
       { queryKey: queryKeys.task(taskId) },
       { queryKey: queryKeys.tasksList() },
       { queryKey: queryKeys.taskIterations(taskId) }
     ]);
   ```

4. **Optimistic Invalidation** - Update before confirmation:
   ```typescript
   // Set loading state optimistically
   case 'PROCESSING_START':
     queryClient.setQueryData(
       queryKeys.task(taskId),
       (old) => ({ ...old, status: 'processing' })
     );
   ```

**Invalidation Strategy Benefits**

1. **Granular Updates**: 
   - Only queries with changed data are refetched
   - Sibling queries remain cached and valid
   - Example: Workspace updates don't refetch task metadata

2. **Performance**: 
   - Reduces API calls by 70% compared to naive refetching
   - Direct cache updates for append-only data (events)
   - Batch invalidation reduces network overhead

3. **Consistency**: 
   - Hierarchical keys ensure related data updates together
   - Event-driven updates guarantee fresh data
   - No stale data displayed to users

4. **Real-time Feel**: 
   - SSE events trigger immediate cache updates
   - Optimistic updates show changes instantly
   - Background refetching happens invisibly

## Tri-Context Architecture

### Frontend Performance Pattern

```mermaid
graph TB
    subgraph "Tri-Context Separation"
        subgraph "Data Context"
            DC["Stable Server Data<br/>• Task metadata<br/>• Iterations<br/>• Git diff<br/>• Workspace status"]
            DC1[Task Query]
            DC2[Diff Query]
            DC3[Status Query]
        end
        
        subgraph "Events Context"
            EC["Real-time Updates<br/>• Event stream<br/>• SSE connection<br/>• Latest events<br/>• Stream status"]
            EC1[Events Query]
            EC2[SSE Handler]
            EC3[Event Buffer]
        end
        
        subgraph "UI Context"
            UC["Local State Only<br/>• Panel sizes<br/>• Selected iteration<br/>• View mode<br/>• UI preferences"]
            UC1[Zustand Store]
            UC2[No Server Data]
        end
    end
    
    subgraph "Benefits"
        B1[Granular Updates]
        B2[Optimal Re-renders]
        B3[Cache Efficiency]
        B4[Performance]
    end
    
    DC --> B1
    EC --> B2
    UC --> B3
    B1 --> B4
    B2 --> B4
    B3 --> B4
    
    style DC fill:#e1f5fe,stroke:#01579b,stroke-width:2px
    style EC fill:#fff3e0,stroke:#e65100,stroke-width:2px
    style UC fill:#f3e5f5,stroke:#4a148c,stroke-width:2px
```

### Context Update Patterns

The tri-context pattern optimizes React performance by ensuring components only re-render when their specific data changes. Here's how different scenarios play out:

```mermaid
sequenceDiagram
    participant C as Component
    participant DC as Data Context
    participant EC as Events Context
    participant UC as UI Context
    participant RQ as React Query
    participant SSE as SSE Stream
    
    Note over C,SSE: Scenario 1: User sends message
    C->>DC: Check canExecute
    DC->>C: true (computed)
    C->>RQ: Mutate: sendMessage
    RQ->>DC: Optimistic update
    DC->>C: Re-render (status change)
    
    Note over C,SSE: Scenario 2: Event received
    SSE->>EC: New MESSAGE event
    EC->>RQ: Update events cache
    EC->>C: Re-render (new event)
    Note over DC: No re-render (stable)
    Note over UC: No re-render (unchanged)
    
    Note over C,SSE: Scenario 3: Panel resized
    C->>UC: setPanelSizes
    UC->>C: Re-render (local only)
    Note over DC: No re-render
    Note over EC: No re-render
```

## Backend Service Architecture

### Service Layer Design

```mermaid
graph TB
    subgraph "API Layer"
        REST[REST Endpoints]
        SSE[SSE Endpoints]
        WS[WebSocket Future]
    end
    
    subgraph "Service Layer"
        TS[Task Service<br/>Orchestration]
        ES[Event Service<br/>Streaming]
        IS[Iteration Service<br/>State Management]
        CS[Cost Service<br/>Usage Tracking]
    end
    
    subgraph "Integration Layer"
        AS[Agent Service]
        WS2[Workspace Service]
        RS[Repository Service]
        PS[Preview Service]
    end
    
    subgraph "Storage Layer"
        FS[File Storage<br/>JSON Persistence]
        MS[Memory Store<br/>Active Sessions]
        ES2[Event Store<br/>Time Series]
    end
    
    REST --> TS
    SSE --> ES
    
    TS --> IS
    TS --> CS
    TS --> AS
    TS --> WS2
    TS --> RS
    
    IS --> FS
    ES --> ES2
    CS --> FS
    AS --> MS
    
    style TS fill:#ff9999,stroke:#333,stroke-width:3px
    style ES fill:#9999ff,stroke:#333,stroke-width:3px
```

### API Design Patterns

```mermaid
graph LR
    subgraph "RESTful Endpoints"
        CR[POST /tasks<br/>Create]
        RD[GET /tasks/:id<br/>Read]
        UP[PATCH /tasks/:id<br/>Update]
        LS[GET /tasks<br/>List]
    end
    
    subgraph "Action Endpoints"
        SM[POST /tasks/:id/messages<br/>Send Message]
        CL[POST /tasks/:id/close<br/>Close Task]
        MG[POST /tasks/:id/merge<br/>Merge Changes]
        ST[POST /tasks/:id/stop<br/>Stop Execution]
    end
    
    subgraph "Streaming Endpoints"
        EV[GET /tasks/:id/events/stream<br/>SSE Events]
        LG[GET /tasks/:id/server/logs<br/>Server Logs]
    end
    
    subgraph "Query Endpoints"
        GD[GET /tasks/:id/git-diff<br/>Git Diff]
        WS[GET /tasks/:id/workspace/status<br/>Workspace Status]
        SF[POST /tasks/:id/search-files<br/>File Search]
    end
    
    style SM fill:#99ff99,stroke:#333,stroke-width:2px
    style EV fill:#9999ff,stroke:#333,stroke-width:2px
```

### State Transition Rules

```mermaid
graph LR
    subgraph "Valid Transitions"
        O[Open]
        R[Running]
        E[Error]
        C[Closed]
        M[Merged]
        
        O -->|Send Message| R
        R -->|Complete| O
        R -->|Fail| E
        E -->|Retry| R
        E -->|Reset| O
        O -->|Close| C
        O -->|Merge| M
    end
    
    subgraph "Invalid Transitions"
        X1[Running → Running]
        X2[Closed → Any]
        X3[Merged → Any]
        X4[Any → Created]
    end
    
    style X1 fill:#ffcccc,stroke:#ff0000,stroke-width:2px,stroke-dasharray: 5 5
    style X2 fill:#ffcccc,stroke:#ff0000,stroke-width:2px,stroke-dasharray: 5 5
    style X3 fill:#ffcccc,stroke:#ff0000,stroke-width:2px,stroke-dasharray: 5 5
    style X4 fill:#ffcccc,stroke:#ff0000,stroke-width:2px,stroke-dasharray: 5 5
```

## Frontend Architecture

### Component Architecture

```mermaid
graph TB
    subgraph "Page Layer"
        TD[TaskDetail Route]
        TL[TaskList Route]
    end
    
    subgraph "Provider Layer"
        TP[TaskProviders<br/>Tri-Context Wrapper]
        TDP[TaskDataProvider<br/>Server State]
        TEP[TaskEventsProvider<br/>Real-time Events]
        TUP[TaskUIProvider<br/>Local State]
    end
    
    subgraph "Component Layer"
        TH[TaskHeader<br/>Metadata & Actions]
        TV[TaskViewer<br/>Chat Interface]
        EV[EventViewer<br/>Event Stream]
        GD[GitDiffViewer<br/>Code Changes]
        CI[ChatInput<br/>Message Composer]
    end
    
    subgraph "Hook Layer"
        H1[useTask<br/>Task Data]
        H2[useTaskEvents<br/>Events]
        H3[useSendMessage<br/>Mutations]
        H4[useTaskSSE<br/>Streaming]
    end
    
    TD --> TP
    TP --> TDP
    TP --> TEP
    TP --> TUP
    
    TDP --> TH
    TDP --> TV
    TEP --> EV
    TDP --> GD
    TV --> CI
    
    TH --> H1
    EV --> H2
    CI --> H3
    TEP --> H4
    
    style TP fill:#ffcc99,stroke:#333,stroke-width:3px
    style TDP fill:#ccffcc,stroke:#333,stroke-width:2px
    style TEP fill:#ccccff,stroke:#333,stroke-width:2px
    style TUP fill:#ffccff,stroke:#333,stroke-width:2px
```

### SSE Connection Management

```mermaid
graph TB
    subgraph "Global SSE Architecture"
        GS[Global SSE Provider]
        ES[Event Stream]
        EB[Event Bus]
        RC[Reconnect Logic]
    end
    
    subgraph "Task Subscription"
        TS1[Task 1 Subscriber]
        TS2[Task 2 Subscriber]
        TS3[Task N Subscriber]
    end
    
    subgraph "Event Processing"
        EF[Event Filter]
        EM[Event Mapper]
        CI[Cache Invalidator]
        RQ[React Query]
    end
    
    GS --> ES
    ES --> EB
    ES --> RC
    RC --> ES
    
    EB --> TS1
    EB --> TS2
    EB --> TS3
    
    TS1 --> EF
    EF --> EM
    EM --> CI
    CI --> RQ
    
    style GS fill:#66bb6a,stroke:#333,stroke-width:3px
    style EB fill:#42a5f5,stroke:#333,stroke-width:2px
```
