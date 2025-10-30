# Task Lifecycle Pattern

## State Definitions

### Task States
| State | Description | Location |
|-------|-------------|----------|
| `open` | Task container active, can receive iterations | apps/backend/services/task.service.ts:384 |
| `closed` | Task archived, workspace cleaned up | apps/backend/services/task.service.ts:1100 |
| `error` | Task has encountered an error state | apps/backend/services/task.service.ts |

**Note**: Running state is derived from agent state (`agent.state === 'running'`).

### Iteration States
| State | Description | Location |
|-------|-------------|----------|
| `created` | Message received, awaiting execution | apps/backend/services/task.service.ts:399,834 |
| `active` | Agent currently executing | apps/backend/services/task.service.ts:594 |
| `completed` | Execution finished successfully | apps/backend/services/task.service.ts:200 |
| `failed` | Execution error or agent failure | apps/backend/services/task.service.ts:178,260,492,641,776,1022 |

### Workspace States
| State | Description | Location |
|-------|-------------|----------|
| `exists` | File system path valid and accessible | apps/backend/workspaces/workspace.ts:783 |
| `missing` | Path doesn't exist on disk | apps/backend/workspaces/workspace.ts:783 |
| `invalid` | Path exists but corrupted/not git repo | apps/backend/workspaces/workspace.ts:793 |

### Environment States
| State | Description | Location |
|-------|-------------|----------|
| `not_created` | Initial state before setup | apps/backend/environments/local-environment.ts:44 |
| `initializing` | Setup in progress (container creation/workspace init) | apps/backend/environments/local-environment.ts:42 |
| `ready` | Operational, can execute commands/agents | apps/backend/environments/local-environment.ts:43 |
| `error` | Setup or runtime failure | apps/backend/environments/local-environment.ts:41 |
| `cleaning` | Teardown in progress | apps/backend/environments/docker-environment.ts:68 |

### Agent States
| State | Description | Location |
|-------|-------------|----------|
| `idle` | No agent executing | apps/backend/workspaces/workspace.ts:845 |
| `running` | Agent actively processing | apps/backend/workspaces/workspace.ts:845 |

## Task Creation Flow

1. **createTask**: Creates task metadata and initiates workspace - apps/backend/services/task.service.ts:367
   - Creates task record with `status: 'open'` and saves to storage
   - If message provided, creates first iteration with `status: 'created'`
   - Calls workspaceService.createWorkspace to set up execution environment
   - Returns task immediately, setup continues in background

2. **createWorkspace**: Establishes workspace directory and environment - apps/backend/services/workspace.service.ts:30
   - Creates workspace path (repository directly or git worktree based on config)
   - Instantiates environment based on repository config (LocalEnvironment for local execution, DockerEnvironment for containerized)
   - Creates unified Workspace instance composing environment and file operations
   - Stores workspace in activeWorkspaces map

3. **setupWorkspaceInBackground**: Async workspace initialization - apps/backend/services/task.service.ts:463
   - Runs workspace setup without blocking task creation response
   - Calls workspaceService.setupWorkspace for environment initialization
   - If start=true and message exists, automatically starts task execution
   - Updates task status to error if setup fails

4. **setupWorkspace**: Environment-specific initialization - apps/backend/services/workspace.service.ts:534
   - Delegates to workspace.setup which calls environment.setup
   - LocalEnvironment.setup: Validates paths, runs setup commands - apps/backend/environments/local-environment.ts:59
   - DockerEnvironment.setup: Creates/starts container, copies configs, waits for worker - apps/backend/environments/docker-environment.ts:117
   - Updates workspace path in task metadata after successful setup

## Iteration Execution Flow

1. **startTask**: Activates pending iteration for execution - apps/backend/services/task.service.ts:562
   - Validates last iteration has `status: 'created'` (pending execution)
   - Waits for workspace to reach 'idle' state (max 30s)
   - Updates iteration `status: 'active'` to indicate processing started
   - Emits `taskIteration` event with `subType: 'active'` for UI updates
   - Delegates agent execution to AgentService

2. **Agent Execution via AgentService**: TaskService delegates agent execution to AgentService
   - TaskService creates AgentExecutionRequest with task details
   - AgentService.executeAgent() handles workspace validation and agent execution
   - AgentService validates workspace status and prepares execution context
   - AgentService streams execution events back to TaskService
   - TaskService emits events to EventService for frontend consumption
   - Updates iteration end time and duration on completion

