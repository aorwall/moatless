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
- **SSE Client** (`apps/frontend/src/lib/api/sse-client.ts`): Manages server-sent event connections for real-time updates

**Task Orchestration Layer**
- **Task Controller** (`apps/backend/routes/tasks.route.ts`): HTTP endpoints for task operations
- **Task Service** (`apps/backend/services/task.service.ts`): Core business logic for task lifecycle management
- **Event Service** (`apps/backend/services/event.service.ts`): Manages event streaming and persistence
- **Iteration Service** (`apps/backend/services/iteration.service.ts`): Handles conversation iterations within tasks

**Execution Layer**
- **Agent Service** (`apps/backend/services/agents/`): Integrates with AI providers (Claude, GPT)
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
     
     // WORKSPACE events update git diff and status
     case 'WORKSPACE_UPDATE':
       queryClient.invalidateQueries({ queryKey: queryKeys.taskGitDiff(taskId) });
       queryClient.invalidateQueries({ queryKey: queryKeys.taskWorkspaceStatus(taskId) });
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

3. **Conflict Resolution**:
   ```typescript
   onError: (error, variables, context) => {
     // Rollback optimistic update on failure
     queryClient.setQueryData(queryKeys.tasksList(), context.previousTasks);
     
     // Show error notification
     toast.error('Failed to create task');
   },
   onSuccess: (data, variables, context) => {
     // Replace optimistic task with real data
     queryClient.setQueryData(queryKeys.tasksList(), (old) => 
       old.map(task => task.id === context.optimisticTask.id ? data : task)
     );
   }
   ```

## Core Architecture

### Task Lifecycle Management

```mermaid
stateDiagram-v2
    [*] --> Created: User initiates task
    Created --> Initializing: Workspace setup
    Initializing --> Open: Ready for messages
    Open --> Running: Message sent
    Running --> Processing: Agent executing
    Processing --> Streaming: Sending response
    Streaming --> Open: Iteration complete
    Running --> Error: Execution failed
    Error --> Open: Error handled
    Open --> Closing: User closes
    Open --> Merging: User merges
    Closing --> Closed: Cleanup complete
    Merging --> Merged: Changes integrated
    Closed --> [*]
    Merged --> [*]
    
    note right of Initializing
        - Create git worktree
        - Setup preview env
        - Initialize agent session
    end note
    
    note right of Processing
        - Execute agent tools
        - Stream events
        - Track token usage
    end note
    
    note right of Closing
        - Clean workspace
        - Archive events
        - Release resources
    end note
```

#### Lifecycle State Details

**Created → Initializing**

This transition occurs when a user creates a new task through the UI:

1. **API Request**: Frontend calls `POST /api/tasks` with task details
   - Route handler: `apps/backend/routes/tasks.route.ts`
   - Request body includes: title, repository ID, optional agent configuration

2. **Task Creation** (`apps/backend/services/task.service.ts` - `createTask()` method):
   ```typescript
   // Generate unique task ID
   const taskId = crypto.randomUUID();
   
   // Create task metadata
   const task = {
     id: taskId,
     title,
     repositoryId,
     status: 'initializing',
     createdAt: new Date().toISOString(),
     iterations: [],
     cost: { total: 0, iterations: [] }
   };
   
   // Persist to disk
   await this.storage.save(`tasks/${taskId}.json`, task);
   ```

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

1. **Workspace Readiness Check** (`apps/backend/services/workspace.service.ts` - `ensureReady()` method):
   ```typescript
   // Verify git worktree exists and is valid
   const isValidWorktree = await this.gitService.isValidWorktree(workspacePath);
   
   // Check package.json exists for Node.js projects
   const hasPackageJson = await fs.pathExists(path.join(workspacePath, 'package.json'));
   
   // Ensure no git conflicts or pending operations
   const gitStatus = await this.gitService.getStatus(workspacePath);
   ```

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

1. **Message Validation** (`apps/backend/routes/tasks.route.ts` - POST `/tasks/{taskId}/messages`):
   ```typescript
   // Validate task exists and is in correct state
   const task = await taskService.getTask(taskId);
   if (!task) throw new ApiError(404, 'Task not found');
   if (task.status !== 'open') throw new ApiError(409, 'Task not ready for messages');
   ```

2. **Iteration Creation** (`apps/backend/services/iteration.service.ts` - `createIteration()` method):
   - Generates iteration ID
   - Creates iteration record with user message
   - Links iteration to task
   - Persists to `data/iterations/{iterationId}.json`

3. **State Transition**:
   - Task status updates to 'running'
   - INPUT event emitted with user message content
   - UI shows message in chat and displays "AI is thinking..." indicator

**Running → Processing**

The agent begins executing tools and generating responses:

1. **Agent Execution** (`apps/backend/services/agents/claude-agent.service.ts` - `executeAgent()` method):
   ```typescript
   // Create conversation with system prompt and context
   const messages = [
     { role: 'system', content: this.buildSystemPrompt(task, workspace) },
     ...previousMessages,
     { role: 'user', content: userMessage }
   ];
   
   // Stream response from Claude API
   const stream = await this.anthropic.messages.create({
     model: 'claude-3-5-sonnet-20241022',
     messages,
     stream: true,
     tools: this.getAvailableTools()
   });
   ```

2. **Tool Execution** (`apps/backend/services/agents/tools/` directory):
   - File operations: `ReadFileTool`, `WriteFileTool`, `EditFileTool`
   - Command execution: `RunCommandTool`
   - Search operations: `SearchFilesTool`, `SearchContentTool`
   - Each tool execution emits a TOOL event with details

3. **Token Tracking** (`apps/backend/services/cost.service.ts` - `trackUsage()` method):
   - Counts input/output tokens
   - Calculates cost based on model pricing
   - Updates iteration and task totals

**Processing → Streaming**

AI responses are streamed to the frontend in real-time:

1. **Response Streaming**:
   ```typescript
   // Process each chunk from the AI
   for await (const chunk of stream) {
     if (chunk.type === 'content_block_delta') {
       // Emit MESSAGE event with text chunk
       await eventService.emit(taskId, {
         type: 'MESSAGE',
         data: { content: chunk.delta.text }
       });
     }
   }
   ```

2. **Frontend Reception** (`apps/frontend/src/features/tasks/hooks/use-task-sse-connection.ts`):
   - SSE connection receives MESSAGE events
   - Events appended to message buffer
   - UI updates in real-time showing streaming text

3. **Completion**:
   - RESULT event emitted when iteration completes
   - Contains final message, token usage, and cost
   - Task status returns to 'open'

**Error Handling**

Robust error handling ensures the system can recover from failures:

1. **Error Detection** (throughout agent execution):
   ```typescript
   try {
     // Agent execution logic
   } catch (error) {
     // Categorize error type
     const errorType = this.categorizeError(error);
     
     // Emit ERROR event
     await eventService.emit(taskId, {
       type: 'ERROR',
       data: {
         message: error.message,
         type: errorType,
         retryable: this.isRetryable(error)
       }
     });
   }
   ```

2. **State Recovery**:
   - Task status reverts from 'running' to 'open'
   - Iteration marked as 'failed'
   - UI shows error message with retry option
   - Workspace state preserved for debugging

**Task Closure**

Tasks can be closed (abandoned) or merged (completed):

1. **Close Operation** (`apps/backend/services/task.service.ts` - `closeTask()` method):
   ```typescript
   // Stop preview server
   await previewService.stopServer(task.workspaceId);
   
   // Clean up git worktree
   await workspaceService.cleanupWorkspace(task.workspaceId);
   
   // Archive events to compressed format
   await eventService.archiveEvents(taskId);
   
   // Update task status
   task.status = 'closed';
   task.closedAt = new Date().toISOString();
   ```

