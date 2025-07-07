# Agent Architecture

## Table of Contents
1. [Overview](#overview)
2. [Core Concepts](#core-concepts)
3. [Architecture](#architecture)
4. [Backend Implementation](#backend-implementation)
5. [Frontend Implementation](#frontend-implementation)
6. [Data Flow](#data-flow)
7. [Integration Points](#integration-points)
8. [Configuration](#configuration)
9. [Best Practices](#best-practices)
10. [Troubleshooting](#troubleshooting)

## Overview

### Purpose
The Agent system provides the core AI capabilities for Moatless Swarm UI, enabling AI-powered code generation, modification, and analysis. It integrates Claude Code and other AI agents into the task execution pipeline with proper workspace access, tool execution, and real-time streaming.

### Key Responsibilities
- Execute AI agents with workspace context
- Manage agent lifecycle (start, run, stop)
- Stream responses in real-time
- Execute tools (web fetch, search, bash commands)
- Track costs and usage metrics
- Handle errors and recovery
- Maintain session continuity
- Emit events for UI updates

### Position in Architecture
Agents are the intelligence layer that processes user requests, modifies code, and provides insights. They sit between the task management system and workspace execution environment.

## Core Concepts

### Domain Terminology

| Term | Definition |
|------|------------|
| Agent | AI model that processes messages and executes actions |
| Tool | Capability available to agents (bash, web search, etc.) |
| Iteration | Single request-response cycle within a task |
| Session | Continuous conversation context across iterations |
| Message Event | Streamed response chunk from agent |
| Result Event | Final outcome of agent execution |
| Tool Use | Agent invoking a tool to perform an action |

### Key Abstractions

#### Base Agent
```typescript
abstract class BaseAgent {
  protected sessionId: string | null = null
  protected stopRequested = false
  protected currentIteration = 0
  
  abstract getAgentType(): string
  abstract run(
    message: string,
    options: AgentRunOptions
  ): AsyncGenerator<AgentEvent>
  
  stop(): void {
    this.stopRequested = true
  }
  
  getSessionId(): string | null {
    return this.sessionId
  }
}
```

#### Agent Event
```typescript
interface AgentEvent {
  type: 'message' | 'result' | 'error'
  subtype?: string
  data: any
  timestamp: string
  iteration?: number
  sessionId?: string
}
```

### Design Decisions
- **One agent per task** prevents concurrent execution conflicts
- **Event streaming** provides real-time feedback
- **Background execution** keeps API responsive
- **Tool permissions** are configurable per agent
- **Session continuity** enables resuming conversations
- **Workspace isolation** ensures security

## Architecture

### High-Level Overview

The Agent system orchestrates AI-powered code modifications through a carefully designed flow that ensures workspace isolation, real-time feedback, and reliable execution.

```mermaid
graph TB
    subgraph "Client Layer"
        UI[User Interface]
        TM[Task Manager]
    end
    
    subgraph "Agent Orchestration Layer"
        AS[Agent Service]
        AQ[Agent Queue]
        AM[Agent Monitor]
    end
    
    subgraph "Agent Execution Layer"
        BA[Base Agent]
        CC[Claude Code Agent]
        CA[Custom Agents]
    end
    
    subgraph "Tool Execution Layer"
        TE[Tool Executor]
        TM2[Tool Manager]
        TP[Tool Permissions]
    end
    
    subgraph "Integration Layer"
        WS[Workspace Service]
        ES[Event Service]
        SS[Storage Service]
    end
    
    subgraph "External Systems"
        SDK[Claude Code SDK]
        AI[AI APIs]
        GH[GitHub API]
    end
    
    UI -->|User Message| TM
    TM -->|Create Iteration| AS
    AS -->|Queue Task| AQ
    AQ -->|Execute| BA
    BA -->|Polymorphic| CC
    BA -->|Polymorphic| CA
    
    CC -->|API Calls| SDK
    CA -->|API Calls| AI
    
    BA -->|Execute Tools| TE
    TE -->|Check Permissions| TP
    TE -->|Route Tool| TM2
    
    AS <-->|Events| ES
    AS <-->|Workspace| WS
    AS <-->|Persist| SS
    
    ES -->|SSE Stream| UI
    AM -->|Monitor Health| AS
    
    style AS fill:#f9f,stroke:#333,stroke-width:4px
    style CC fill:#bbf,stroke:#333,stroke-width:2px
    style ES fill:#fbf,stroke:#333,stroke-width:2px
```

#### Component Flow Description

The high-level architecture consists of several interconnected layers, each with specific responsibilities:

1. **Client Layer**:
   - **User Interface** (`apps/frontend/src/features/tasks/components/chat-input.tsx`): Captures user messages and displays agent responses
   - **Task Manager** (`apps/frontend/src/features/tasks/hooks/use-create-iteration.ts`): Manages task iterations and coordinates with the backend

2. **Agent Orchestration Layer**:
   - **Agent Service** (`apps/backend/services/agent.service.ts`): Central service that manages agent lifecycle, handles execution requests, and coordinates with other services
   - **Agent Queue**: In-memory queue that prevents concurrent agent execution on the same task
   - **Agent Monitor**: Health monitoring component that tracks agent performance and resource usage

3. **Agent Execution Layer**:
   - **Base Agent** (`apps/backend/services/agents/base.agent.ts`): Abstract base class defining the agent interface and common functionality
   - **Claude Code Agent** (`apps/backend/services/agents/claude-code.agent.ts`): Concrete implementation integrating with Claude Code SDK
   - **Custom Agents**: Placeholder for future agent implementations (e.g., GPT-4, local models)

4. **Tool Execution Layer**:
   - **Tool Executor**: Processes tool requests from agents and delegates to appropriate handlers
   - **Tool Manager**: Manages available tools and their configurations
   - **Tool Permissions**: Enforces security policies and access controls for tool usage

5. **Integration Layer**:
   - **Workspace Service** (`apps/backend/services/workspace.service.ts`): Provides isolated file system access for agent operations
   - **Event Service** (`apps/backend/services/event.service.ts`): Manages real-time event streaming to clients
   - **Storage Service** (`apps/backend/services/storage.service.ts`): Persists agent results and metadata

#### Data Flow Through the System

When a user submits a message, the following sequence occurs:

1. The UI component captures the input and sends it to the Task Manager
2. Task Manager creates a new iteration and calls the Agent Service
3. Agent Service validates the request and queues it for execution
4. The queue dispatcher creates the appropriate agent instance (usually Claude Code)
5. The agent processes the message and may request tool executions
6. Tool requests are validated against permissions before execution
7. Results stream back through the Event Service to the UI in real-time
8. Final results are persisted via the Storage Service

### Agent Execution Flow

The agent execution follows a precise choreography designed for reliability and real-time feedback:

```mermaid
sequenceDiagram
    participant U as User
    participant UI as Frontend
    participant API as API Route
    participant AS as Agent Service
    participant WS as Workspace Service
    participant AG as Agent (Claude)
    participant TE as Tool Executor
    participant ES as Event Service
    
    U->>UI: Submit message
    UI->>API: POST /tasks/{id}/iterations
    API->>AS: executeAgent(taskId, message)
    
    Note over AS: Validation Phase
    AS->>AS: Check for running agents
    AS->>WS: validateWorkspace(taskId)
    WS-->>AS: Workspace info
    
    Note over AS: Initialization Phase
    AS->>AS: createAgent(type, config)
    AS->>AG: Initialize with workspace context
    
    Note over AS,ES: Execution Phase
    AS->>AS: Start background execution
    
    loop Event Stream
        AG->>AG: Process message
        AG->>TE: Execute tool
        TE->>WS: Perform action
        TE-->>AG: Tool result
        AG->>AS: Emit event
        AS->>ES: Publish to stream
        ES->>UI: SSE update
        UI->>U: Real-time feedback
    end
    
    AG->>AS: Final result
    AS->>ES: Result event
    AS->>AS: Cleanup agent
    ES->>UI: Completion
    UI->>U: Show result
```

#### Detailed Execution Flow Steps

**1. User Input Capture**
- **Location**: `apps/frontend/src/features/tasks/components/chat-input.tsx`
- **Process**: User types a message in the chat interface
- **Action**: The ChatInput component captures the text and triggers the submission handler

**2. API Request**
- **Location**: `apps/frontend/src/features/tasks/hooks/use-create-iteration.ts`
- **Process**: Frontend creates a new iteration using React Query mutation
- **Route**: `POST /api/v1/tasks/{taskId}/iterations`
- **Handler**: `apps/backend/routes/task.routes.ts`

**3. Agent Service Invocation**
- **Location**: `apps/backend/services/agent.service.ts` - `executeAgent()` method
- **Process**: The route handler delegates to the Agent Service for execution

**4. Validation Phase**
- **Concurrent Check**: 
  - Location: `apps/backend/services/agent.service.ts` - `runningAgents` Map
  - Process: Verifies no other agent is running for this task
  - Error: Throws if agent already active
- **Workspace Validation**:
  - Location: `apps/backend/services/workspace.service.ts` - `validateWorkspace()` 
  - Process: Ensures workspace exists and is accessible
  - Recovery: Auto-creates workspace if missing

**5. Initialization Phase**
- **Agent Factory**:
  - Location: `apps/backend/services/agent.service.ts` - `createAgent()` method
  - Process: Instantiates appropriate agent based on type (default: claude-code)
- **Context Setup**:
  - Working directory path from workspace
  - Model configuration (e.g., claude-3-5-sonnet)
  - Session ID for conversation continuity

**6. Background Execution**
- **Location**: `apps/backend/services/agent.service.ts` - `processAgentEvents()` method
- **Process**: Executes agent in background to keep API responsive
- **Error Handling**: Catches and emits errors as events

**7. Event Stream Loop**
- **Message Processing**:
  - Location: `apps/backend/services/agents/claude-code.agent.ts` - `run()` generator
  - Process: Agent analyzes message and determines required actions
- **Tool Execution**:
  - Request: Agent requests tool use (bash, web search, etc.)
  - Validation: Tool permissions checked
  - Execution: Tool performs action in workspace context
  - Result: Tool output returned to agent
- **Event Emission**:
  - Location: `apps/backend/services/event.service.ts` - `emitTaskEvent()`
  - Types: MESSAGE, TOOL_USE, TOOL_RESULT, ERROR
  - Transport: Server-Sent Events (SSE) to frontend
- **UI Updates**:
  - Location: `apps/frontend/src/features/tasks/hooks/use-task-sse-connection.ts`
  - Process: SSE handler updates UI in real-time
  - Display: Messages appear as they stream

**8. Completion**
- **Final Result**:
  - Location: `apps/backend/services/agents/claude-code.agent.ts`
  - Content: Summary, cost metrics, session ID
- **Cleanup**:
  - Location: `apps/backend/services/agent.service.ts`
  - Actions: Remove from running agents map, abort controller cleanup
- **UI Finalization**:
  - Status update to "completed"
  - Enable input for next message
  - Show cost and token usage

### Tool Execution Architecture

Tools extend agent capabilities by providing controlled access to external resources:

```mermaid
graph LR
    subgraph "Tool Categories"
        subgraph "Code Tools"
            BASH[Bash Executor]
            GIT[Git Operations]
            FILE[File System]
        end
        
        subgraph "Research Tools"
            WEB[Web Fetch]
            SEARCH[Web Search]
            DOC[Documentation]
        end
        
        subgraph "Analysis Tools"
            CODE[Code Analysis]
            SEC[Security Scan]
            PERF[Performance]
        end
    end
    
    subgraph "Tool Execution Pipeline"
        REQ[Tool Request]
        VAL[Validation]
        PERM[Permission Check]
        EXEC[Execution]
        LOG[Logging]
        RES[Result]
    end
    
    REQ --> VAL
    VAL --> PERM
    PERM --> EXEC
    EXEC --> LOG
    LOG --> RES
    
    EXEC -.-> BASH
    EXEC -.-> WEB
    EXEC -.-> CODE
    
    style PERM fill:#f99,stroke:#333,stroke-width:2px
    style EXEC fill:#9f9,stroke:#333,stroke-width:2px
```

#### Tool Execution Pipeline Details

**1. Tool Request Phase**
- **Origin**: Agent determines it needs to execute a tool based on user request
- **Format**: Tool request includes tool name, parameters, and context
- **Example**: `{ tool: "Bash", params: { command: "npm test" } }`

**2. Validation Phase**
- **Location**: Tool-specific validators in Claude Code SDK
- **Checks**:
  - Parameter format and types
  - Command syntax validation
  - Path existence verification
  - Input sanitization

**3. Permission Check Phase**
- **Location**: `apps/backend/services/agents/claude-code.agent.ts` - `sdkOptions.allowedTools`
- **Process**:
  - Verify tool is in allowed list for agent
  - Check user permissions for tool category
  - Validate scope of requested operation
- **Default Allowed Tools**: WebFetch, WebSearch, Bash

**4. Execution Phase**
- **Bash Executor**:
  - Location: Handled by Claude Code SDK
  - Context: Executes in task workspace directory
  - Security: Sandboxed to workspace path
  - Timeout: Configurable per command
- **Web Fetch**:
  - Location: Claude Code SDK internal
  - Features: HTML to markdown conversion
  - Limits: Content length restrictions
- **Web Search**:
  - Location: Claude Code SDK internal  
  - Provider: Integrated search API
  - Filtering: Domain allow/block lists

**5. Logging Phase**
- **Location**: `apps/backend/services/event.service.ts`
- **Events Emitted**:
  - TOOL_USE: When tool execution starts
  - TOOL_RESULT: When tool completes
  - TOOL_ERROR: On tool failure
- **Audit Trail**: All tool uses logged with timestamp, parameters, and results

**6. Result Phase**
- **Format**: Structured result object with output, error status, execution time
- **Delivery**: Results returned to agent for processing
- **Integration**: Agent incorporates results into response generation

## Backend Implementation

### Agent Lifecycle Management

The backend manages agent lifecycles through a sophisticated state machine that ensures proper resource management and error recovery:

```mermaid
stateDiagram-v2
    [*] --> Idle
    Idle --> Initializing: executeAgent()
    Initializing --> Validating: Agent created
    Validating --> Running: Workspace valid
    Running --> Processing: Message received
    Processing --> Running: Event emitted
    Running --> Completing: Execution done
    Completing --> Idle: Cleanup done
    
    Validating --> Error: Validation failed
    Running --> Stopping: stop() called
    Processing --> Error: Tool failed
    Error --> Idle: Error handled
    Stopping --> Idle: Stopped
    
    state Running {
        [*] --> StreamingMessage
        StreamingMessage --> ExecutingTool: Tool request
        ExecutingTool --> StreamingMessage: Tool complete
        StreamingMessage --> [*]: Message complete
    }
```

#### Lifecycle State Transitions

**1. Idle State**
- **Description**: No agent is active for the task
- **Location**: `apps/backend/services/agent.service.ts` - `runningAgents` Map is empty for task
- **Entry Actions**: None
- **Exit Actions**: Register new agent in running agents map

**2. Initializing State**
- **Trigger**: `executeAgent()` method called
- **Location**: `apps/backend/services/agent.service.ts` - `executeAgent()` method
- **Actions**:
  - Validate no concurrent agent running
  - Retrieve task and workspace information
  - Create agent instance via factory

**3. Validating State**
- **Location**: `apps/backend/services/workspace.service.ts` - `validateWorkspace()` method
- **Checks**:
  - Workspace directory exists
  - File system permissions valid
  - Git repository initialized (if needed)
- **Recovery**: Auto-create missing workspace

**4. Running State**
- **Location**: `apps/backend/services/agent.service.ts` - `processAgentEvents()` method
- **Characteristics**:
  - Background execution active
  - Event stream open
  - Agent generator yielding events
- **Sub-states**:
  - **StreamingMessage**: Agent generating response text
  - **ExecutingTool**: Tool operation in progress

**5. Processing State**
- **Location**: `apps/backend/services/agents/claude-code.agent.ts` - `run()` generator
- **Actions**:
  - Parse user message
  - Determine required tools
  - Generate response chunks
  - Emit events for each chunk

**6. Completing State**
- **Trigger**: Agent generator completes
- **Actions**:
  - Emit final result event
  - Calculate usage metrics
  - Store completion status

**7. Error State**
- **Triggers**:
  - Validation failures
  - Tool execution errors
  - Network timeouts
  - SDK errors
- **Location**: Error handling in `processAgentEvents()`
- **Recovery**: Emit error event, cleanup resources

**8. Stopping State**
- **Trigger**: `stopAgent()` method called
- **Location**: `apps/backend/services/agent.service.ts` - `stopAgent()` method
- **Actions**:
  - Call agent's `stop()` method
  - Abort any active HTTP requests
  - Clean up from running agents map

### Concurrent Execution Prevention

The system enforces single-agent-per-task to prevent race conditions:

```mermaid
graph TB
    subgraph "Request Flow"
        R1[Request 1: Execute Agent]
        R2[Request 2: Execute Agent]
        R3[Request 3: Stop Agent]
    end
    
    subgraph "Agent Service"
        MAP[Running Agents Map]
        CHECK{Agent Running?}
        CREATE[Create Agent]
        REJECT[Reject Request]
        STOP[Stop Agent]
    end
    
    R1 --> CHECK
    CHECK -->|No| CREATE
    CHECK -->|Yes| REJECT
    CREATE --> MAP
    
    R2 --> CHECK
    R3 --> STOP
    STOP --> MAP
    
    MAP -->|taskId -> agent| CHECK
    
    style REJECT fill:#f99,stroke:#333,stroke-width:2px
    style CREATE fill:#9f9,stroke:#333,stroke-width:2px
```

#### Concurrency Control Implementation

**1. Running Agents Map**
- **Location**: `apps/backend/services/agent.service.ts` - `private runningAgents: Map<string, BaseAgent>`
- **Structure**: Maps task ID to active agent instance
- **Purpose**: Tracks which tasks have agents currently executing

**2. Request Processing Flow**

**Request 1 - First Agent Execution**:
- **Check**: `runningAgents.has(taskId)` returns false
- **Action**: Create new agent and add to map
- **Result**: Agent begins execution

**Request 2 - Concurrent Attempt**:
- **Check**: `runningAgents.has(taskId)` returns true
- **Action**: Throw error "Agent already running for task"
- **Result**: Request rejected, user notified

**Request 3 - Stop Request**:
- **Action**: Retrieve agent from map
- **Process**:
  - Call `agent.stop()` to set stop flag
  - Call `abortController.abort()` to cancel HTTP requests
  - Remove from running agents map
- **Result**: Agent stops gracefully

**3. Implementation Details**

```typescript
// Location: apps/backend/services/agent.service.ts

private runningAgents = new Map<string, BaseAgent>();

async executeAgent(taskId: string, ...): Promise<void> {
  // Concurrency check
  if (this.runningAgents.has(taskId)) {
    throw new Error('Agent already running for this task');
  }
  
  // Create and register
  const agent = this.createAgent(...);
  this.runningAgents.set(taskId, agent);
  
  // Clean up on completion
  try {
    await this.processAgentEvents(agent, ...);
  } finally {
    this.runningAgents.delete(taskId);
  }
}
```

**4. Race Condition Prevention**
- **Synchronous Check**: Map check and insertion are synchronous
- **No Await Between**: No async operations between check and set
- **Guaranteed Cleanup**: Finally block ensures removal even on error

#### Claude Code Agent
```typescript
// apps/backend/services/agents/claude-code.agent.ts
export class ClaudeCodeAgent extends BaseAgent {
  private abortController: AbortController | null = null
  
  getAgentType(): string {
    return 'claude-code'
  }
  
  async *run(
    message: string,
    options: AgentRunOptions
  ): AsyncGenerator<AgentEvent> {
    this.currentIteration++
    this.abortController = new AbortController()
    
    try {
      // Configure SDK
      const sdkOptions: ClaudeCodeOptions = {
        allowedTools: ['WebFetch', 'WebSearch', 'Bash'],
        permissionMode: 'acceptEdits',
        cwd: options.workingDirectory,
        model: options.model || 'claude-3-5-sonnet-20241022',
        resume: options.resumeSessionId,
        abortSignal: this.abortController.signal
      }
      
      // Execute query
      const { messages, metadata } = await query(
        message,
        options.attachments,
        sdkOptions
      )
      
      // Capture session ID
      this.sessionId = metadata.sessionId
      
      // Process messages
      for (const msg of messages) {
        if (this.stopRequested) break
        
        yield this.createEvent('message', {
          role: msg.role,
          content: msg.content,
          tool_use: msg.tool_use
        })
      }
      
      // Final result
      yield this.createEvent('result', {
        status: 'completed',
        cost: metadata.cost,
        sessionId: this.sessionId,
        resultText: this.extractResultText(messages)
      })
      
    } catch (error) {
      yield this.createEvent('error', {
        message: error.message,
        stack: error.stack
      })
    }
  }
  
  stop(): void {
    super.stop()
    this.abortController?.abort()
  }
}
```

#### Key Methods
| Method | Purpose | Parameters | Returns |
|--------|---------|------------|---------|
| `executeAgent` | Run agent for task | taskId, message, options | void (async) |
| `stopAgent` | Stop running agent | taskId | void |
| `createAgent` | Factory for agents | type, config | BaseAgent |
| `processAgentEvents` | Handle event stream | agent, taskId, iterationId | void |

### Message Processing

#### Message Flow
```typescript
async executeAgent(
  taskId: string,
  iterationId: string,
  message: string,
  options?: ExecuteAgentOptions
): Promise<void> {
  // 1. Validate workspace
  const workspace = await this.validateWorkspace(taskId)
  
  // 2. Check for running agent
  if (this.runningAgents.has(taskId)) {
    throw new Error('Agent already running for this task')
  }
  
  // 3. Create agent instance
  const agent = this.createAgent(options?.agentType || 'claude-code', {
    workingDirectory: workspace.path,
    model: options?.model,
    resumeSessionId: options?.resumeSessionId
  })
  
  // 4. Register agent
  this.runningAgents.set(taskId, agent)
  
  // 5. Execute in background
  this.processAgentEvents(agent, taskId, iterationId)
    .catch(error => {
      console.error('Agent execution failed:', error)
      this.eventService.emitTaskEvent({
        taskId,
        iterationId,
        type: EventType.ERROR,
        payload: { error: error.message }
      })
    })
    .finally(() => {
      this.runningAgents.delete(taskId)
    })
}
```

#### Event Processing
```typescript
private async processAgentEvents(
  agent: BaseAgent,
  taskId: string,
  iterationId: string
): Promise<void> {
  try {
    for await (const event of agent.run(message, options)) {
      // Transform agent event to task event
      const taskEvent = this.createTaskEvent(taskId, iterationId, event)
      
      // Emit via event service
      await this.eventService.emitTaskEvent(taskEvent)
      
      // Handle special events
      if (event.type === 'result') {
        await this.handleResultEvent(taskId, iterationId, event)
      }
    }
  } catch (error) {
    if (error.code === 'ELIFECYCLE' && [143, 130].includes(error.exitCode)) {
      // Expected exit codes for stop requests
      console.log('Agent stopped gracefully')
    } else {
      throw error
    }
  }
}
```

### Tool Execution Framework

#### Available Tools
```typescript
interface ToolConfig {
  WebFetch: {
    enabled: boolean
    maxContentLength?: number
    timeout?: number
  }
  WebSearch: {
    enabled: boolean
    maxResults?: number
    blockedDomains?: string[]
  }
  Bash: {
    enabled: boolean
    allowedCommands?: string[]
    timeout?: number
  }
}
```

#### Tool Security
- **Bash**: Executes only within workspace directory
- **WebFetch**: Respects content length limits
- **WebSearch**: Can block specific domains
- All tools respect agent permissions

### Error Recovery

#### Workspace Recovery
```typescript
private async validateWorkspace(taskId: string): Promise<WorkspaceInfo> {
  const workspace = await this.workspaceService.getWorkspace(taskId)
  
  // Check if workspace exists
  if (!workspace || !await fs.pathExists(workspace.path)) {
    // Recreate workspace
    console.log('Workspace missing, recreating...')
    await this.workspaceService.createWorkspace(taskId)
    return await this.workspaceService.getWorkspace(taskId)
  }
  
  return workspace
}
```

#### Error Event Handling
```typescript
private handleAgentError(error: Error, taskId: string): AgentEvent {
  return {
    type: 'error',
    data: {
      message: error.message,
      code: error.code,
      recoverable: this.isRecoverableError(error)
    },
    timestamp: new Date().toISOString()
  }
}
```

### API Endpoints

#### Route Definitions
| Method | Path | Purpose | Auth Required |
|--------|------|---------|---------------|
| GET | `/api/v1/agents` | List available agents | No |
| GET | `/api/v1/agents/:type` | Get agent details | No |
| POST | `/api/tasks/:taskId/messages` | Send message to agent | No |
| POST | `/api/tasks/:taskId/stop` | Stop running agent | No |

#### Request/Response Schemas
```typescript
// Send message request
interface SendMessageRequest {
  content: string
  attachments?: FileAttachment[]
  model?: string
  resumeSessionId?: string
}

// Agent info response
interface AgentInfoResponse {
  type: string
  displayName: string
  description: string
  supportedModels: string[]
  availableTools: string[]
  defaultModel: string
}
```

## Frontend Implementation

### Agent Integration Architecture

The frontend integrates agents through the task system, providing a seamless chat-based interface for AI interactions:

```mermaid
graph TB
    subgraph "User Interface Layer"
        CHAT[Chat Input]
        VIEWER[Message Viewer]
        STATUS[Status Indicator]
    end
    
    subgraph "Task Context Layer"
        TC[Task Context Provider]
        TH[Task Hooks]
        IM[Iteration Manager]
    end
    
    subgraph "Agent Communication Layer"
        MSG[Message Sender]
        SSE[SSE Handler]
        ERR[Error Handler]
    end
    
    subgraph "State Management"
        RQ[React Query Cache]
        LS[Local State]
        OPT[Optimistic Updates]
    end
    
    CHAT -->|Submit| MSG
    MSG -->|Mutation| RQ
    RQ -->|API Call| TC
    
    SSE -->|Events| VIEWER
    SSE -->|Updates| STATUS
    SSE -->|Cache| RQ
    
    TC -->|Provide Data| TH
    TH -->|Consume| IM
    
    ERR -->|Fallback| STATUS
    OPT -->|Immediate| VIEWER
    
    style TC fill:#f9f,stroke:#333,stroke-width:2px
    style SSE fill:#9ff,stroke:#333,stroke-width:2px
```

#### Frontend Architecture Components

**1. User Interface Layer**
- **Chat Input** (`apps/frontend/src/features/tasks/components/chat-input.tsx`):
  - Captures user messages
  - Handles file attachments
  - Manages input state and validation
  - Disables during agent execution
  
- **Message Viewer** (`apps/frontend/src/features/tasks/components/task-viewer.tsx`):
  - Displays conversation history
  - Renders markdown content
  - Shows tool executions
  - Streams responses in real-time
  
- **Status Indicator** (`apps/frontend/src/features/tasks/components/task-header.tsx`):
  - Shows agent execution state
  - Displays cost and token usage
  - Indicates active/completed status

**2. Task Context Layer**
- **Task Context Provider** (`apps/frontend/src/features/tasks/contexts/task-context.tsx`):
  - Central data provider for task components
  - Manages task metadata, iterations, and events
  - Coordinates SSE connections
  - Handles state synchronization
  
- **Task Hooks** (`apps/frontend/src/features/tasks/hooks/`):
  - `useTask()`: Fetches task metadata
  - `useTaskEvents()`: Manages event stream
  - `useCreateIteration()`: Handles message submission
  - `useStopTask()`: Cancels agent execution
  
- **Iteration Manager**:
  - Tracks conversation history
  - Manages selected iteration state
  - Handles iteration navigation

**3. Agent Communication Layer**
- **Message Sender** (`apps/frontend/src/features/tasks/hooks/use-create-iteration.ts`):
  - React Query mutation for message submission
  - Handles optimistic updates
  - Manages loading states
  
- **SSE Handler** (`apps/frontend/src/features/tasks/hooks/use-task-sse-connection.ts`):
  - Establishes EventSource connection
  - Processes incoming events
  - Updates React Query cache
  - Handles reconnection logic
  
- **Error Handler**:
  - Catches API errors
  - Displays user-friendly messages
  - Provides retry mechanisms

**4. State Management**
- **React Query Cache**:
  - Caches task data, iterations, events
  - Manages cache invalidation
  - Handles background refetching
  
- **Local State**:
  - UI-only state (selected panels, filters)
  - Form state management
  - Temporary display states
  
- **Optimistic Updates**:
  - Immediate UI feedback on submission
  - Rollback on error
  - Smooth user experience

### Real-time Event Processing

The frontend processes agent events through a sophisticated streaming architecture:

```mermaid
sequenceDiagram
    participant UI as UI Component
    participant SSE as SSE Connection
    participant EH as Event Handler
    participant RQ as React Query
    participant DOM as DOM Updates
    
    Note over SSE: SSE Connection Established
    
    loop Event Stream
        SSE->>EH: Receive Event
        
        alt Message Event
            EH->>RQ: Update Message Cache
            RQ->>UI: Trigger Re-render
            UI->>DOM: Update Chat Display
        else Result Event
            EH->>RQ: Invalidate Queries
            EH->>UI: Show Completion
            UI->>DOM: Update Status
        else Error Event
            EH->>UI: Show Error
            UI->>DOM: Display Error UI
        end
    end
    
    Note over SSE: Connection Closed
```

#### Event Processing Implementation

**1. SSE Connection Setup**
- **Location**: `apps/frontend/src/features/tasks/hooks/use-task-sse-connection.ts`
- **Process**:
  ```typescript
  const eventSource = new EventSource(`/api/v1/tasks/${taskId}/events`);
  eventSource.onmessage = handleEvent;
  eventSource.onerror = handleError;
  ```
- **Features**:
  - Auto-reconnection on disconnect
  - Connection health monitoring
  - Cleanup on unmount

**2. Event Types and Handlers**

**Message Events**:
- **Type**: `EVENT_MESSAGE`
- **Handler Location**: `apps/frontend/src/features/tasks/utils/event-handlers.ts`
- **Processing**:
  - Parse message content
  - Update iteration's message array
  - Trigger UI re-render
  - Scroll to latest content

**Tool Use Events**:
- **Type**: `TOOL_USE`
- **Display**: Show tool indicator with spinning icon
- **Content**: Tool name, parameters, execution status

**Result Events**:
- **Type**: `EVENT_RESULT`
- **Actions**:
  - Mark iteration as complete
  - Update cost metrics
  - Enable chat input
  - Invalidate related queries

**Error Events**:
- **Type**: `EVENT_ERROR`
- **Display**: Error alert with retry option
- **Logging**: Console error for debugging

**3. Cache Update Strategy**
- **Location**: `apps/frontend/src/lib/api/query-client.ts`
- **Method**: Direct cache manipulation for real-time updates
- **Pattern**:
  ```typescript
  queryClient.setQueryData(
    queryKeys.taskEvents(taskId),
    (old) => [...old, newEvent]
  );
  ```

**4. UI Update Flow**
- **Message Streaming**:
  - Append text chunks to existing message
  - Maintain cursor position
  - Preserve formatting
- **Status Updates**:
  - Update task header status
  - Show/hide loading indicators
  - Update progress bars
- **Error Handling**:
  - Display inline error messages
  - Maintain conversation context
  - Allow retry from last point

## Data Flow

### End-to-End Message Processing

The complete data flow from user input to AI response demonstrates the system's sophisticated orchestration:

```mermaid
graph LR
    subgraph "User Journey"
        U1[User Types Message]
        U2[Sees Typing Indicator]
        U3[Reads AI Response]
        U4[Sees Code Changes]
    end
    
    subgraph "Frontend Flow"
        F1[Capture Input]
        F2[Optimistic Update]
        F3[API Mutation]
        F4[SSE Handler]
        F5[UI Update]
    end
    
    subgraph "Backend Flow"
        B1[Validate Request]
        B2[Queue Execution]
        B3[Agent Processing]
        B4[Tool Execution]
        B5[Event Emission]
    end
    
    subgraph "Agent Flow"
        A1[Context Loading]
        A2[Message Analysis]
        A3[Tool Selection]
        A4[Response Generation]
        A5[Result Compilation]
    end
    
    U1 --> F1
    F1 --> F2
    F2 --> U2
    F3 --> B1
    B1 --> B2
    B2 --> B3
    B3 --> A1
    A1 --> A2
    A2 --> A3
    A3 --> B4
    B4 --> A4
    A4 --> B5
    B5 --> F4
    F4 --> F5
    F5 --> U3
    A5 --> U4
    
    style U2 fill:#ff9,stroke:#333,stroke-width:2px
    style B3 fill:#9f9,stroke:#333,stroke-width:2px
    style A3 fill:#99f,stroke:#333,stroke-width:2px
```

#### Detailed Flow Steps

**1. User Journey**
- **U1 - User Types Message**:
  - Location: Chat interface in task detail view
  - Component: `apps/frontend/src/features/tasks/components/chat-input.tsx`
  - User enters request like "Add error handling to the login function"

- **U2 - Sees Typing Indicator**:
  - Immediate feedback via optimistic update
  - Message appears in UI with "pending" status
  - Input disabled, loading spinner shown

- **U3 - Reads AI Response**:
  - Response streams in real-time
  - Markdown formatted with syntax highlighting
  - Tool executions shown inline

- **U4 - Sees Code Changes**:
  - File modifications displayed in diff viewer
  - Updated workspace reflected in UI
  - Git status shows changes

**2. Frontend Flow**
- **F1 - Capture Input**:
  - Location: `apps/frontend/src/features/tasks/components/chat-input.tsx`
  - Validates non-empty message
  - Prepares mutation payload

- **F2 - Optimistic Update**:
  - Location: `apps/frontend/src/features/tasks/hooks/use-create-iteration.ts`
  - Adds message to UI immediately
  - Sets iteration status to "active"

- **F3 - API Mutation**:
  - Endpoint: `POST /api/v1/tasks/{taskId}/iterations`
  - Payload: `{ content: string, attachments?: File[] }`
  - Handled by React Query

- **F4 - SSE Handler**:
  - Location: `apps/frontend/src/features/tasks/hooks/use-task-sse-connection.ts`
  - Processes streaming events
  - Updates React Query cache

- **F5 - UI Update**:
  - Re-renders affected components
  - Scrolls to new content
  - Updates status indicators

**3. Backend Flow**
- **B1 - Validate Request**:
  - Location: `apps/backend/routes/task.routes.ts`
  - Checks task exists and is active
  - Validates message format

- **B2 - Queue Execution**:
  - Location: `apps/backend/services/agent.service.ts`
  - Checks no agent already running
  - Creates background execution task

- **B3 - Agent Processing**:
  - Creates agent instance
  - Initializes with workspace context
  - Begins event generation

- **B4 - Tool Execution**:
  - Agent requests tool use
  - Tools execute in workspace
  - Results returned to agent

- **B5 - Event Emission**:
  - Location: `apps/backend/services/event.service.ts`
  - Broadcasts via SSE
  - Persists to event store

**4. Agent Flow**
- **A1 - Context Loading**:
  - Loads workspace information
  - Retrieves conversation history
  - Sets up execution environment

- **A2 - Message Analysis**:
  - Parses user intent
  - Identifies required actions
  - Plans execution strategy

- **A3 - Tool Selection**:
  - Determines necessary tools
  - Prepares tool parameters
  - Validates permissions

- **A4 - Response Generation**:
  - Formulates natural language response
  - Incorporates tool results
  - Streams output chunks

- **A5 - Result Compilation**:
  - Summarizes actions taken
  - Calculates usage metrics
  - Prepares final state

### Session Management Architecture

The system maintains conversation continuity across multiple interactions:

```mermaid
stateDiagram-v2
    [*] --> NewSession: First Message
    NewSession --> ActiveSession: Session Created
    ActiveSession --> Processing: New Message
    Processing --> ActiveSession: Response Complete
    ActiveSession --> Paused: User Idle
    Paused --> ActiveSession: Resume Message
    ActiveSession --> Completed: Task Done
    Completed --> [*]
    
    state ActiveSession {
        [*] --> WaitingForUser
        WaitingForUser --> ProcessingMessage: User Input
        ProcessingMessage --> StreamingResponse: Agent Active
        StreamingResponse --> WaitingForUser: Complete
    }
    
    state Processing {
        [*] --> LoadContext
        LoadContext --> ExecuteTools
        ExecuteTools --> GenerateResponse
        GenerateResponse --> [*]
    }
```

### Error Recovery Flow

The system handles errors gracefully at multiple levels:

```mermaid
graph TB
    subgraph "Error Detection"
        E1[API Error]
        E2[Agent Error]
        E3[Tool Error]
        E4[Network Error]
    end
    
    subgraph "Error Handling"
        H1[Retry Logic]
        H2[Fallback Response]
        H3[User Notification]
        H4[State Recovery]
    end
    
    subgraph "Recovery Actions"
        R1[Reconnect SSE]
        R2[Restore Session]
        R3[Replay Events]
        R4[Update UI]
    end
    
    E1 --> H1
    E2 --> H2
    E3 --> H2
    E4 --> H1
    
    H1 --> R1
    H1 --> R2
    H2 --> H3
    H3 --> R4
    H4 --> R3
    
    style H1 fill:#f99,stroke:#333,stroke-width:2px
    style H3 fill:#ff9,stroke:#333,stroke-width:2px
    style R4 fill:#9f9,stroke:#333,stroke-width:2px
```

## Integration Points

### System Integration Architecture

The Agent system integrates with multiple components to provide a cohesive AI experience:

```mermaid
graph TB
    subgraph "Agent Core"
        AGENT[Agent System]
    end
    
    subgraph "Direct Dependencies"
        TASK[Task Service]
        WS[Workspace Service]
        EVENT[Event Service]
        STORE[Storage Service]
    end
    
    subgraph "Indirect Dependencies"
        REPO[Repository Service]
        ENV[Environment Service]
        GIT[Git Service]
    end
    
    subgraph "External Systems"
        SDK[Claude Code SDK]
        AI[AI APIs]
        WEB[Web Services]
    end
    
    AGENT <--> TASK
    AGENT <--> WS
    AGENT <--> EVENT
    AGENT <--> STORE
    
    WS <--> REPO
    WS <--> ENV
    WS <--> GIT
    
    AGENT --> SDK
    SDK --> AI
    AGENT --> WEB
    
    style AGENT fill:#f9f,stroke:#333,stroke-width:4px
    style TASK fill:#9ff,stroke:#333,stroke-width:2px
    style WS fill:#9ff,stroke:#333,stroke-width:2px
```

#### Integration Details

**1. Direct Dependencies**

**Task Service Integration**:
- **Location**: `apps/backend/services/task.service.ts`
- **Purpose**: Manages task lifecycle and iterations
- **Interactions**:
  - Agent retrieves task metadata
  - Creates new iterations for messages
  - Updates task status on completion
  - Stores result summaries

**Workspace Service Integration**:
- **Location**: `apps/backend/services/workspace.service.ts`
- **Purpose**: Provides isolated execution environment
- **Interactions**:
  - Validates workspace exists before execution
  - Provides working directory path
  - Manages file system operations
  - Handles workspace cleanup

**Event Service Integration**:
- **Location**: `apps/backend/services/event.service.ts`
- **Purpose**: Real-time event broadcasting
- **Interactions**:
  - Emits agent events (messages, tools, results)
  - Manages SSE connections
  - Persists events to storage
  - Handles event replay

**Storage Service Integration**:
- **Location**: `apps/backend/services/storage.service.ts`
- **Purpose**: Persistent data storage
- **Interactions**:
  - Stores iteration messages
  - Persists agent results
  - Saves cost metrics
  - Archives conversation history

**2. Indirect Dependencies**

**Repository Service**:
- **Location**: `apps/backend/services/repository.service.ts`
- **Access**: Via Workspace Service
- **Purpose**: Repository configuration for workspaces

**Environment Service**:
- **Location**: `apps/backend/services/environment.service.ts`
- **Access**: Via Workspace Service
- **Purpose**: Environment setup for agent execution

**Git Service**:
- **Location**: `apps/backend/services/git.service.ts`
- **Access**: Via Workspace Service
- **Purpose**: Version control operations in workspace

**3. External Systems**

**Claude Code SDK**:
- **Location**: `@anthropic/claude-code-sdk`
- **Purpose**: Core AI capabilities
- **Features**:
  - Message processing
  - Tool execution
  - Response generation
  - Session management

**AI APIs**:
- **Access**: Through Claude Code SDK
- **Purpose**: LLM inference
- **Models**: Claude 3.5 Sonnet, etc.

**Web Services**:
- **Purpose**: External data access
- **Tools**:
  - Web fetch for content retrieval
  - Web search for information gathering
  - API integrations

### Event-Driven Communication

The agent system communicates through a well-defined event protocol:

```mermaid
sequenceDiagram
    participant A as Agent
    participant ES as Event Service
    participant C as Client
    participant Q as Query Cache
    
    Note over A,Q: Message Processing
    A->>ES: MESSAGE event
    ES->>C: SSE: Message chunk
    C->>C: Append to display
    
    Note over A,Q: Tool Execution
    A->>ES: TOOL_USE event
    ES->>C: SSE: Tool started
    C->>C: Show tool indicator
    
    A->>ES: TOOL_RESULT event
    ES->>C: SSE: Tool complete
    C->>C: Update UI
    
    Note over A,Q: Completion
    A->>ES: RESULT event
    ES->>C: SSE: Execution complete
    C->>Q: Invalidate queries
    Q->>C: Refresh data
```

#### Event Protocol Details

**1. Message Events**
- **Type**: `EVENT_MESSAGE`
- **Emitter**: `apps/backend/services/agents/claude-code.agent.ts`
- **Payload**:
  ```typescript
  {
    type: 'EVENT_MESSAGE',
    subtype: 'assistant',
    payload: {
      content: string,
      role: 'assistant' | 'user',
      tool_use?: ToolUse[]
    }
  }
  ```
- **Client Handler**: `apps/frontend/src/features/tasks/utils/event-handlers.ts`
- **UI Effect**: Appends message chunk to conversation display

**2. Tool Events**
- **TOOL_USE Event**:
  - Type: `TOOL_USE`
  - Indicates tool execution started
  - Payload: Tool name, parameters
  - UI: Shows spinning indicator with tool name

- **TOOL_RESULT Event**:
  - Type: `TOOL_RESULT`
  - Contains tool execution output
  - Payload: Result data, execution time
  - UI: Updates tool card with results

**3. Result Event**
- **Type**: `EVENT_RESULT`
- **Emitter**: Agent completion handler
- **Payload**:
  ```typescript
  {
    type: 'EVENT_RESULT',
    payload: {
      status: 'completed' | 'failed',
      cost: { input_tokens: number, output_tokens: number },
      sessionId: string,
      resultText: string
    }
  }
  ```
- **Actions**:
  - Updates iteration status
  - Stores cost metrics
  - Enables chat input
  - Triggers cache invalidation

**4. Error Events**
- **Type**: `EVENT_ERROR`
- **Triggers**: Validation failures, tool errors, SDK issues
- **Payload**: Error message, stack trace, recoverable flag
- **UI**: Error alert with context-specific message

**5. Event Flow Implementation**
- **Backend Emission**:
  ```typescript
  // Location: apps/backend/services/agent.service.ts
  await this.eventService.emitTaskEvent({
    taskId,
    iterationId,
    type: event.type,
    subtype: event.subtype,
    payload: event.data,
    timestamp: event.timestamp
  });
  ```

- **Frontend Reception**:
  ```typescript
  // Location: apps/frontend/src/features/tasks/hooks/use-task-sse-connection.ts
  eventSource.onmessage = (event) => {
    const data = JSON.parse(event.data);
    handleTaskEvent(data);
  };
  ```

### Cross-Component Data Flow

The agent system participates in complex data flows across the architecture:

```mermaid
graph LR
    subgraph "Input Sources"
        UI[User Interface]
        API[API Calls]
        SCHED[Scheduled Jobs]
    end
    
    subgraph "Processing Pipeline"
        AGENT[Agent Engine]
        TOOLS[Tool Executor]
        VALID[Validators]
    end
    
    subgraph "Output Targets"
        WS[Workspace Files]
        DB[Database]
        EVENTS[Event Stream]
        LOGS[Audit Logs]
    end
    
    UI --> AGENT
    API --> AGENT
    SCHED --> AGENT
    
    AGENT --> TOOLS
    TOOLS --> VALID
    
    VALID --> WS
    VALID --> DB
    AGENT --> EVENTS
    TOOLS --> LOGS
    
    style AGENT fill:#f9f,stroke:#333,stroke-width:2px
    style TOOLS fill:#9f9,stroke:#333,stroke-width:2px
```

## Best Practices

### Architectural Principles

The agent system follows key architectural principles for reliability and security:

```mermaid
graph TD
    subgraph "Core Principles"
        ISO[Workspace Isolation]
        SRP[Single Responsibility]
        EVT[Event-Driven Design]
        FAL[Fail-Safe Defaults]
    end
    
    subgraph "Implementation Patterns"
        SING[Single Agent per Task]
        STRM[Real-time Streaming]
        ASYNC[Async Execution]
        AUDIT[Audit Logging]
    end
    
    subgraph "Security Measures"
        PERM[Permission Model]
        SAND[Sandboxed Execution]
        VAL[Input Validation]
        LIMIT[Rate Limiting]
    end
    
    ISO --> SING
    ISO --> SAND
    SRP --> ASYNC
    EVT --> STRM
    FAL --> PERM
    FAL --> VAL
    
    SING --> AUDIT
    STRM --> AUDIT
    PERM --> LIMIT
    
    style ISO fill:#f9f,stroke:#333,stroke-width:2px
    style FAL fill:#9ff,stroke:#333,stroke-width:2px
```

### Performance Architecture

The system optimizes performance through careful design choices:

```mermaid
graph LR
    subgraph "Performance Strategies"
        GEN[Generator Pattern]
        CACHE[Smart Caching]
        QUEUE[Request Queuing]
        BGND[Background Jobs]
    end
    
    subgraph "Resource Management"
        POOL[Connection Pooling]
        CLEAN[Auto Cleanup]
        LIMIT[Context Limiting]
        BATCH[Event Batching]
    end
    
    GEN --> POOL
    CACHE --> LIMIT
    QUEUE --> BATCH
    BGND --> CLEAN
    
    style GEN fill:#9f9,stroke:#333,stroke-width:2px
    style BGND fill:#9f9,stroke:#333,stroke-width:2px
```

## Troubleshooting

### Diagnostic Flow

When issues arise, follow this systematic diagnostic approach:

```mermaid
graph TD
    START[Issue Detected]
    
    CHECK1{Agent Running?}
    CHECK2{Workspace Valid?}
    CHECK3{Events Flowing?}
    CHECK4{Tools Working?}
    
    FIX1[Restart Agent Service]
    FIX2[Recreate Workspace]
    FIX3[Check SSE Connection]
    FIX4[Verify Permissions]
    
    RESOLVED[Issue Resolved]
    ESCALATE[Check Logs]
    
    START --> CHECK1
    CHECK1 -->|No| FIX1
    CHECK1 -->|Yes| CHECK2
    CHECK2 -->|No| FIX2
    CHECK2 -->|Yes| CHECK3
    CHECK3 -->|No| FIX3
    CHECK3 -->|Yes| CHECK4
    CHECK4 -->|No| FIX4
    
    FIX1 --> RESOLVED
    FIX2 --> RESOLVED
    FIX3 --> RESOLVED
    FIX4 --> RESOLVED
    
    CHECK4 -->|Yes| ESCALATE
    
    style START fill:#f99,stroke:#333,stroke-width:2px
    style RESOLVED fill:#9f9,stroke:#333,stroke-width:2px
```

#### Diagnostic Steps

**1. Check Agent Running**
- **How to Check**:
  - Look at `apps/backend/services/agent.service.ts` logs
  - Check if `runningAgents` Map contains the task ID
  - Verify agent process is active in system
- **Common Issues**:
  - Agent crashed without cleanup
  - Concurrent execution attempt blocked
  - Agent stuck in processing state
- **Fix**: Restart agent service or manually clear running agents map

**2. Validate Workspace**
- **Location**: `apps/backend/services/workspace.service.ts`
- **Checks**:
  ```bash
  # Check workspace directory exists
  ls -la data/workspaces/[task-id]
  
  # Verify permissions
  ls -la data/workspaces/[task-id]/.git
  ```
- **Common Issues**:
  - Workspace deleted or corrupted
  - Permission issues
  - Git repository not initialized
- **Fix**: Delete workspace directory and let system recreate

**3. Verify Event Flow**
- **Frontend Check**:
  - Browser DevTools > Network > EventStream
  - Look for `/api/v1/tasks/[id]/events` connection
  - Verify events arriving
- **Backend Check**:
  - Check `apps/backend/services/event.service.ts` logs
  - Verify SSE clients connected
- **Common Issues**:
  - SSE connection dropped
  - Proxy/firewall blocking
  - Client timeout
- **Fix**: Refresh page to reconnect SSE

**4. Test Tool Execution**
- **Manual Test**:
  ```typescript
  // In agent code, check allowed tools
  sdkOptions.allowedTools // Should include needed tools
  ```
- **Permission Check**:
  - Verify tool in allowed list
  - Check workspace directory access
  - Validate tool parameters
- **Common Issues**:
  - Tool not in allowed list
  - Workspace path issues
  - Tool timeout
- **Fix**: Update agent configuration or permissions

**5. Check Logs**
- **Backend Logs**:
  ```bash
  # Agent service logs
  grep "agent.service" logs/app.log
  
  # Event service logs  
  grep "event.service" logs/app.log
  
  # Error traces
  grep "ERROR" logs/app.log | grep agent
  ```
- **Frontend Console**:
  - Check for WebSocket errors
  - Look for failed API calls
  - Verify React Query cache state

### System Health Monitoring

Key indicators for agent system health:

```mermaid
graph LR
    subgraph "Health Metrics"
        M1[Active Agents]
        M2[Event Throughput]
        M3[Tool Success Rate]
        M4[Response Time]
    end
    
    subgraph "Warning Signs"
        W1[Agent Timeouts]
        W2[Event Backlog]
        W3[Tool Failures]
        W4[Memory Growth]
    end
    
    subgraph "Critical Issues"
        C1[Service Crash]
        C2[Workspace Corruption]
        C3[SDK Errors]
        C4[Resource Exhaustion]
    end
    
    M1 -.->|Monitor| W1
    M2 -.->|Monitor| W2
    M3 -.->|Monitor| W3
    M4 -.->|Monitor| W4
    
    W1 -->|Escalate| C1
    W2 -->|Escalate| C4
    W3 -->|Escalate| C3
    W4 -->|Escalate| C2
    
    style M1 fill:#9f9,stroke:#333,stroke-width:2px
    style W1 fill:#ff9,stroke:#333,stroke-width:2px
    style C1 fill:#f99,stroke:#333,stroke-width:2px
```

## Related Documentation
- [Task Management Architecture](./tasks.md)
- [Event Architecture](./events.md)
- [Workspace Architecture](./workspaces.md)
- [Claude Code SDK Documentation](https://github.com/anthropics/claude-code)