3. **environment.runAgent**: Processes agent messages and events - apps/backend/environments/base-environment.ts:53
   - Creates agent instance using environment's createAgent method
   - Stores agent in environment.currentAgent for status tracking
   - Executes agent.run generator with workspace path and iteration number
   - Yields all events from agent execution
   - Clears currentAgent when execution completes or fails

4. **Message Persistence**: AgentService saves messages immediately - apps/backend/services/agent.service.ts:119
   - **saveMessageAsync**: Fire-and-forget message persistence during event processing
     - Extracts message content from agent events with subType 'message'
     - Converts events to AgentMessage format using convertEventToAgentMessage
     - Appends to JSONL file via storage.appendJsonl(`messages/${taskId}`, agentMessage)
     - Non-blocking saves ensure event stream continues uninterrupted
     - Errors logged but don't fail the agent execution
   - **Storage Format**: `data/messages/{taskId}.jsonl` - one AgentMessage JSON per line
   - **Conversion Logic**: Maps Event payload to AgentMessage structure with proper typing

5. **Agent Event Handling**: Reacts to agent events to update state - apps/backend/services/task.service.ts:61
   - handleResultEvent: Processes execution completion - apps/backend/services/task.service.ts:154
     - Updates iteration status to `completed` (success) or `failed` (error)
     - Captures result text and cost information
     - Updates task status back to `open` (ready for next iteration)
     - Captures git status/changes for iteration record
   - handleErrorEvent: Handles execution failures - apps/backend/services/task.service.ts:247
     - Updates iteration status to `failed` with error details
     - Captures error message and stack trace
     - Task status remains `open` (not affected by iteration failure)
   - handleMessageEvent: Updates progress metrics - apps/backend/services/task.service.ts:300
     - Updates context tokens count for token usage tracking

## State Change Propagation

### Backend Event Emission

1. **Task Status Changes**: Emitted when task or iteration status changes
   - Task-level events: Event type `task` for task creation, opening, closing
   - Iteration-level events: Event type `taskIteration` for iteration status changes
   - SubTypes: `created`, `active`, `completed`, `error` (replaces `failed`)
   - Payload includes previousStatus, new status, and iteration number
   - Triggers frontend to refresh task details and lists

2. **Agent Status Changes**: Track agent execution lifecycle
   - Emission points: apps/backend/agents/claude-code/claude-code-agent.ts:81,155,175
   - Event type: `agent`
   - Subtypes:
     - `started`: Emitted when agent begins processing (first non-system event received)
     - `error`: Emitted when agent encounters an error during execution
   - Payload includes error details for error events
   - Allows frontend to track agent progress and handle failures

3. **Environment Status Changes**: Track environment lifecycle transitions
   - Should emit when environment status changes (not_created → initializing → ready)
   - Event type: `environment`
   - Subtype contains new status value (e.g., 'ready', 'error')
   - Allows frontend to update workspace status displays

4. **Event Service**: Central event distribution hub - apps/backend/services/event.service.ts:16
   - Maintains in-memory event cache per task for fast retrieval
   - Emits to task-specific channel: `task:${taskId}` for targeted updates
   - Emits to global channel for system-wide listeners
   - Persists non-claude-code events to storage for history

### Frontend Event Reception

1. **Global Event Stream**: SSE connection for real-time updates - apps/frontend/src/features/events/contexts/global-event-stream-provider.tsx:48
   - Maintains persistent SSE connection to /api/v1/events/stream
   - Receives NDJSON formatted events in real-time
   - Distributes events to task/type/global subscribers
   - Triggers React Query invalidations based on event type

2. **Query Invalidation**: Smart cache updates based on events - apps/frontend/src/features/events/utils/event-handlers.ts
   - Maps event types to affected query keys using invalidation handlers
   - Task status events invalidate task detail and all task lists
   - Result events invalidate messages, events, and git diff queries
   - Environment events invalidate workspace status queries
   - Uses queueMicrotask for optimal re-render batching

3. **Component Updates**: Automatic UI refresh on state changes
   - Components using React Query hooks re-render when queries invalidate
   - useTask hook fetches updated task with iterations - apps/frontend/src/features/tasks/hooks/use-tasks.ts:46
   - useWorkspaceStatus hook retrieves new environment/agent states
   - Task context propagates changes to all child components