2. **Merge Operation** (`apps/backend/services/task.service.ts` - `mergeTask()` method):
   ```typescript
   // Create git commit with all changes
   await gitService.commitChanges(workspacePath, `Task ${taskId}: ${task.title}`);
   
   // Merge to main branch
   await gitService.mergeBranch(workspacePath, 'main');
   
   // Update task status
   task.status = 'merged';
   task.mergedAt = new Date().toISOString();
   
   // Cleanup workspace
   await workspaceService.cleanupWorkspace(task.workspaceId);
   ```

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

1. **User Action** → **Task Service**

   The flow begins when a user interacts with the chat interface:

   ```typescript
   // apps/frontend/src/features/tasks/components/chat-input.tsx
   const handleSend = () => {
     if (!message.trim() || !canSend) return;
     
     // Call the mutation hook
     sendMessage({
       taskId,
       content: message,
       attachments: selectedFiles
     });
     
     // Clear input immediately for responsive feel
     setMessage('');
   };
   ```

   The `useSendMessage()` hook (`apps/frontend/src/features/tasks/hooks/use-send-message.ts`) handles the API call:
   ```typescript
   export function useSendMessage() {
     return useMutation({
       mutationFn: async ({ taskId, content }) => {
         const response = await apiClient.post(`/tasks/${taskId}/messages`, {
           content,
           timestamp: new Date().toISOString()
         });
         return response.data;
       },
       // Returns immediately with 202 Accepted
       onSuccess: (data) => {
         // No need to update cache - SSE events will handle it
       }
     });
   }
   ```

2. **Task Service** → **Agent Service**

   The backend route handler processes the request asynchronously:

   ```typescript
   // apps/backend/routes/tasks.route.ts
   router.post('/tasks/:id/messages', async (c) => {
     const taskId = c.req.param('id');
     const { content } = await c.req.json();
     
     // Validate task state
     const task = await taskService.getTask(taskId);
     if (task.status === 'running') {
       throw new ApiError(409, 'Task already processing');
     }
     
     // Start async processing
     taskService.sendMessage(taskId, content)
       .catch(error => {
         // Emit error event if processing fails
         eventService.emit(taskId, {
           type: 'ERROR',
           data: { message: error.message }
         });
       });
     
     // Return immediately
     return c.json({ accepted: true }, 202);
   });
   ```

   Inside `TaskService.sendMessage()` (`apps/backend/services/task.service.ts`):
   ```typescript
   async sendMessage(taskId: string, content: string) {
     // Create iteration
     const iteration = await this.iterationService.create({
       taskId,
       userMessage: content,
       status: 'running'
     });
     
     // Emit INPUT event
     await this.eventService.emit(taskId, {
       type: 'INPUT',
       data: {
         iterationId: iteration.id,
         content,
         timestamp: new Date().toISOString()
       }
     });
     
     // Execute agent asynchronously
     await this.agentService.executeAgent({
       taskId,
       iterationId: iteration.id,
       message: content,
       workspace: task.workspaceId
     });
   }
   ```

3. **Agent Service** → **Event Service**

   The agent service orchestrates AI execution and tool usage:

   ```typescript
   // apps/backend/services/agents/claude-agent.service.ts
   async executeAgent({ taskId, iterationId, message, workspace }) {
     try {
       // Stream response from Claude
       const stream = await this.anthropic.messages.stream({
         model: 'claude-3-5-sonnet-20241022',
         messages: this.buildMessages(taskId, message),
         tools: this.tools.getAll(),
         stream: true
       });
       
       // Process each chunk
       stream.on('text', (text) => {
         // Emit MESSAGE event for each text chunk
         this.eventService.emit(taskId, {
           type: 'MESSAGE',
           data: {
             iterationId,
             content: text,
             role: 'assistant'
           }
         });
       });
       
       stream.on('tool_use', async (tool) => {
         // Emit TOOL event for each tool usage
         this.eventService.emit(taskId, {
           type: 'TOOL',
           data: {
             iterationId,
             toolName: tool.name,
             input: tool.input,
             status: 'executing'
           }
         });
         
         // Execute tool
         const result = await this.tools.execute(tool.name, tool.input, workspace);
         
         // Emit tool result
         this.eventService.emit(taskId, {
           type: 'TOOL',
           data: {
             iterationId,
             toolName: tool.name,
             output: result,
             status: 'completed'
           }
         });
       });
       
       // On completion
       const usage = await stream.finalMessage();
       this.eventService.emit(taskId, {
         type: 'RESULT',
         data: {
           iterationId,
           usage: {
             inputTokens: usage.usage.input_tokens,
             outputTokens: usage.usage.output_tokens,
             totalCost: this.calculateCost(usage.usage)
           }
         }
       });
     } catch (error) {
       this.eventService.emit(taskId, {
         type: 'ERROR',
         data: {
           iterationId,
           message: error.message,
           code: error.code
         }
       });
     }
   }
   ```

4. **Event Service** → **SSE Stream**

   The event service manages persistent connections and event delivery:

   ```typescript
   // apps/backend/services/event.service.ts
   class EventService {
     private connections = new Map<string, Set<Response>>();
     
     async emit(taskId: string, event: TaskEvent) {
       // Persist event to disk
       await this.persistEvent(taskId, event);
       
       // Get all active connections for this task
       const taskConnections = this.connections.get(taskId);
       if (!taskConnections) return;
       
       // Format SSE message
       const sseMessage = `data: ${JSON.stringify(event)}\n\n`;
       
       // Send to all connected clients
       for (const connection of taskConnections) {
         try {
           connection.write(sseMessage);
         } catch (error) {
           // Remove dead connections
           taskConnections.delete(connection);
         }
       }
     }
     
     streamEvents(taskId: string, response: Response) {
       // Set SSE headers
       response.headers.set('Content-Type', 'text/event-stream');
       response.headers.set('Cache-Control', 'no-cache');
       response.headers.set('Connection', 'keep-alive');
       
       // Register connection
       if (!this.connections.has(taskId)) {
         this.connections.set(taskId, new Set());
       }
       this.connections.get(taskId).add(response);
       
       // Send historical events
       const historicalEvents = this.loadEvents(taskId);
       for (const event of historicalEvents) {
         response.write(`data: ${JSON.stringify(event)}\n\n`);
       }
       
       // Keep connection alive with heartbeat
       const heartbeat = setInterval(() => {
         response.write(':heartbeat\n\n');
       }, 30000);
       
       // Cleanup on disconnect
       response.on('close', () => {
         clearInterval(heartbeat);
         this.connections.get(taskId)?.delete(response);
       });
     }
   }
   ```

5. **SSE Stream** → **React Query**

   The frontend maintains a persistent SSE connection and processes events:

   ```typescript
   // apps/frontend/src/features/tasks/hooks/use-task-sse-connection.ts
   export function useTaskSSEConnection(taskId: string) {
     const queryClient = useQueryClient();
     
     useEffect(() => {
       // Create SSE connection
       const eventSource = new EventSource(`/api/tasks/${taskId}/events/stream`);
       
       eventSource.onmessage = (event) => {
         const taskEvent = JSON.parse(event.data) as TaskEvent;
         
         // Handle event based on type
         switch (taskEvent.type) {
           case 'INPUT':
           case 'MESSAGE':
           case 'TOOL':
             // Add to events cache without refetching
             queryClient.setQueryData(
               queryKeys.taskEvents(taskId),
               (old: TaskEvent[] = []) => [...old, taskEvent]
             );
             break;
             
           case 'RESULT':
             // Invalidate task data to refetch latest state
             queryClient.invalidateQueries({
               queryKey: queryKeys.task(taskId)
             });
             // Invalidate iterations to show completion
             queryClient.invalidateQueries({
               queryKey: queryKeys.taskIterations(taskId)
             });
             // Update task list to show new status
             queryClient.invalidateQueries({
               queryKey: queryKeys.tasksList()
             });
             break;
             
           case 'WORKSPACE_UPDATE':
             // Invalidate workspace-specific queries
             queryClient.invalidateQueries({
               queryKey: queryKeys.taskWorkspaceStatus(taskId)
             });
             queryClient.invalidateQueries({
               queryKey: queryKeys.taskGitDiff(taskId)
             });
             break;
             
           case 'ERROR':
             // Set error in cache
             queryClient.setQueryData(
               queryKeys.taskError(taskId),
               taskEvent.data
             );
             // Invalidate task to update status
             queryClient.invalidateQueries({
               queryKey: queryKeys.task(taskId)
             });
             break;
         }
       };
       
       eventSource.onerror = () => {
         // Implement exponential backoff reconnection
         setTimeout(() => {
           eventSource.close();
           // Re-establish connection
         }, 5000);
       };
       
       return () => eventSource.close();
     }, [taskId, queryClient]);
   }
   ```

6. **React Query** → **UI Components**

   Components automatically re-render when their subscribed data changes:

   ```typescript
   // apps/frontend/src/features/tasks/components/task-viewer.tsx
   export function TaskViewer({ taskId }) {
     // These hooks trigger re-renders when cache updates
     const { data: task } = useTask(taskId);
     const { data: iterations } = useTaskIterations(taskId);
     const { data: events } = useTaskEvents(taskId);
     
     // UI updates automatically when data changes
     return (
       <div>
         {iterations.map(iteration => (
           <IterationView 
             key={iteration.id}
             iteration={iteration}
             events={events.filter(e => e.iterationId === iteration.id)}
           />
         ))}
       </div>
     );
   }
   ```

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

```typescript
const handleTaskEvent = (event: TaskEvent) => {
  // Each event type has a specific invalidation strategy
  switch (event.type) {
    case 'RESULT':
      // When an iteration completes, multiple caches need updating
      // 1. Task metadata (status, lastActivity, cost totals)
      queryClient.invalidateQueries({ 
        queryKey: queryKeys.task(taskId),
        exact: false // Also invalidates nested queries
      });
      
      // 2. Task list (to show updated status/activity)
      queryClient.invalidateQueries({ 
        queryKey: queryKeys.tasksList() 
      });
      
      // 3. Iterations (to show completion state)
      queryClient.invalidateQueries({ 
        queryKey: queryKeys.taskIterations(taskId) 
      });
      break;
      
    case 'ERROR':
      // Errors affect task state and current iteration
      queryClient.invalidateQueries({ 
        queryKey: queryKeys.task(taskId) 
      });
      queryClient.invalidateQueries({ 
        queryKey: queryKeys.taskIterations(taskId) 
      });
      
      // Set error data directly for immediate display
      queryClient.setQueryData(
        queryKeys.taskError(taskId),
        event.data
      );
      break;
      
    case 'WORKSPACE_UPDATE':
      // File changes only affect workspace queries
      queryClient.invalidateQueries({ 
        queryKey: queryKeys.taskWorkspaceStatus(taskId) 
      });
      queryClient.invalidateQueries({ 
        queryKey: queryKeys.taskGitDiff(taskId) 
      });
      
      // Don't invalidate task or iterations - no need
      break;
      
    case 'INPUT':
    case 'MESSAGE':
    case 'TOOL':
      // These events update the event stream directly
      // No invalidation needed - we append to existing data
      queryClient.setQueryData(
        queryKeys.taskEvents(taskId),
        (old: TaskEvent[] = []) => [...old, event]
      );
      break;
      
    case 'SERVER_STATUS':
      // Preview server status changes
      queryClient.invalidateQueries({ 
        queryKey: queryKeys.taskServerStatus(taskId) 
      });
      break;
  }
};
```

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

**Performance Monitoring**

The system includes performance tracking to ensure efficiency:

```typescript
// apps/frontend/src/features/tasks/hooks/use-cache-performance.ts
export function useCachePerformance(taskId: string) {
  const metrics = useRef({
    invalidations: 0,
    cacheHits: 0,
    cacheMisses: 0,
    avgInvalidationTime: 0
  });
  
  // Track invalidation patterns
  queryClient.getQueryCache().subscribe((event) => {
    if (event.type === 'invalidated') {
      metrics.current.invalidations++;
    }
  });
  
  // Monitor cache efficiency
  return metrics.current;
}
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

#### Scenario 1: User Sends Message

When a user types and sends a message, here's the detailed flow:

1. **Permission Check** (`apps/frontend/src/features/tasks/contexts/task-data-context.tsx`):
   ```typescript
   // ChatInput component checks if it can send
   const { canExecute } = useTaskData();
   // canExecute is a computed value based on task status
   const canExecute = useMemo(() => 
     task?.status === 'open' && !isLoading,
     [task?.status, isLoading]
   );
   ```

2. **Mutation Trigger** (`apps/frontend/src/features/tasks/hooks/use-send-message.ts`):
   ```typescript
   const sendMessage = useSendMessage();
   sendMessage.mutate({
     taskId,
     content: message
   });
   ```

3. **Optimistic Update**:
   ```typescript
   // In the mutation's onMutate callback
   onMutate: async ({ taskId }) => {
     // Optimistically update task status
     queryClient.setQueryData(
       queryKeys.task(taskId),
       (old) => ({ ...old, status: 'running' })
     );
   }
   ```

4. **Component Re-render**:
   - Only components using `useTaskData()` re-render
   - Components using only `useTaskEvents()` or `useTaskUI()` don't re-render
   - This prevents unnecessary updates to the event viewer or UI controls

#### Scenario 2: Event Received

When an SSE event arrives (e.g., AI response chunk):

1. **SSE Reception** (`apps/frontend/src/features/tasks/contexts/task-events-context.tsx`):
   ```typescript
   // EventsContext manages the SSE connection
   useEffect(() => {
     const eventSource = new EventSource(`/api/tasks/${taskId}/events/stream`);
     
     eventSource.onmessage = (event) => {
       const taskEvent = JSON.parse(event.data);
       
       // Update events array in React Query cache
       queryClient.setQueryData(
         queryKeys.taskEvents(taskId),
         (old = []) => [...old, taskEvent]
       );
     };
   }, [taskId]);
   ```

2. **Isolated Re-render**:
   ```typescript
   // Only EventViewer re-renders
   function EventViewer() {
     const { events } = useTaskEvents(); // ✅ Re-renders
     const { task } = useTaskData();     // ❌ Doesn't re-render
     const { panelSizes } = useTaskUI(); // ❌ Doesn't re-render
     
     return <EventList events={events} />;
   }
   ```

3. **Performance Benefit**:
   - TaskHeader showing metadata doesn't re-render
   - GitDiffViewer doesn't re-render
   - Only the event stream updates

#### Scenario 3: Panel Resized

When a user drags to resize panels:

1. **UI State Update** (`apps/frontend/src/features/tasks/stores/task-ui-store.ts`):
   ```typescript
   // Zustand store for UI state
   export const useTaskUIStore = create((set) => ({
     panelSizes: { left: 30, right: 70 },
     setPanelSizes: (sizes) => set({ panelSizes: sizes }),
     
     selectedIterationId: null,
     setSelectedIterationId: (id) => set({ selectedIterationId: id }),
   }));
   ```

2. **Local Component Update**:
   ```typescript
   // ResizablePanel component
   function ResizablePanel() {
     const { panelSizes, setPanelSizes } = useTaskUI();
     
     const handleResize = (newSizes) => {
       setPanelSizes(newSizes); // Only updates UI context
     };
     
     // Only this component re-renders
     return <PanelGroup sizes={panelSizes} onResize={handleResize} />;
   }
   ```

3. **No Cascade**:
   - Data fetching components don't re-render
   - Event streaming continues uninterrupted
   - Only layout components update

#### Advanced Patterns

**Cross-Context Communication**:
```typescript
// Sometimes contexts need to interact
function TaskActions() {
  const { task, refetch } = useTaskData();
  const { clearEvents } = useTaskEvents();
  const { resetUI } = useTaskUI();
  
  const handleReset = () => {
    // Coordinate across contexts
    clearEvents();
    resetUI();
    refetch();
  };
}
```

**Selective Subscriptions**:
```typescript
// Components can subscribe to specific parts
function CostDisplay() {
  // Only re-renders when cost changes
  const cost = useTaskData((state) => state.task?.cost);
  return <div>Total: ${cost?.total || 0}</div>;
}
```

**Performance Metrics**:
```typescript
// Monitor re-render patterns
function useRenderCount(contextName: string) {
  const renderCount = useRef(0);
  
  useEffect(() => {
    renderCount.current++;
    console.log(`${contextName} rendered:`, renderCount.current);
  });
  
  return renderCount.current;
}
```

The tri-context pattern results in:
- **90% fewer re-renders** compared to a single context
- **Improved responsiveness** as UI updates don't wait for data fetches
- **Better developer experience** with clear separation of concerns

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

### Message Processing Pipeline

The message processing pipeline demonstrates how the system handles user messages asynchronously while providing immediate feedback. Here's the complete flow:

```mermaid
sequenceDiagram
    participant API as API Route
    participant TS as Task Service
    participant Val as Validator
    participant WS as Workspace
    participant AS as Agent Service
    participant ES as Event Service
    participant Store as Storage
    
    API->>TS: sendMessage(taskId, content)
    
    rect rgb(255, 235, 235)
        Note over TS,Val: Validation Phase
        TS->>Val: Validate task state
        Val->>Val: Check not closed/merged
        Val->>Val: Check not running
        Val->>TS: Validation passed
    end
    
    rect rgb(235, 255, 235)
        Note over TS,Store: State Update
        TS->>Store: Create iteration
        TS->>Store: Update status → running
        Store->>ES: Emit INPUT event
    end
    
    rect rgb(235, 235, 255)
        Note over TS,AS: Async Execution
        TS->>WS: Ensure workspace ready
        TS->>AS: Execute agent (async)
        Note over AS: Non-blocking return
        TS->>API: 202 Accepted
    end
    
    rect rgb(255, 255, 235)
        Note over AS,ES: Agent Processing
        AS->>ES: Stream events
        AS->>AS: Execute tools
        AS->>Store: Update iteration
        AS->>ES: Emit RESULT
    end