## Workspace Status Architecture

### Separation of Concerns
As of recent changes, workspace status is fetched separately from task data to improve performance and enable more granular cache invalidation:

1. **Task Data**: Core task metadata (status, iterations, cost, etc.)
   - Fetched via `/api/v1/tasks/:taskId` endpoint
   - Cached with longer stale time (30s) as it changes less frequently
   - Includes only task-specific information, no workspace state

2. **Workspace Status**: Dynamic workspace state (agent, environment, git)
   - Fetched via `/api/v1/tasks/:taskId/workspace-status` endpoint
   - Only fetched when task status is `open` (not needed for closed/merged tasks)
   - Polled during transitional states (agent running, environment initializing)
   - Cached with shorter stale time (2s) for real-time updates

### Smart Fetching Strategy
The `useWorkspaceStatus` hook implements intelligent fetching:
```typescript
// Only fetch workspace status for open tasks
const shouldFetch = task?.status === 'open'

// Poll during transitional states
const shouldPoll = workspace?.agent?.state === 'running' || 
                  workspace?.environment?.status === 'initializing' ||
                  workspace?.environment?.status === 'cleaning'
```

### Cache Invalidation Patterns
Different event types trigger specific cache invalidations:
- **Task events** (`task`, `taskIteration`): Invalidate task queries only
- **Workspace events** (`workspace`): Invalidate workspace status and related queries
- **Agent events** (`agent`): SubType-aware invalidation - `message` only affects messages, `completed` affects task and git
- **Environment events** (`environment`): Invalidate workspace status only

This separation ensures:
- Task lists don't refetch when only workspace status changes
- Workspace polling doesn't trigger unnecessary task data fetches
- Components can subscribe to only the data they need

## Key State Transitions

### Task Lifecycle
1. **Created**: Task initialized with `status: 'open'`, no iterations - ready to receive messages
2. **Message sent**: Creates new iteration with `status: 'created'` - queued for execution
3. **Execution starts**: Iteration transitions to `status: 'active'` - agent processing
4. **Execution completes**: Iteration becomes `status: 'completed'`, task remains `status: 'open'` - ready for next message
5. **Execution fails**: Iteration marked `status: 'failed'`, task goes to `status: 'error'` - indicates problem
6. **Task closed**: Task set to `status: 'closed'`, workspace cleaned up, resources released - end of lifecycle

### Workspace Lifecycle
1. **Created**: Workspace instance created, environment `status: 'not_created'` - awaiting setup
2. **Setup called**: Environment transitions to `status: 'initializing'` - creating container/validating paths
3. **Setup complete**: Environment reaches `status: 'ready'` - can execute commands and agents
4. **Agent running**: Agent state becomes `state: 'running'` - actively processing iteration
5. **Agent stopped**: Agent returns to `state: 'idle'` - execution complete or stopped
6. **Teardown**: Environment enters `status: 'cleaning'` - stopping services and cleaning resources

## Critical Methods

### Task Service
- **createTask**: Entry point for new tasks, returns immediately
- **startTask**: Activates pending iteration for execution  
- **sendMessage**: Creates new iteration and delegates execution to AgentService
- **handleResultEvent**: Processes execution completion, updates states
- **handleErrorEvent**: Handles execution failures, marks iteration failed
- **closeTask**: Archives task and cleans up all resources

### Agent Service
- **executeAgent**: Main entry point for agent execution with workspace validation
- **getAgentStatus**: Returns current agent status and session information
- **stopAgent**: Stops running agent execution
- **prepareMessageWithAttachments**: Handles file attachment processing  
- **checkPreviewServerAvailability**: Determines if preview server features are available
- **generateSystemPromptParts**: Creates context-aware system prompts

### Workspace Service
- **createWorkspace**: Initializes workspace directory and environment - apps/backend/services/workspace.service.ts:30
- **setupWorkspace**: Runs environment-specific initialization - apps/backend/services/workspace.service.ts:534
- **getWorkspaceStatus**: Returns comprehensive status object - apps/backend/services/workspace.service.ts:566
- **cleanupWorkspace**: Tears down environment and releases resources - apps/backend/services/workspace.service.ts:483