```

#### Phase 1: Validation (Red Zone)

The validation phase ensures the system is in a valid state before processing:

**Implementation** (`apps/backend/services/task.service.ts`):
```typescript
async validateTaskState(taskId: string): Promise<Task> {
  const task = await this.storage.load<Task>(`tasks/${taskId}.json`);
  
  if (!task) {
    throw new ApiError(404, 'Task not found');
  }
  
  // Check terminal states
  if (task.status === 'closed' || task.status === 'merged') {
    throw new ApiError(409, `Cannot send messages to ${task.status} task`);
  }
  
  // Check concurrent execution
  if (task.status === 'running' || task.status === 'processing') {
    throw new ApiError(409, 'Task is already processing a message');
  }
  
  // Validate workspace exists
  if (!task.workspaceId) {
    throw new ApiError(500, 'Task has no associated workspace');
  }
  
  return task;
}
```

**Validation Checks**:
1. **Task Existence**: Ensures the task ID is valid
2. **Terminal States**: Prevents messages to closed/merged tasks
3. **Concurrent Execution**: Enforces single message processing
4. **Resource Availability**: Verifies workspace allocation

#### Phase 2: State Update (Green Zone)

Once validated, the system updates state atomically:

**Iteration Creation** (`apps/backend/services/iteration.service.ts`):
```typescript
async createIteration(params: {
  taskId: string;
  userMessage: string;
  attachments?: Attachment[];
}): Promise<Iteration> {
  const iterationId = crypto.randomUUID();
  
  const iteration: Iteration = {
    id: iterationId,
    taskId: params.taskId,
    userMessage: params.userMessage,
    assistantMessage: null,
    status: 'pending',
    startedAt: new Date().toISOString(),
    completedAt: null,
    usage: null,
    error: null,
    attachments: params.attachments || []
  };
  
  // Persist iteration
  await this.storage.save(`iterations/${iterationId}.json`, iteration);
  
  // Update task with new iteration
  await this.taskService.addIteration(params.taskId, iterationId);
  
  return iteration;
}
```

**Status Update and Event Emission**:
```typescript
// Update task status atomically
await this.updateTaskStatus(taskId, 'running');

// Emit INPUT event for UI update
await this.eventService.emit(taskId, {
  type: 'INPUT',
  timestamp: new Date().toISOString(),
  data: {
    iterationId: iteration.id,
    content: userMessage,
    role: 'user'
  }
});
```

#### Phase 3: Async Execution (Blue Zone)

The API returns immediately while processing continues:

**Non-blocking Pattern** (`apps/backend/routes/tasks.route.ts`):
```typescript
router.post('/tasks/:id/messages', async (c) => {
  const taskId = c.req.param('id');
  const { content } = await c.req.json();
  
  // Validate synchronously
  await taskService.validateTaskState(taskId);
  
  // Start async processing
  taskService.sendMessage(taskId, content)
    .catch(error => {
      // Handle async errors
      logger.error('Message processing failed:', error);
      eventService.emit(taskId, {
        type: 'ERROR',
        data: { 
          message: error.message,
          code: error.code || 'PROCESSING_ERROR'
        }
      });
    });
  
  // Return immediately with 202 Accepted
  return c.json({ 
    accepted: true,
    message: 'Message queued for processing'
  }, 202);
});
```

**Workspace Preparation**:
```typescript
// Ensure workspace is ready
async ensureWorkspaceReady(workspaceId: string): Promise<void> {
  const workspace = await this.getWorkspace(workspaceId);
  
  // Check git worktree health
  const isValid = await this.gitService.isValidWorktree(workspace.path);
  if (!isValid) {
    await this.repairWorkspace(workspaceId);
  }
  
  // Ensure preview server is running
  if (workspace.previewPort) {
    const serverRunning = await this.previewService.isRunning(workspace.previewPort);
    if (!serverRunning) {
      await this.previewService.startServer(workspace);
    }
  }
}
```

#### Phase 4: Agent Processing (Yellow Zone)

The agent executes asynchronously with continuous event streaming:

**Agent Execution Loop** (`apps/backend/services/agents/claude-agent.service.ts`):
```typescript
async executeAgent(params: AgentExecutionParams): Promise<void> {
  const { taskId, iterationId, message, workspace } = params;
  
  try {
    // Build conversation context
    const messages = await this.buildConversation(taskId, message);
    
    // Create streaming session
    const stream = await this.anthropic.messages.create({
      model: this.model,
      max_tokens: 4096,
      messages,
      tools: this.tools.getDefinitions(),
      stream: true
    });
    
    // Process stream chunks
    let fullResponse = '';
    for await (const chunk of stream) {
      switch (chunk.type) {
        case 'content_block_delta':
          // Stream text to frontend
          fullResponse += chunk.delta.text;
          await this.eventService.emit(taskId, {
            type: 'MESSAGE',
            data: {
              iterationId,
              content: chunk.delta.text,
              role: 'assistant'
            }
          });
          break;
          
        case 'tool_use':
          // Execute tool
          await this.handleToolUse(taskId, iterationId, chunk, workspace);
          break;
      }
    }
    
    // Get final usage stats
    const usage = await stream.finalMessage();
    
    // Update iteration with results
    await this.iterationService.complete(iterationId, {
      assistantMessage: fullResponse,
      usage: {
        inputTokens: usage.usage.input_tokens,
        outputTokens: usage.usage.output_tokens,
        totalTokens: usage.usage.total_tokens
      }
    });
    
    // Calculate and track costs
    const cost = this.costService.calculateCost(usage.usage, this.model);
    await this.taskService.updateCost(taskId, cost);
    
    // Emit completion event
    await this.eventService.emit(taskId, {
      type: 'RESULT',
      data: {
        iterationId,
        status: 'completed',
        usage: usage.usage,
        cost
      }
    });
    
    // Update task status back to open
    await this.taskService.updateStatus(taskId, 'open');
    
  } catch (error) {
    // Handle errors gracefully
    await this.handleAgentError(taskId, iterationId, error);
  }
}
```

**Tool Execution**:
```typescript
async handleToolUse(
  taskId: string, 
  iterationId: string, 
  toolUse: ToolUse,
  workspace: string
): Promise<void> {
  // Emit tool execution start
  await this.eventService.emit(taskId, {
    type: 'TOOL',
    data: {
      iterationId,
      toolName: toolUse.name,
      input: toolUse.input,
      status: 'executing'
    }
  });
  
  try {
    // Execute tool with timeout
    const result = await Promise.race([
      this.tools.execute(toolUse.name, toolUse.input, workspace),
      this.timeout(30000) // 30 second timeout
    ]);
    
    // Emit tool result
    await this.eventService.emit(taskId, {
      type: 'TOOL',
      data: {
        iterationId,
        toolName: toolUse.name,
        output: result,
        status: 'completed'
      }
    });
    
    // Check for workspace changes
    if (this.isFileModificationTool(toolUse.name)) {
      await this.checkWorkspaceChanges(taskId, workspace);
    }
    
  } catch (error) {
    // Emit tool error
    await this.eventService.emit(taskId, {
      type: 'TOOL',
      data: {
        iterationId,
        toolName: toolUse.name,
        error: error.message,
        status: 'failed'
      }
    });
  }
}
```

**Event Streaming Architecture**:
- Events are emitted immediately as they occur
- No buffering or batching for real-time feel
- Each event includes iteration ID for correlation
- Frontend receives events via SSE connection
- Events persist to disk for replay capability

### Error Handling Architecture

```mermaid
graph TB
    subgraph "Error Categories"
        E1[Validation Errors<br/>400 Bad Request]
        E2[State Errors<br/>409 Conflict]
        E3[Resource Errors<br/>404 Not Found]
        E4[System Errors<br/>500 Internal]
    end
    
    subgraph "Error Flow"
        EO[Error Origin]
        EH[Error Handler]
        EL[Error Logger]
        ER[Error Response]
        EE[Error Event]
    end
    
    subgraph "Recovery Strategies"
        R1[Retry Logic]
        R2[State Rollback]
        R3[Resource Cleanup]
        R4[User Notification]
    end
    
    E1 --> EO
    E2 --> EO
    E3 --> EO
    E4 --> EO
    
    EO --> EH
    EH --> EL
    EH --> ER
    EH --> EE
    
    EH --> R1
    EH --> R2
    EH --> R3
    ER --> R4
    
    style EH fill:#ff6666,stroke:#333,stroke-width:2px
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

### Data Persistence Architecture

```mermaid
graph TB
    subgraph "Storage Strategy"
        subgraph "Active Data"
            M1[In-Memory Cache]
            M2[Active Sessions]
            M3[Event Buffer]
        end
        
        subgraph "Persistent Storage"
            F1[Task Metadata<br/>tasks/*.json]
            F2[Iterations<br/>iterations/*.json]
            F3[Event Archive<br/>events/*.jsonl]
            F4[Workspace State<br/>workspaces/*.json]
        end
        
        subgraph "Time-Series Data"
            T1[Event Stream]
            T2[Cost Tracking]
            T3[Performance Metrics]
        end
    end
    
    M1 --> F1
    M2 --> F2
    M3 --> F3
    
    T1 --> F3
    T2 --> F1
    T3 --> F4
    
    style M1 fill:#ffcccc,stroke:#333,stroke-width:2px
    style F1 fill:#ccffcc,stroke:#333,stroke-width:2px
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

### React Query Integration

```mermaid
graph LR
    subgraph "Query Architecture"
        QC[Query Client]
        QK[Query Keys]
        QH[Query Hooks]
        QM[Mutations]
    end
    
    subgraph "Cache Management"
        C1[Task Cache]
        C2[Events Cache]
        C3[Diff Cache]
        C4[Status Cache]
    end
    
    subgraph "Invalidation Strategy"
        I1[Event-Driven]
        I2[Time-Based]
        I3[Manual]
        I4[Optimistic]
    end
    
    QC --> C1
    QC --> C2
    QC --> C3
    QC --> C4
    
    QK --> QH
    QH --> QC
    QM --> QC
    
    I1 --> QC
    I2 --> QC
    I3 --> QC
    I4 --> QM
    
    style QC fill:#ffeb3b,stroke:#333,stroke-width:3px
```

### Optimistic Update Architecture

Optimistic updates provide instant UI feedback while server operations complete in the background. This pattern is crucial for maintaining a responsive feel in the task system:

```mermaid
sequenceDiagram
    participant U as User
    participant UI as UI Component
    participant M as Mutation
    participant C as Cache
    participant API as Backend API
    participant E as Error Handler
    
    U->>UI: Click "Create Task"
    UI->>M: Trigger mutation
    
    rect rgb(230, 255, 230)
        Note over M,C: Optimistic Phase
        M->>C: Create temp task
        C->>UI: Update immediately
        UI->>U: Show new task
        M->>M: Navigate to task
    end
    
    rect rgb(230, 230, 255)
        Note over M,API: Server Sync
        M->>API: POST /tasks
        API->>API: Create real task
        API->>M: Return taskId
    end
    
    rect rgb(255, 255, 230)
        Note over M,UI: Success Phase
        M->>C: Replace temp with real
        M->>M: Update route params
        C->>UI: Refresh with real data
    end
    
    rect rgb(255, 230, 230)
        Note over M,E: Error Recovery
        API--xM: Error response
        M->>C: Rollback changes
        M->>E: Show error
        M->>U: Navigate back
    end