### Environment
- **runAgent**: Main entry for agent execution requests - apps/backend/environments/base-environment.ts:53
- **getAgentStatus**: Returns current agent status and session information - apps/backend/environments/base-environment.ts:67
- **stopAgent**: Interrupts running agent execution - apps/backend/environments/base-environment.ts:86
- **isRunning**: Checks if agent is currently executing - apps/backend/environments/base-environment.ts:135

### Workspace Class
- **getWorkspaceStatus**: Computes full status with all sub-states - apps/backend/workspaces/workspace.ts:426
- **getWorkspaceExistence**: Checks if workspace path is valid - apps/backend/workspaces/workspace.ts:774
- **getWorkspaceChangeStatus**: Calculates git change status - apps/backend/workspaces/workspace.ts:802
- **getAgentState**: Returns current agent execution state - apps/backend/workspaces/workspace.ts:842

### Message Reading Flow

The message reading system implements a hierarchical approach for maximum reliability and backward compatibility:

1. **MessageService.getTaskMessages()**: Main entry point for message retrieval - apps/backend/services/message.service.ts:27
   - First attempts to read from local storage: `data/messages/{taskId}.jsonl`
   - If local storage is empty, falls back to Claude directory: `~/.claude/sessions/{sessionId}.jsonl`
   - Transforms AgentMessage format to UI Message format for frontend consumption

2. **Local Storage Priority**: Modern message persistence (AgentService → Storage)
   - **getMessagesFromRepository()**: Reads from the task message repository - apps/backend/src/features/agents/services/message.service.ts
   - Uses storage.readJsonl() to parse each line as AgentMessage
   - Provides fast access without depending on Claude's file system
   - Handles parse errors gracefully, skipping corrupted lines

3. **Claude Directory Fallback**: Backward compatibility (legacy tasks)
   - **readMessagesFromClaudeDirectory()**: Existing logic for Claude file reading
   - Searches Claude's session directories for matching session IDs
   - Transforms Claude's JSONL format to AgentMessage format
   - Maintains support for tasks created before local persistence

4. **Migration Strategy**:
   - **New Tasks**: Messages saved immediately to local storage during execution
   - **Existing Tasks**: Continue reading from Claude directory until new messages generated
   - **Hybrid Support**: Can read from both sources as tasks transition to new system

### Frontend Hooks
- **useTask**: Fetches single task with all iterations - apps/frontend/src/features/tasks/hooks/use-tasks.ts:46
- **useTasksList**: Retrieves paginated task list - apps/frontend/src/features/tasks/hooks/use-tasks.ts:30
- **useSendMessage**: Mutation for sending new messages - apps/frontend/src/features/tasks/hooks/use-tasks.ts:91
- **useWorkspaceStatus**: Polls workspace status during operations

## Status Computation

### Workspace Status Response Structure
Returned by: apps/backend/workspaces/workspace.ts:426
```typescript
{
  // Workspace info (flattened)
  workspaceState: 'exists' | 'missing' | 'invalid',
  workspacePath: string,
  workspaceChangeStatus?: 'no_changes' | 'merged' | 'committed_unmerged' | 'uncommitted' | 'conflicts',
  
  // Environment info (flattened)
  environmentType: 'local' | 'docker',
  environmentStatus: 'not_created' | 'initializing' | 'ready' | 'error' | 'cleaning',
  environmentError?: string,
  environmentCreatedAt: string,
  
  // Agent info (flattened)
  agentState: 'idle' | 'running' | 'none',
  agentType?: string,
  
  // Git status (unchanged)
  gitStatus?: GitStatus,
  
  // Capabilities (unchanged)
  capabilities: {
    fileOperations: boolean,
    commandExecution: boolean,
    serverManagement: boolean
  },
  
  // Computed states (added)
  isRunning: boolean,
  workspaceReady: boolean,
  canExecute: boolean
}
```

### Change Status Calculation
Determines git synchronization state - apps/backend/workspaces/workspace.ts:802
- `conflicts`: Git merge conflicts detected - gitStatus.hasConflicts
- `uncommitted`: Working directory has unstaged/uncommitted changes - gitStatus.hasUncommittedChanges
- `committed_unmerged`: Local commits not yet pushed/merged - gitStatus.commitsAhead > 0
- `merged`: Fully synchronized with main branch - commitsAhead === 0 && commitsBehind === 0
- `no_changes`: Clean working directory, no divergence - default state