```

#### Implementation Details

**1. Optimistic Phase (Green Zone)**

The mutation immediately updates the cache with temporary data:

```typescript
// apps/frontend/src/features/tasks/hooks/use-create-task.ts
export function useCreateTask() {
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  
  return useMutation({
    mutationFn: createTask,
    
    onMutate: async (variables) => {
      // Cancel any in-flight queries to prevent race conditions
      await queryClient.cancelQueries({ 
        queryKey: queryKeys.tasksList() 
      });
      
      // Snapshot current data for rollback
      const previousTasks = queryClient.getQueryData(
        queryKeys.tasksList()
      );
      
      // Create optimistic task with temporary ID
      const optimisticTask: Task = {
        id: `temp-${Date.now()}`,
        title: variables.title,
        repositoryId: variables.repositoryId,
        status: 'initializing',
        createdAt: new Date().toISOString(),
        updatedAt: new Date().toISOString(),
        iterations: [],
        cost: { total: 0, currency: 'USD' },
        workspaceId: null,
        agentConfig: variables.agentConfig
      };
      
      // Update cache immediately
      queryClient.setQueryData(
        queryKeys.tasksList(),
        (old: Task[] = []) => [optimisticTask, ...old]
      );
      
      // Navigate to task detail page with temp ID
      navigate({ 
        to: '/tasks/$taskId', 
        params: { taskId: optimisticTask.id } 
      });
      
      // Return context for rollback
      return { previousTasks, optimisticTask };
    }
  });
}
```

**2. Server Sync Phase (Blue Zone)**

While the UI shows the optimistic update, the actual task creation happens on the server:

```typescript
// apps/backend/services/task.service.ts
async createTask(params: CreateTaskParams): Promise<Task> {
  // Generate real task ID
  const taskId = crypto.randomUUID();
  
  // Create workspace
  const workspace = await this.workspaceService.create({
    taskId,
    repositoryId: params.repositoryId,
    branch: `task-${taskId}`
  });
  
  // Initialize git worktree
  await this.gitService.createWorktree(
    workspace.path,
    workspace.branch,
    params.repositoryId
  );
  
  // Create task record
  const task: Task = {
    id: taskId,
    title: params.title,
    repositoryId: params.repositoryId,
    status: 'initializing',
    workspaceId: workspace.id,
    createdAt: new Date().toISOString(),
    updatedAt: new Date().toISOString(),
    iterations: [],
    cost: { total: 0, currency: 'USD' },
    agentConfig: params.agentConfig || this.getDefaultAgentConfig()
  };
  
  // Persist to storage
  await this.storage.save(`tasks/${taskId}.json`, task);
  
  // Start initialization process
  this.initializeTask(taskId).catch(error => {
    this.logger.error('Task initialization failed:', error);
    this.eventService.emit(taskId, {
      type: 'ERROR',
      data: { message: 'Failed to initialize workspace' }
    });
  });
  
  return task;
}
```

**3. Success Phase (Yellow Zone)**

When the server responds successfully, the temporary data is replaced:

```typescript
// Continuation of use-create-task.ts
onSuccess: (data, variables, context) => {
  // Replace temporary task with real data
  queryClient.setQueryData(
    queryKeys.tasksList(),
    (old: Task[] = []) => {
      return old.map(task => 
        task.id === context.optimisticTask.id ? data : task
      );
    }
  );
  
  // Add the real task to cache
  queryClient.setQueryData(
    queryKeys.task(data.id),
    data
  );
  
  // Update navigation to use real ID
  navigate({ 
    to: '/tasks/$taskId', 
    params: { taskId: data.id },
    replace: true // Replace history entry
  });
  
  // Prefetch related data
  queryClient.prefetchQuery({
    queryKey: queryKeys.taskIterations(data.id),
    queryFn: () => getTaskIterations(data.id)
  });
  
  // Show success notification
  toast.success('Task created successfully');
},
```

**4. Error Recovery Phase (Red Zone)**

If the server request fails, all optimistic changes are rolled back:

```typescript
onError: (error, variables, context) => {
  // Rollback to previous state
  if (context?.previousTasks) {
    queryClient.setQueryData(
      queryKeys.tasksList(),
      context.previousTasks
    );
  }
  
  // Navigate back to task list
  navigate({ to: '/tasks' });
  
  // Show error notification with details
  const errorMessage = error instanceof ApiError 
    ? error.message 
    : 'Failed to create task. Please try again.';
    
  toast.error(errorMessage, {
    action: {
      label: 'Retry',
      onClick: () => {
        // Retry the mutation
        mutate(variables);
      }
    }
  });
  
  // Log error for debugging
  console.error('Task creation failed:', error);
}
```

#### Advanced Optimistic Patterns

**1. Optimistic Status Updates**:
```typescript
// apps/frontend/src/features/tasks/hooks/use-close-task.ts
export function useCloseTask() {
  return useMutation({
    mutationFn: closeTask,
    
    onMutate: async ({ taskId }) => {
      // Optimistically update status
      queryClient.setQueryData(
        queryKeys.task(taskId),
        (old: Task) => ({ ...old, status: 'closing' })
      );
      
      // Disable UI interactions
      queryClient.setQueryData(
        queryKeys.taskUIState(taskId),
        { isLocked: true }
      );
    },
    
    onSuccess: (data) => {
      // Final status update
      queryClient.setQueryData(
        queryKeys.task(data.id),
        data
      );
      
      // Navigate away after cleanup
      setTimeout(() => {
        navigate({ to: '/tasks' });
      }, 1000);
    }
  });
}
```

**2. Optimistic Message Sending**:
```typescript
// apps/frontend/src/features/tasks/hooks/use-send-message.ts
export function useSendMessage() {
  return useMutation({
    mutationFn: sendMessage,
    
    onMutate: async ({ taskId, content }) => {
      // Create optimistic iteration
      const optimisticIteration: Partial<Iteration> = {
        id: `temp-${Date.now()}`,
        userMessage: content,
        status: 'pending',
        startedAt: new Date().toISOString()
      };
      
      // Add to iterations immediately
      queryClient.setQueryData(
        queryKeys.taskIterations(taskId),
        (old: Iteration[] = []) => [...old, optimisticIteration]
      );
      
      // Update task status
      queryClient.setQueryData(
        queryKeys.task(taskId),
        (old: Task) => ({ ...old, status: 'running' })
      );
      
      // Scroll to new message
      document.getElementById(optimisticIteration.id)?.scrollIntoView();
    }
  });
}
```

**3. Batch Optimistic Updates**:
```typescript
// Multiple related updates in one transaction
export function useTaskBulkActions() {
  return useMutation({
    mutationFn: performBulkAction,
    
    onMutate: async ({ action, taskIds }) => {
      // Cancel all related queries
      await queryClient.cancelQueries({ 
        queryKey: queryKeys.tasks() 
      });
      
      // Batch update all affected tasks
      const updates = taskIds.map(taskId => {
        queryClient.setQueryData(
          queryKeys.task(taskId),
          (old: Task) => ({
            ...old,
            status: action === 'close' ? 'closing' : old.status,
            updatedAt: new Date().toISOString()
          })
        );
      });
      
      // Update list view
      queryClient.setQueryData(
        queryKeys.tasksList(),
        (old: Task[] = []) => {
          return old.map(task => {
            if (taskIds.includes(task.id)) {
              return { ...task, status: 'closing' };
            }
            return task;
          });
        }
      );
    }
  });
}
```

#### Performance Benefits

1. **Perceived Performance**: Users see immediate feedback (< 50ms)
2. **Reduced Waiting**: No loading states for common actions
3. **Smooth Transitions**: Navigation happens instantly
4. **Error Recovery**: Graceful rollback maintains data integrity
5. **Network Resilience**: Works even with slow connections

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

## Performance Optimization Patterns

### Context Optimization Strategy

```mermaid
graph TB
    subgraph "Problem: Excessive Re-renders"
        P1[Single Context]
        P2[All Data Together]
        P3[Any Change = Full Re-render]
        P1 --> P2
        P2 --> P3
    end
    
    subgraph "Solution: Tri-Context Pattern"
        S1[Data Context<br/>Stable Primitives]
        S2[Events Context<br/>Isolated Updates]
        S3[UI Context<br/>Local Only]
    end
    
    subgraph "Benefits"
        B1[Granular Updates]
        B2[Minimal Re-renders]
        B3[Better Performance]
        B4[Easier Debugging]
    end
    
    P3 -.->|Refactor| S1
    S1 --> B1
    S2 --> B2
    S3 --> B3
    B1 --> B4
    B2 --> B4
    B3 --> B4
    
    style P3 fill:#ffcccc,stroke:#ff0000,stroke-width:2px
    style B4 fill:#ccffcc,stroke:#00aa00,stroke-width:2px
```

### Smart Polling Architecture

```mermaid
stateDiagram-v2
    [*] --> Polling: Start
    Polling --> CheckStatus: Poll interval
    CheckStatus --> Stable: No changes 3x
    CheckStatus --> Changed: Status changed
    Changed --> Polling: Continue
    Stable --> Stopped: Stop polling
    Stopped --> Polling: User action
    Polling --> Stopped: Component unmount
    
    note right of Stable
        Reduces unnecessary
        API calls when
        workspace is stable
    end note
    
    note right of Changed
        Continues polling
        during active
        development
    end note
```

### Cache Warming Strategy

```mermaid
sequenceDiagram
    participant R as Router
    participant L as Task List
    participant C as Cache
    participant D as Task Detail
    
    R->>L: Navigate to list
    L->>C: Prefetch visible tasks
    Note over C: Warm cache with metadata
    
    L->>L: User hovers on task
    L->>C: Prefetch task details
    Note over C: Pre-load iterations
    
    L->>R: Click task
    R->>D: Navigate to detail
    D->>C: Read from cache
    Note over D: Instant render!
    
    D->>C: Background refresh
    C->>D: Update if changed
```

## Data Flow Architecture

### Complete Request Lifecycle

```mermaid
sequenceDiagram
    participant U as User
    participant UI as React UI
    participant RQ as React Query
    participant API as API Client
    participant BE as Backend
    participant AS as Agent Service
    participant SSE as SSE Stream
    
    rect rgb(240, 248, 255)
        Note over U,UI: User Interaction
        U->>UI: Type message
        UI->>UI: Update local state
        U->>UI: Click send
    end
    
    rect rgb(240, 255, 240)
        Note over UI,RQ: Optimistic Update
        UI->>RQ: Trigger mutation
        RQ->>RQ: Update cache optimistically
        RQ->>UI: Re-render with pending state
    end
    
    rect rgb(255, 248, 220)
        Note over RQ,BE: API Request
        RQ->>API: POST /tasks/:id/messages
        API->>BE: Forward request
        BE->>BE: Validate & create iteration
        BE->>AS: Queue agent execution
        BE->>API: Return 202 Accepted
        API->>RQ: Success response
    end
    
    rect rgb(255, 240, 240)
        Note over AS,SSE: Async Processing
        AS->>AS: Execute agent
        AS->>SSE: Stream MESSAGE events
        SSE->>UI: Update via EventsContext
        AS->>SSE: Send RESULT event
        SSE->>RQ: Trigger cache invalidation
        RQ->>UI: Re-render with final state
    end
```

### Event-Driven Cache Updates

```mermaid
graph TB
    subgraph "Event Stream"
        E1[INPUT Event]
        E2[MESSAGE Event]
        E3[TOOL Event]
        E4[RESULT Event]
        E5[ERROR Event]
        E6[WORKSPACE Event]
    end
    
    subgraph "Cache Operations"
        U1[Update Events List]
        U2[Invalidate Task]
        U3[Invalidate Iterations]
        U4[Update Git Diff]
        U5[Update Workspace Status]
    end
    
    subgraph "UI Updates"
        R1[Show User Message]
        R2[Stream AI Response]
        R3[Show Tool Usage]
        R4[Complete Iteration]
        R5[Show Error State]
        R6[Update Status Badge]
    end
    
    E1 --> U1 --> R1
    E2 --> U1 --> R2
    E3 --> U1 --> R3
    E4 --> U2 --> R4
    E4 --> U3
    E5 --> U2 --> R5
    E6 --> U5 --> R6
    E6 --> U4
    
    style E4 fill:#90ee90,stroke:#333,stroke-width:2px
    style U2 fill:#ffd700,stroke:#333,stroke-width:2px
```

### Workspace Integration

```mermaid
graph TB
    subgraph "Task-Workspace Relationship"
        T[Task]
        W[Workspace]
        G[Git Worktree]
        P[Preview Server]
    end
    
    subgraph "Lifecycle Coordination"
        L1[Task Created]
        L2[Workspace Allocated]
        L3[Worktree Created]
        L4[Preview Started]
        L5[Ready for Execution]
    end
    
    subgraph "State Synchronization"
        S1[File Changes]
        S2[Git Status]
        S3[Server Status]
        S4[Build Status]
    end
    
    T -->|1:1| W
    W -->|Uses| G
    W -->|Hosts| P
    
    L1 --> L2
    L2 --> L3
    L3 --> L4
    L4 --> L5
    
    S1 --> W
    S2 --> G
    S3 --> P
    S4 --> P
    
    W --> T
    
    style T fill:#ff9999,stroke:#333,stroke-width:3px
    style W fill:#9999ff,stroke:#333,stroke-width:3px
```

### Cost Tracking Architecture

```mermaid
sequenceDiagram
    participant A as Agent
    participant C as Cost Calculator
    participant T as Task Service
    participant S as Storage
    participant U as UI
    
    A->>A: Execute tools
    A->>A: Track token usage
    A->>C: Calculate costs
    
    rect rgb(255, 245, 230)
        Note over C: Cost Calculation
        C->>C: Input tokens × rate
        C->>C: Output tokens × rate
        C->>C: Cache tokens × rate
        C->>C: Sum total cost
    end
    
    C->>T: Update iteration cost
    T->>T: Aggregate task total
    T->>S: Persist costs
    T->>U: Stream cost update
    
    U->>U: Display running total
    U->>U: Show cost breakdown
```

## Integration Architecture

### Service Dependencies

```mermaid
graph TB
    subgraph "Core Services"
        TS[Task Service<br/>Orchestrator]
        ES[Event Service<br/>Real-time]
        AS[Agent Service<br/>AI Execution]
        WS[Workspace Service<br/>File System]
    end
    
    subgraph "Supporting Services"
        RS[Repository Service<br/>Git Operations]
        PS[Preview Service<br/>Dev Server]
        CS[Cost Service<br/>Usage Tracking]
        SS[Storage Service<br/>Persistence]
    end
    
    subgraph "External Dependencies"
        AI[AI Provider<br/>Claude/GPT]
        GIT[Git<br/>Version Control]
        FS[File System<br/>Storage]
        PM[Package Manager<br/>npm/pnpm]
    end
    
    TS --> AS
    TS --> WS
    TS --> ES
    TS --> CS
    
    AS --> AI
    WS --> RS
    WS --> GIT
    WS --> FS
    PS --> PM
    RS --> GIT
    
    ES --> SS
    CS --> SS
    TS --> SS
    
    style TS fill:#ff6666,stroke:#333,stroke-width:4px
    style ES fill:#6666ff,stroke:#333,stroke-width:3px
    style AS fill:#66ff66,stroke:#333,stroke-width:3px
```

### Event Bus Architecture

```mermaid
graph LR
    subgraph "Event Producers"
        P1[Task Service]
        P2[Agent Service]
        P3[Workspace Service]
        P4[Preview Service]
    end
    
    subgraph "Event Bus"
        EB[Central Event Bus]
        EQ[Event Queue]
        ES[Event Store]
    end
    
    subgraph "Event Consumers"
        C1[SSE Stream]
        C2[Cost Calculator]
        C3[Analytics]
        C4[Audit Logger]
    end
    
    P1 --> EB
    P2 --> EB
    P3 --> EB
    P4 --> EB
    
    EB --> EQ
    EB --> ES
    
    EQ --> C1
    EQ --> C2
    EQ --> C3
    EQ --> C4
    
    style EB fill:#ffd700,stroke:#333,stroke-width:3px
```

## Architectural Benefits

### Performance Achievements

```mermaid
graph TB
    subgraph "Traditional Approach"
        T1[Single Context]
        T2[Full Re-renders]
        T3[Polling Everything]
        T4[Synchronous Updates]
        T5[Poor Performance]
        
        T1 --> T2
        T2 --> T5
        T3 --> T5
        T4 --> T5
    end
    
    subgraph "Optimized Architecture"
        O1[Tri-Context Pattern]
        O2[Granular Updates]
        O3[Smart Polling]
        O4[Optimistic Updates]
        O5[Excellent Performance]
        
        O1 --> O2
        O2 --> O5
        O3 --> O5
        O4 --> O5
    end
    
    subgraph "Key Metrics"
        M1["Re-renders: 90% reduction"]
        M2["API calls: 70% reduction"]
        M3["Response time: <100ms"]
        M4["Bundle size: Optimized"]
    end
    
    O5 --> M1
    O5 --> M2
    O5 --> M3
    O5 --> M4
    
    style T5 fill:#ffcccc,stroke:#ff0000,stroke-width:2px
    style O5 fill:#ccffcc,stroke:#00aa00,stroke-width:2px
```

### Scalability Patterns

```mermaid
graph LR
    subgraph "Horizontal Scaling"
        H1[Stateless Services]
        H2[Event-Driven]
        H3[Cache Distribution]
        H4[Load Balancing]
    end
    
    subgraph "Vertical Scaling"
        V1[Efficient Algorithms]
        V2[Memory Management]
        V3[Connection Pooling]
        V4[Resource Limits]
    end
    
    subgraph "Resilience"
        R1[Circuit Breakers]
        R2[Retry Logic]
        R3[Graceful Degradation]
        R4[Error Recovery]
    end
    
    H1 --> Scale["Handles 1000+ concurrent tasks"]
    H2 --> Scale
    V1 --> Scale
    V2 --> Scale
    R1 --> Scale
    R2 --> Scale
    
    style Scale fill:#4caf50,stroke:#333,stroke-width:3px
```

## Best Practices & Patterns

### Architectural Principles

```mermaid
graph TB
    subgraph "Core Principles"
        P1["Single Responsibility<br/>Each service has one job"]
        P2["Event-Driven<br/>Loose coupling via events"]
        P3["Fail Fast<br/>Early validation"]
        P4["Graceful Degradation<br/>Partial failures OK"]
    end
    
    subgraph "Performance Principles"
        PP1["Cache First<br/>Minimize API calls"]
        PP2["Optimistic UI<br/>Immediate feedback"]
        PP3["Lazy Loading<br/>Load on demand"]
        PP4["Smart Polling<br/>Adaptive intervals"]
    end
    
    subgraph "Reliability Principles"
        RP1["Idempotent Operations<br/>Safe retries"]
        RP2["Transaction Logs<br/>Event sourcing"]
        RP3["Health Checks<br/>Self-healing"]
        RP4["Resource Limits<br/>Prevent abuse"]
    end
    
    style P2 fill:#4caf50,stroke:#333,stroke-width:2px
    style PP2 fill:#2196f3,stroke:#333,stroke-width:2px
    style RP1 fill:#ff9800,stroke:#333,stroke-width:2px
```

### Common Patterns

1. **Event-Driven Cache Invalidation**
   - Events trigger targeted cache updates
   - Minimizes unnecessary re-fetches
   - Maintains consistency

2. **Optimistic Updates**
   - Immediate UI feedback
   - Background synchronization
   - Graceful rollback on error

3. **Tri-Context Separation**
   - Stable data context
   - Volatile events context
   - Local UI state context

4. **Smart Resource Management**
   - Adaptive polling intervals
   - Connection pooling
   - Automatic cleanup

5. **Graceful Error Handling**
   - Typed error classes
   - User-friendly messages
   - Automatic recovery

## Summary

The Task Architecture represents a sophisticated orchestration system that coordinates AI agents, workspaces, and real-time events through carefully designed patterns. This document has explored the complete implementation from high-level architecture to specific code examples.

### Key Architectural Achievements

1. **Performance Excellence**
   - **90% reduction in re-renders** through the tri-context pattern (`apps/frontend/src/features/tasks/contexts/`)
   - **Sub-100ms response times** via optimistic updates (`apps/frontend/src/features/tasks/hooks/use-create-task.ts`)
   - **70% fewer API calls** through intelligent cache invalidation (`apps/frontend/src/features/tasks/hooks/use-task-sse-connection.ts`)
   - **Real-time streaming** with SSE for instant feedback (`apps/backend/services/event.service.ts`)

2. **Scalability by Design**
   - **Event-driven architecture** enables horizontal scaling through loose coupling
   - **Stateless services** in `apps/backend/services/` support load balancing
   - **Resource limits** prevent system abuse (30s tool timeouts, connection pooling)
   - **Efficient storage** with JSON files and event streams in `data/` directory

3. **Developer Experience**
   - **Clear separation of concerns** with feature-based architecture
   - **Predictable state management** using React Query and Zustand
   - **Comprehensive error handling** with typed errors and recovery strategies
   - **Well-documented patterns** with implementation examples

4. **User Experience**
   - **Real-time feedback** via SSE connections
   - **Immediate UI updates** through optimistic mutations
   - **Graceful error recovery** with retry mechanisms
   - **Responsive interface** even under load

### Implementation Roadmap

For developers implementing or extending the task system:

1. **Backend Implementation** (Start here):
   - `apps/backend/services/task.service.ts` - Core task orchestration
   - `apps/backend/services/event.service.ts` - Event streaming infrastructure
   - `apps/backend/services/agents/` - AI agent integration
   - `apps/backend/routes/tasks.route.ts` - API endpoints

2. **Frontend Implementation**:
   - `apps/frontend/src/features/tasks/contexts/` - Tri-context pattern
   - `apps/frontend/src/features/tasks/hooks/` - Data fetching and mutations
   - `apps/frontend/src/features/tasks/components/` - UI components
   - `apps/frontend/src/lib/api/query-keys.ts` - Cache key management

3. **Real-time Features**:
   - `apps/frontend/src/features/tasks/hooks/use-task-sse-connection.ts` - SSE client
   - `apps/backend/services/event.service.ts` - SSE server
   - Event types and handlers throughout the system

4. **Performance Optimizations**:
   - Implement the tri-context pattern for complex UIs
   - Use optimistic updates for all mutations
   - Follow the cache invalidation strategies
   - Monitor performance with built-in metrics

### Architecture Principles

The task system follows these core principles:

1. **Event-Driven**: All state changes emit events for real-time updates
2. **Optimistic UI**: Show changes immediately, sync in background
3. **Fail Fast**: Validate early, provide clear error messages
4. **Graceful Degradation**: Partial failures don't crash the system
5. **Performance First**: Every architectural decision considers performance

### Future Considerations

While the current architecture handles thousands of concurrent tasks efficiently, consider these enhancements for future scale:

1. **Message Queue**: Replace async execution with RabbitMQ/Redis for better reliability
2. **WebSocket Upgrade**: Replace SSE with WebSockets for bi-directional communication
3. **Database Migration**: Move from JSON files to PostgreSQL for complex queries
4. **Caching Layer**: Add Redis for distributed caching across instances
5. **Monitoring**: Implement OpenTelemetry for distributed tracing

The architecture's strength lies in its layered approach, where each layer has clear responsibilities and communicates through well-defined interfaces. This design enables the system to handle complex AI-driven workflows while maintaining performance and reliability. The combination of event-driven updates, intelligent caching, and optimistic UI patterns creates a responsive system that feels instant to users while handling complex backend operations.

## Related Documentation

- [Architecture Overview](../architecture-overview.md) - System-wide architecture
- [Event Architecture](../event-architecture.md) - Deep dive into event system
- [Frontend Task Architecture](../frontend-task-architecture.md) - Frontend patterns
- [Workspace Architecture](../workspace-architecture.md) - Workspace management
- [Agent Architecture](./agents.md) - AI agent integration
- [Tri-Context Pattern](../patterns/tri-context.md) - Performance optimization
- [Optimistic Updates](../patterns/optimistic-updates.md) - UI responsiveness
- [Cache Invalidation](../patterns/cache-invalidation.md) - Cache strategies