# Event Architecture

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
The Event system is the real-time communication backbone of Moatless Swarm UI, enabling instant feedback as AI agents modify code. It uses Server-Sent Events (SSE) to stream updates from backend services to frontend components, ensuring users see live progress without polling.

### Key Responsibilities
- Stream real-time updates via SSE with automatic reconnection
- Manage dual-stream architecture (global and task-specific)
- Orchestrate cache invalidation for UI consistency
- Buffer events for reliable delivery during disconnections
- Track connection health with ping/pong heartbeats
- Filter and route events to appropriate consumers
- Provide debugging and monitoring capabilities
- Ensure ordered event delivery with sequence numbers

### Position in Architecture
The Event system bridges the gap between backend actions and frontend reactions. It transforms discrete service operations into continuous streams of updates, enabling the reactive UI patterns that make Moatless feel responsive and alive.

## Core Concepts

### Domain Terminology

| Term | Definition |
|------|------------|
| Event | A discrete notification about a state change or action |
| SSE | Server-Sent Events - unidirectional real-time communication protocol |
| Event Stream | Continuous connection for receiving events |
| Global Stream | Application-wide event stream for all events |
| Task Stream | Task-specific event stream for focused updates |
| Event Type | Category of event (e.g., MESSAGE, RESULT, ERROR) |
| Event Subtype | Specific variant within an event type |
| NDJSON | Newline-delimited JSON format used for streaming |
| Connection ID | Unique identifier for each SSE connection |
| Sequence Number | Ordered counter for event ordering |
| Event Bus | Central dispatcher for event routing |
| Heartbeat | Periodic ping to maintain connection health |

### Key Abstractions

#### Event Hierarchy
```typescript
interface BaseEvent {
  eventId: string
  timestamp: Date
  sequenceNumber: number
}

interface TaskEvent extends BaseEvent {
  taskId: string
  iterationId?: string
  agentType?: string
}

interface SystemEvent extends BaseEvent {
  source: string
  severity: 'info' | 'warning' | 'error'
}
```

### Design Decisions

- **Dual-Stream Architecture**: Separate global and task streams for scalability
- **NDJSON over SSE**: Simpler parsing, better proxy compatibility
- **Event Buffering**: In-memory cache for reconnection recovery
- **Selective Persistence**: Store only business-critical events
- **Automatic Enrichment**: Add metadata at emission time
- **Connection Multiplexing**: Single connection for multiple subscriptions
- **Graceful Degradation**: Fall back to polling if SSE fails

## Architecture

### High-Level Overview

The Event system orchestrates real-time communication through a sophisticated multi-layered architecture that ensures reliable, ordered delivery of updates across the application.

#### Event Processing Flow

The event system processes updates through several distinct stages, each implemented in specific modules:

1. **Event Generation** (Backend Services)
   - **Agent Service** (`apps/backend/services/agents/`) - Emits events when AI agents perform actions like code generation, file modifications, or tool executions
   - **Task Service** (`apps/backend/services/task.service.ts`) - Generates events for task lifecycle changes, status updates, and iteration results
   - **Workspace Service** (`apps/backend/services/workspace.service.ts`) - Produces events when workspace files change, git operations occur, or server states update
   - **Preview Service** (`apps/backend/services/preview.service.ts`) - Creates events for preview server lifecycle, hot reloads, and build status

2. **Event Processing Core** (`apps/backend/services/event.service.ts`)
   - **Event Bus** - Central dispatcher that receives all events and manages distribution
   - **Event Enricher** - Adds metadata like timestamps, sequence numbers, and correlation IDs
   - **Event Filter** - Applies routing rules and permission checks before delivery
   - **Event Cache** - Maintains recent events for recovery and debugging

3. **Delivery Infrastructure** (`apps/backend/utils/sse-manager.ts`)
   - **SSE Manager** - Manages Server-Sent Event connections and handles streaming protocol
   - **Connection Manager** - Tracks active connections, manages lifecycles, and handles cleanup
   - **Health Monitor** - Sends periodic heartbeats and detects stale connections

4. **Frontend Reception** (`apps/frontend/src/features/tasks/hooks/`)
   - **Event Parser** (`use-task-sse-connection.ts`) - Parses NDJSON format and validates event structure
   - **Event Router** - Distributes events to appropriate subscribers based on type and filters
   - **Event Handlers** - Process events and trigger UI updates

5. **UI Integration**
   - **React Query** (`apps/frontend/src/lib/api/query-client.ts`) - Invalidates caches based on event types
   - **State Updates** - Direct component state modifications for real-time feedback
   - **UI Components** - Final rendering of updated data in the interface

```mermaid
graph TB
    subgraph "Event Sources"
        AS[Agent Service]
        TS[Task Service]
        WS[Workspace Service]
        PS[Preview Service]
    end
    
    subgraph "Event Processing Core"
        EB[Event Bus]
        EE[Event Enricher]
        EF[Event Filter]
        EC[Event Cache]
    end
    
    subgraph "Delivery Infrastructure"
        SM[SSE Manager]
        CM[Connection Manager]
        HM[Health Monitor]
    end
    
    subgraph "Stream Types"
        GS[Global Stream]
        TS2[Task Streams]
        AS2[Admin Stream]
    end
    
    subgraph "Frontend Reception"
        EP[Event Parser]
        ER[Event Router]
        EH[Event Handlers]
    end
    
    subgraph "UI Updates"
        RQ[React Query]
        ST[State Updates]
        UI[UI Components]
    end
    
    AS --> EB
    TS --> EB
    WS --> EB
    PS --> EB
    
    EB --> EE
    EE --> EF
    EF --> EC
    EC --> SM
    
    SM --> CM
    CM --> GS
    CM --> TS2
    CM --> AS2
    
    HM -.->|Monitor| CM
    
    GS --> EP
    TS2 --> EP
    
    EP --> ER
    ER --> EH
    
    EH --> RQ
    EH --> ST
    RQ --> UI
    ST --> UI
    
    style EB fill:#f9f,stroke:#333,stroke-width:4px
    style SM fill:#9ff,stroke:#333,stroke-width:2px
    style ER fill:#ff9,stroke:#333,stroke-width:2px
```

### Event Flow Architecture

The event system implements a sophisticated pipeline that transforms service-level operations into UI updates. Here's what happens at each stage:

#### Event Lifecycle Stages

1. **Event Creation** (`apps/backend/services/*.service.ts`)
   - Services generate events when state changes occur
   - Example: When an agent modifies a file, the workspace service creates a `WORKSPACE_FILE_CHANGE` event
   - Events are created with minimal data - just the essential payload

2. **Event Enrichment** (`apps/backend/services/event.service.ts#enrichEvent`)
   - System adds metadata to raw events:
     - Unique event ID (UUID v4)
     - Timestamp with millisecond precision
     - Sequence number for ordering
     - Service source identifier
     - Correlation IDs for request tracing

3. **Event Filtering** (`apps/backend/services/event.service.ts#filterEvent`)
   - Applies business rules to determine event routing:
     - Permission checks based on user context
     - Rate limiting to prevent spam
     - Content filtering for sensitive data
     - Event type validation

4. **Event Routing** (`apps/backend/services/event.service.ts#routeEvent`)
   - Determines which streams receive the event:
     - Task-specific events go to task streams
     - System-wide events go to global stream
     - Administrative events go to admin stream
   - Multiple streams can receive the same event

5. **Event Delivery** (`apps/backend/utils/sse-manager.ts#sendEvent`)
   - Serializes events to NDJSON format
   - Writes to active SSE connections
   - Handles connection failures gracefully
   - Buffers events for disconnected clients

6. **Event Processing** (`apps/frontend/src/features/tasks/hooks/use-task-event-subscription.ts`)
   - Frontend receives and parses events
   - Triggers appropriate handlers
   - Updates UI through React Query or direct state changes

```mermaid
graph LR
    subgraph "Event Lifecycle"
        CREATE[Event Created]
        ENRICH[Event Enriched]
        FILTER[Event Filtered]
        ROUTE[Event Routed]
        DELIVER[Event Delivered]
        PROCESS[Event Processed]
    end
    
    subgraph "Processing Steps"
        S1[Add Metadata]
        S2[Validate Schema]
        S3[Apply Filters]
        S4[Select Streams]
        S5[Queue Delivery]
        S6[Update UI]
    end
    
    CREATE --> ENRICH
    ENRICH --> S1
    S1 --> S2
    S2 --> FILTER
    FILTER --> S3
    S3 --> ROUTE
    ROUTE --> S4
    S4 --> S5
    S5 --> DELIVER
    DELIVER --> PROCESS
    PROCESS --> S6
    
    style CREATE fill:#9f9,stroke:#333,stroke-width:2px
    style DELIVER fill:#99f,stroke:#333,stroke-width:2px
    style S6 fill:#f99,stroke:#333,stroke-width:2px
```

### Connection Management Architecture

The system manages SSE connections through a sophisticated state machine that ensures reliability. Each state transition is carefully orchestrated to maintain continuous event delivery:

#### Connection States and Transitions

1. **Disconnected State** (Initial)
   - Implementation: `apps/frontend/src/features/tasks/hooks/use-task-sse-connection.ts#connect`
   - No active connection exists
   - Waiting for explicit connection request or auto-reconnect timer

2. **Connecting State**
   - Creates new EventSource instance
   - Establishes TCP connection to server
   - Timeout: 30 seconds before failing

3. **Handshaking State**
   - Implementation: `apps/backend/routes/events.routes.ts#handleSSEConnection`
   - Server validates request headers
   - Sets up SSE-specific response headers
   - Establishes streaming response

4. **Authenticating State**
   - Validates optional authentication tokens
   - Assigns connection ID
   - Registers with connection pool

5. **Connected State** (Active)
   - Implementation: `apps/backend/utils/sse-manager.ts#registerConnection`
   - Connection ready to receive events
   - Heartbeat monitoring active
   - Sub-states handle event processing:
     - **Idle**: Waiting for events
     - **Receiving**: Event data arriving
     - **Processing**: Parsing NDJSON payload
     - **Dispatching**: Routing to subscribers

6. **Streaming State**
   - Actively transmitting events
   - Buffer management active
   - Monitors write pressure

7. **Heartbeat State**
   - Implementation: `apps/backend/utils/sse-manager.ts#startHeartbeat`
   - Sends ping every 30 seconds
   - Expects pong within 5 seconds
   - Prevents proxy timeouts

8. **Stale State**
   - Heartbeat timeout detected
   - Connection possibly dead
   - Triggers automatic recovery

9. **Reconnecting State**
   - Implementation: `apps/frontend/src/features/tasks/hooks/use-task-sse-connection.ts#reconnect`
   - Exponential backoff: 1s, 2s, 4s, 8s, max 30s
   - Preserves event buffer
   - Maintains subscription state

10. **Failed State**
    - Permanent failure detected
    - Authentication errors
    - Maximum retries exceeded

```mermaid
stateDiagram-v2
    [*] --> Disconnected
    
    Disconnected --> Connecting: connect()
    Connecting --> Handshaking: TCP established
    Handshaking --> Authenticating: SSE headers sent
    Authenticating --> Connected: Auth success
    
    Connected --> Streaming: Events available
    Streaming --> Connected: Event delivered
    
    Connected --> Heartbeat: Ping interval
    Heartbeat --> Connected: Pong received
    Heartbeat --> Stale: Pong timeout
    
    Stale --> Reconnecting: Auto-reconnect
    Reconnecting --> Connecting: Backoff complete
    
    Authenticating --> Failed: Auth failed
    Failed --> Disconnected: Give up
    
    state Connected {
        [*] --> Idle
        Idle --> Receiving: Event arrives
        Receiving --> Processing: Parse NDJSON
        Processing --> Dispatching: Route event
        Dispatching --> Idle: Complete
    }
    
    Connected --> Disconnected: disconnect()
    Streaming --> Disconnected: Connection lost
```

## Backend Implementation

### Event Processing Pipeline

The backend implements a sophisticated event processing pipeline that ensures reliable delivery and proper ordering. Here's a detailed walkthrough of each stage:

#### Event Source Integration

Events originate from three primary sources, each handled differently:

1. **Service-Emitted Events** (`apps/backend/services/*.service.ts`)
   - Direct method calls to EventService.emit()
   - Example: `taskService.updateStatus()` emits `TASK_STATUS_CHANGE`
   - Synchronous emission with immediate processing

2. **Background Job Events** (`apps/backend/services/agents/`)
   - Asynchronous operations emit progress events
   - Example: Code generation jobs emit `AGENT_PROGRESS` events
   - Queued for processing to avoid blocking

3. **System Events** (`apps/backend/app.ts`)
   - Infrastructure-level events (startup, shutdown, errors)
   - Health monitoring events
   - Configuration change notifications

#### Processing Stages

**Enrichment Stage** (`apps/backend/services/event.service.ts#enrichEvent`):
1. **Generate Event ID**: Creates UUID v4 for unique identification
2. **Add Timestamp**: Captures event creation time with microsecond precision
3. **Assign Sequence**: Increments global counter for ordering guarantees
4. **Add Context**: Attaches request ID, user context, and service metadata

**Validation Stage** (`apps/backend/services/event.service.ts#validateEvent`):
1. **Schema Validation**: Ensures event conforms to TypeScript interfaces
2. **Permission Check**: Verifies user can receive this event type
3. **Rate Limiting**: Prevents event flooding (100 events/second per connection)

**Distribution Stage** (`apps/backend/services/event.service.ts#distributeEvent`):
1. **Task Channel**: Routes task-specific events to subscribed connections
2. **Global Channel**: Broadcasts system-wide events to all connections
3. **Admin Channel**: Sends monitoring events to administrative users

**Persistence Stage** (`apps/backend/services/event.service.ts#persistEvent`):
1. **Check Rules**: Determines if event needs persistence
2. **Write Storage**: Saves to file system in NDJSON format
3. **Update Cache**: Maintains in-memory buffer for quick access

**Delivery Mechanisms**:
- **SSE Connections**: Primary real-time delivery via `apps/backend/utils/sse-manager.ts`
- **WebSocket Fallback**: Alternative for environments blocking SSE
- **Polling Endpoint**: Last resort for restrictive networks

```mermaid
graph TB
    subgraph "Event Sources"
        S1[Service Emits Event]
        S2[Background Job Event]
        S3[System Event]
    end
    
    subgraph "Event Processing"
        subgraph "Enrichment Stage"
            E1[Generate Event ID]
            E2[Add Timestamp]
            E3[Assign Sequence]
            E4[Add Context]
        end
        
        subgraph "Validation Stage"
            V1[Schema Validation]
            V2[Permission Check]
            V3[Rate Limiting]
        end
        
        subgraph "Distribution Stage"
            D1[Task Channel]
            D2[Global Channel]
            D3[Admin Channel]
        end
        
        subgraph "Persistence Stage"
            P1[Check Persistence Rules]
            P2[Write to Storage]
            P3[Update Cache]
        end
    end
    
    subgraph "Delivery"
        SSE1[SSE Connections]
        WS1[WebSocket Fallback]
        POLL[Polling Endpoint]
    end
    
    S1 --> E1
    S2 --> E1
    S3 --> E1
    
    E1 --> E2 --> E3 --> E4
    E4 --> V1 --> V2 --> V3
    
    V3 --> D1
    V3 --> D2
    V3 --> D3
    
    D1 --> SSE1
    D2 --> SSE1
    D3 --> SSE1
    
    V3 --> P1
    P1 -->|Persist| P2
    P1 -->|Cache Only| P3
    
    SSE1 -.->|Fallback| WS1
    WS1 -.->|Fallback| POLL
    
    style E1 fill:#9f9,stroke:#333,stroke-width:2px
    style V1 fill:#ff9,stroke:#333,stroke-width:2px
    style D2 fill:#99f,stroke:#333,stroke-width:2px
```

### Event Service Architecture

The Event Service acts as the central hub for all event operations:

```mermaid
classDiagram
    class EventService {
        -sequenceCounter: number
        -eventCache: Map
        -eventBus: EventEmitter
        -storage: EventStorage
        +emitEvent(event): Promise
        +subscribeToTask(taskId, handler): Unsubscribe
        +subscribeToType(type, handler): Unsubscribe
        +getRecentEvents(filters): Event[]
        -enrichEvent(event): Event
        -validateEvent(event): boolean
        -distributeEvent(event): void
    }
    
    class EventStorage {
        <<interface>>
        +saveEvents(taskId, events): Promise
        +loadEvents(taskId, options): Promise
        +deleteEvents(taskId): Promise
    }
    
    class EventCache {
        -cache: LRUCache
        -index: Map
        +add(event): void
        +getByTask(taskId): Event[]
        +getByType(type): Event[]
        +clear(): void
    }
    
    class EventDistributor {
        -channels: Map
        +register(channel, handler): void
        +distribute(event): void
        +getActiveChannels(): string[]
    }
    
    EventService --> EventStorage
    EventService --> EventCache
    EventService --> EventDistributor
```

### SSE Connection Management

The SSE Manager handles the complexity of maintaining persistent connections through a well-defined lifecycle:

#### Connection Lifecycle Details

1. **New Connection** (`apps/backend/routes/events.routes.ts#streamEvents`)
   - HTTP request arrives at `/api/v1/events/stream/:type`
   - Headers validated for SSE compatibility
   - Response headers set: `Content-Type: text/event-stream`

2. **Registration** (`apps/backend/utils/sse-manager.ts#registerConnection`)
   - Generates unique connection ID
   - Creates connection metadata object
   - Adds to appropriate connection pool

3. **Authentication** (`apps/backend/utils/sse-manager.ts#authenticateConnection`)
   - Optional token validation
   - User context extraction
   - Permission scope determination

4. **Subscription** (`apps/backend/utils/sse-manager.ts#subscribeConnection`)
   - Registers event type filters
   - Sets up task-specific subscriptions
   - Configures delivery preferences

5. **Event Streaming** (`apps/backend/utils/sse-manager.ts#streamToConnection`)
   - Writes events in NDJSON format
   - Manages write buffer pressure
   - Handles backpressure gracefully

6. **Connection Closure** (`apps/backend/utils/sse-manager.ts#closeConnection`)
   - Removes from connection pools
   - Cleans up subscriptions
   - Logs connection metrics

#### Connection Pool Management

The system maintains three separate connection pools for different use cases:

- **Global Pool** (`apps/backend/utils/sse-manager.ts#globalPool`)
  - All application-wide events
  - Default for new connections
  - Broadcasts system notifications

- **Task Pool** (`apps/backend/utils/sse-manager.ts#taskPool`)
  - Task-specific event streams
  - Filtered by task ID
  - Higher priority delivery

- **Admin Pool** (`apps/backend/utils/sse-manager.ts#adminPool`)
  - System monitoring events
  - Debug information
  - Performance metrics

#### Health Monitoring Implementation

The health system ensures connections remain active:

1. **Ping Scheduler** (`apps/backend/utils/sse-manager.ts#schedulePing`)
   - Runs every 30 seconds
   - Sends `:ping\n\n` to all connections
   - Tracks last activity time

2. **Health Check** (`apps/backend/utils/sse-manager.ts#checkHealth`)
   - Identifies stale connections (no activity > 60s)
   - Validates socket state
   - Monitors memory usage

3. **Cleanup Process** (`apps/backend/utils/sse-manager.ts#cleanupStale`)
   - Closes dead connections
   - Frees associated resources
   - Updates connection metrics

```mermaid
graph LR
    subgraph "Connection Lifecycle"
        NEW[New Connection]
        REG[Register]
        AUTH[Authenticate]
        SUB[Subscribe]
        STREAM[Stream Events]
        CLOSE[Close]
    end
    
    subgraph "Connection Pool"
        POOL[Connection Pool]
        GLOBAL[Global Connections]
        TASK[Task Connections]
        ADMIN[Admin Connections]
    end
    
    subgraph "Health Management"
        PING[Ping Scheduler]
        CHECK[Health Check]
        CLEAN[Cleanup]
    end
    
    NEW --> REG
    REG --> AUTH
    AUTH --> SUB
    SUB --> STREAM
    STREAM --> CLOSE
    
    REG --> POOL
    POOL --> GLOBAL
    POOL --> TASK
    POOL --> ADMIN
    
    PING --> CHECK
    CHECK --> CLEAN
    CLEAN --> POOL
    
    style NEW fill:#9f9,stroke:#333,stroke-width:2px
    style STREAM fill:#99f,stroke:#333,stroke-width:2px
    style CLEAN fill:#f99,stroke:#333,stroke-width:2px
```

### Event Routing Logic

| Event Type | Global Stream | Task Stream | Admin Stream | Persistence |
|------------|---------------|-------------|--------------|-------------|
| MESSAGE | ✓ | ✓ | - | Conditional |
| RESULT | ✓ | ✓ | ✓ | Always |
| ERROR | ✓ | ✓ | ✓ | Always |
| TOOL | - | ✓ | - | Never |
| WORKSPACE | ✓ | ✓ | - | Always |
| SERVER | - | ✓ | ✓ | Conditional |

## Frontend Implementation

### Event Reception Architecture

The frontend implements a sophisticated event reception and distribution system that processes real-time updates through multiple layers:

#### Reception Layer Details

1. **SSE Connection** (`apps/frontend/src/features/tasks/hooks/use-task-sse-connection.ts`)
   - Creates EventSource instance with proper URL construction
   - Handles connection lifecycle (open, message, error events)
   - Implements automatic reconnection with exponential backoff
   - Example connection: `new EventSource('/api/v1/events/stream/task/123')`

2. **NDJSON Parser** (`apps/frontend/src/features/tasks/hooks/use-task-sse-connection.ts#parseEvent`)
   - Parses newline-delimited JSON format
   - Handles malformed data gracefully
   - Validates JSON structure before processing
   - Extracts event type and payload

3. **Event Validator** (`apps/frontend/src/lib/utils/event-validator.ts`)
   - Validates event against TypeScript interfaces
   - Ensures required fields are present
   - Type guards for event discrimination
   - Logs validation errors for debugging

#### Processing Layer Implementation

1. **Event Router** (`apps/frontend/src/features/tasks/providers/task-event-provider.tsx`)
   - Central dispatcher for all incoming events
   - Routes based on event type and metadata
   - Maintains routing table for efficiency
   - Handles wildcard subscriptions

2. **Event Filter** (`apps/frontend/src/features/tasks/hooks/use-event-filter.ts`)
   - Applies user-defined filters
   - Filters by event type, severity, source
   - Implements debouncing for high-frequency events
   - Preserves filter state across reconnections

3. **Event Buffer** (`apps/frontend/src/features/tasks/utils/event-buffer.ts`)
   - Circular buffer for recent events
   - Configurable size (default: 1000 events)
   - Enables replay after reconnection
   - Provides event history for debugging

#### Distribution Layer Architecture

**Subscription Manager** manages three types of subscriptions:

1. **Task Subscriptions** (`apps/frontend/src/features/tasks/hooks/use-task-event-subscription.ts`)
   - Subscribes to events for specific tasks
   - Filters by task ID automatically
   - Unsubscribes on component unmount
   - Example: `useTaskEventSubscription(taskId, handleEvent)`

2. **Type Subscriptions** (`apps/frontend/src/features/tasks/hooks/use-event-type-subscription.ts`)
   - Subscribes to specific event types globally
   - Useful for cross-cutting concerns
   - Example: `useEventTypeSubscription('ERROR', handleError)`

3. **Global Subscriptions** (`apps/frontend/src/features/tasks/providers/global-event-provider.tsx`)
   - Receives all events regardless of type
   - Used for logging and monitoring
   - Provides event stream context to entire app

#### UI Update Mechanisms

1. **Event Handlers** (`apps/frontend/src/features/tasks/handlers/`)
   - Type-specific handler functions
   - Transform events into UI actions
   - Handle side effects (notifications, logging)

2. **Cache Invalidator** (`apps/frontend/src/features/tasks/utils/cache-invalidation.ts`)
   - Maps event types to React Query keys
   - Intelligently invalidates affected queries
   - Prevents unnecessary refetches
   - Example: `TASK_STATUS_CHANGE` invalidates `['tasks', taskId]`

3. **State Updater** (`apps/frontend/src/features/tasks/stores/`)
   - Direct state updates for immediate feedback
   - Updates Zustand stores for global state
   - Triggers re-renders efficiently

#### Component Integration Examples

- **Task Components** (`apps/frontend/src/features/tasks/components/`)
  - Task viewer subscribes to iteration events
  - Updates progress in real-time
  - Shows live agent output

- **Sidebar** (`apps/frontend/src/features/navigation/components/sidebar.tsx`)
  - Updates task counts on status changes
  - Shows active task indicators
  - Reflects workspace changes

- **Notifications** (`apps/frontend/src/features/notifications/`)
  - Displays toast notifications for errors
  - Shows success messages for completions
  - Provides action buttons for quick responses

```mermaid
graph TB
    subgraph "Event Reception Layer"
        SSE[SSE Connection]
        PARSER[NDJSON Parser]
        VALIDATOR[Event Validator]
    end
    
    subgraph "Event Processing Layer"
        ROUTER[Event Router]
        FILTER[Event Filter]
        BUFFER[Event Buffer]
    end
    
    subgraph "Distribution Layer"
        subgraph "Subscription Manager"
            TSUB[Task Subscriptions]
            ESUB[Type Subscriptions]
            GSUB[Global Subscriptions]
        end
    end
    
    subgraph "UI Update Layer"
        HANDLER[Event Handlers]
        CACHE[Cache Invalidator]
        STATE[State Updater]
    end
    
    subgraph "Components"
        TASK[Task Components]
        SIDEBAR[Sidebar]
        NOTIFY[Notifications]
    end
    
    SSE --> PARSER
    PARSER --> VALIDATOR
    VALIDATOR --> ROUTER
    
    ROUTER --> FILTER
    FILTER --> BUFFER
    
    BUFFER --> TSUB
    BUFFER --> ESUB
    BUFFER --> GSUB
    
    TSUB --> HANDLER
    ESUB --> HANDLER
    GSUB --> HANDLER
    
    HANDLER --> CACHE
    HANDLER --> STATE
    
    CACHE --> TASK
    STATE --> SIDEBAR
    HANDLER --> NOTIFY
    
    style ROUTER fill:#f9f,stroke:#333,stroke-width:4px
    style HANDLER fill:#9ff,stroke:#333,stroke-width:2px
    style CACHE fill:#ff9,stroke:#333,stroke-width:2px
```

### Event Stream Provider Architecture

The providers manage connection lifecycle and event distribution:

```mermaid
stateDiagram-v2
    [*] --> Unmounted
    
    Unmounted --> Mounting: Component mounts
    Mounting --> Connecting: Initialize SSE
    Connecting --> Connected: SSE open
    Connected --> Active: First event
    
    Active --> Active: Process events
    Active --> Reconnecting: Connection lost
    Reconnecting --> Connecting: Retry
    
    Active --> Unmounting: Component unmounts
    Unmounting --> Cleanup: Close SSE
    Cleanup --> Unmounted: Complete
    
    state Active {
        [*] --> Listening
        Listening --> Processing: Event received
        Processing --> Distributing: Parse success
        Distributing --> Updating: Notify subscribers
        Updating --> Listening: Complete
        
        Processing --> Error: Parse failed
        Error --> Listening: Log & continue
    }
    
    state Reconnecting {
        [*] --> Waiting
        Waiting --> Backoff: Calculate delay
        Backoff --> Attempting: Timer elapsed
        Attempting --> [*]: Retry
    }
```

### React Query Integration

Events trigger intelligent cache updates through a sophisticated invalidation system:

```mermaid
graph LR
    subgraph "Event Types"
        E1[Task Events]
        E2[Result Events]
        E3[Error Events]
        E4[Workspace Events]
    end
    
    subgraph "Invalidation Rules"
        R1[Task List]
        R2[Task Detail]
        R3[Messages]
        R4[Git Diff]
        R5[Server Status]
    end
    
    subgraph "Query Keys"
        K1[tasks.list]
        K2[tasks.detail.*]
        K3[tasks.messages.*]
        K4[tasks.diff.*]
        K5[tasks.server.*]
    end
    
    E1 --> R1
    E1 --> R2
    E2 --> R1
    E2 --> R2
    E2 --> R3
    E3 --> R2
    E4 --> R4
    E4 --> R5
    
    R1 --> K1
    R2 --> K2
    R3 --> K3
    R4 --> K4
    R5 --> K5
    
    style E2 fill:#9f9,stroke:#333,stroke-width:2px
    style R1 fill:#99f,stroke:#333,stroke-width:2px
    style K1 fill:#ff9,stroke:#333,stroke-width:2px
```

### Component Integration Patterns

Components integrate with the event system through well-defined patterns:

```mermaid
graph TB
    subgraph "Hook Layer"
        H1[useGlobalEventStream]
        H2[useTaskEventSubscription]
        H3[useEventTypeSubscription]
        H4[useEventHistory]
    end
    
    subgraph "Context Layer"
        C1[GlobalEventContext]
        C2[TaskEventContext]
        C3[EventFilterContext]
    end
    
    subgraph "Component Examples"
        COMP1[Task List]
        COMP2[Task Viewer]
        COMP3[Event Log]
        COMP4[Status Bar]
    end
    
    subgraph "Update Patterns"
        UP1[Optimistic Updates]
        UP2[Real-time Sync]
        UP3[Error Recovery]
    end
    
    H1 --> C1
    H2 --> C2
    H3 --> C1
    H4 --> C3
    
    COMP1 --> H1
    COMP1 --> UP1
    
    COMP2 --> H2
    COMP2 --> UP2
    
    COMP3 --> H4
    COMP3 --> C3
    
    COMP4 --> H3
    COMP4 --> UP3
    
    style H1 fill:#9ff,stroke:#333,stroke-width:2px
    style UP1 fill:#9f9,stroke:#333,stroke-width:2px
```

### Event Hook Patterns

| Hook | Purpose | Usage Pattern | Returns |
|------|---------|---------------|---------|
| useGlobalEventStream | Access global stream context | `const { subscribe } = useGlobalEventStream()` | Context API |
| useTaskEventSubscription | Subscribe to task events | `useTaskEventSubscription(taskId, handler)` | Cleanup function |
| useEventTypeSubscription | Subscribe by event type | `useEventTypeSubscription('result', handler)` | Cleanup function |
| useEventHistory | Load past events | `const { events, loading } = useEventHistory(taskId)` | Query state |

## Data Flow

### End-to-End Event Journey

The complete event flow demonstrates the sophistication of the real-time architecture. Here's a detailed walkthrough of what happens when a user action triggers an event:

#### Step-by-Step Event Flow

1. **User Interaction** (`apps/frontend/src/features/tasks/components/`)
   - User clicks "Run Task" button in the UI
   - Component calls mutation hook
   - HTTP request sent to backend API

2. **API Processing** (`apps/backend/routes/tasks.routes.ts`)
   - Route handler receives POST request
   - Validates request parameters
   - Calls TaskService to execute action

3. **Event Emission** (`apps/backend/services/task.service.ts`)
   - Service performs business logic
   - Creates event object with type and payload
   - Calls `eventService.emit(event)`

4. **Event Processing Phase**
   - **ID Generation**: UUID v4 created for tracking
   - **Timestamping**: Current time captured with microsecond precision
   - **Sequencing**: Global counter incremented atomically
   - **Schema Validation**: Event structure verified against TypeScript interface

5. **Event Bus Distribution** (`apps/backend/services/event.service.ts`)
   - Event published to internal event bus
   - All subscribers notified synchronously
   - Routing rules applied based on event type

6. **Channel Routing** (`apps/backend/utils/sse-manager.ts`)
   - SSE Manager receives event from bus
   - Determines target connections:
     - Global stream for system-wide events
     - Task stream for task-specific events
   - Filters connections by subscription criteria

7. **Connection Delivery**
   - Each connection receives event independently
   - NDJSON format: `{"type":"TASK_STATUS","data":{...}}\n`
   - Write buffer managed to prevent overflow
   - Failed writes trigger reconnection

8. **Frontend Reception** (`apps/frontend/src/features/tasks/hooks/use-task-sse-connection.ts`)
   - EventSource receives message event
   - Raw data extracted from event
   - NDJSON parsed to JavaScript object

9. **Frontend Processing**
   - **Validation**: Event structure verified
   - **Cache Invalidation**: React Query keys invalidated
   - **State Updates**: Component state modified
   - **UI Rendering**: React re-renders affected components

10. **User Feedback**
    - Visual changes appear immediately
    - Progress indicators update
    - Success/error messages displayed

```mermaid
sequenceDiagram
    participant U as User
    participant UI as UI Component
    participant API as API Endpoint
    participant ES as Event Service
    participant EB as Event Bus
    participant SSE as SSE Manager
    participant CON as Connections
    participant FE as Frontend SSE
    participant RQ as React Query
    participant DOM as DOM Updates
    
    U->>UI: Perform action
    UI->>API: HTTP request
    API->>ES: Emit event
    
    Note over ES,EB: Event Processing
    ES->>ES: Generate ID & timestamp
    ES->>ES: Increment sequence
    ES->>ES: Validate schema
    ES->>EB: Publish to bus
    
    Note over EB,SSE: Distribution
    EB->>SSE: Route to channels
    SSE->>CON: Find connections
    
    par Global Stream
        CON->>FE: NDJSON event
    and Task Stream
        CON->>FE: NDJSON event
    end
    
    Note over FE,DOM: Frontend Processing
    FE->>FE: Parse NDJSON
    FE->>FE: Validate event
    FE->>RQ: Invalidate cache
    FE->>DOM: Update UI
    
    DOM->>U: See changes
```

### Event Routing Architecture

Events flow through a sophisticated routing system that ensures proper delivery:

```mermaid
graph TB
    subgraph "Event Sources"
        S1[User Action]
        S2[Agent Update]
        S3[System Event]
        S4[Background Job]
    end
    
    subgraph "Event Bus"
        EB[Central Bus]
        TOPIC[Topic Router]
        FILTER[Filter Engine]
    end
    
    subgraph "Delivery Channels"
        CH1[Task Channel]
        CH2[Global Channel]
        CH3[Type Channel]
        CH4[Admin Channel]
    end
    
    subgraph "Subscribers"
        SUB1[Task Components]
        SUB2[Sidebar Updates]
        SUB3[Notifications]
        SUB4[Admin Dashboard]
    end
    
    S1 --> EB
    S2 --> EB
    S3 --> EB
    S4 --> EB
    
    EB --> TOPIC
    TOPIC --> FILTER
    
    FILTER --> CH1
    FILTER --> CH2
    FILTER --> CH3
    FILTER --> CH4
    
    CH1 --> SUB1
    CH2 --> SUB2
    CH3 --> SUB3
    CH4 --> SUB4
    
    style EB fill:#f9f,stroke:#333,stroke-width:4px
    style TOPIC fill:#9ff,stroke:#333,stroke-width:2px
    style SUB1 fill:#ff9,stroke:#333,stroke-width:2px
```

### Connection Recovery Flow

The system handles connection failures gracefully with automatic recovery. Here's how each stage ensures no events are lost:

#### Recovery Process Details

1. **Connection Disruption Detection**
   - Implementation: `apps/frontend/src/features/tasks/hooks/use-task-sse-connection.ts#handleError`
   - Triggers: Network failure, server restart, proxy timeout
   - Detection time: < 100ms via EventSource error event

2. **Buffering Stage** (`apps/frontend/src/features/tasks/utils/event-buffer.ts`)
   - **Collecting**: All new events stored in circular buffer
   - **Buffer Management**: 
     - Default size: 1000 events
     - Memory limit: 10MB
     - Oldest events dropped when full
   - **Persistence**: Critical events saved to IndexedDB

3. **Reconnection Process** (`apps/frontend/src/features/tasks/hooks/use-task-sse-connection.ts#reconnect`)
   - **Initial Attempt**: Immediate reconnection try
   - **Backoff Strategy**:
     - 1st retry: 1 second delay
     - 2nd retry: 2 seconds
     - 3rd retry: 4 seconds
     - Max delay: 30 seconds
   - **Connection Validation**: Waits for first event before declaring success

4. **Recovery Handshake** (`apps/backend/routes/events.routes.ts#handleReconnect`)
   - **Send Last Sequence**: Client sends last received sequence number
   - **Server Validation**: Checks if events are still in cache
   - **Gap Detection**: Identifies missing events by sequence

5. **Event Replay** (`apps/backend/utils/sse-manager.ts#replayEvents`)
   - **Missed Events**: Server sends all events after last sequence
   - **Order Preservation**: Events sent in original order
   - **Duplicate Prevention**: Client filters by event ID

6. **State Synchronization**
   - **Cache Refresh**: React Query refetches active queries
   - **UI Update**: Components re-render with latest state
   - **Notification**: User informed of successful reconnection

#### Buffer Management Implementation

The buffering system ensures events aren't lost during disconnections:

- **Circular Buffer**: Fixed-size array with head/tail pointers
- **Priority Levels**: Critical events never dropped
- **Compression**: Events compressed after 30 seconds
- **Persistence**: IndexedDB for long-term storage

```mermaid
stateDiagram-v2
    [*] --> Connected: Initial connection
    
    Connected --> Disrupted: Network issue
    Disrupted --> Buffering: Start buffering
    
    Buffering --> Reconnecting: Attempt reconnect
    Reconnecting --> Handshaking: Connection established
    Handshaking --> Recovering: Send buffered
    Recovering --> Connected: Resume normal
    
    Reconnecting --> Backoff: Connection failed
    Backoff --> Reconnecting: Retry with delay
    
    state Buffering {
        [*] --> Collecting
        Collecting --> Collecting: Add event
        Collecting --> Full: Buffer limit
        Full --> Dropping: Drop oldest
        Dropping --> Collecting: Continue
    }
    
    state Recovering {
        [*] --> SendMissed
        SendMissed --> Validate: Check sequence
        Validate --> FillGaps: Request missing
        FillGaps --> [*]: Complete
    }
```

## Integration Points

### System Integration Map

The Event system integrates deeply with all major components:

```mermaid
graph TB
    subgraph "Event Core"
        EVENT[Event System]
    end
    
    subgraph "Event Producers"
        AGENT[Agent Service]
        TASK[Task Service]
        WORK[Workspace Service]
        PREV[Preview Service]
    end
    
    subgraph "Event Consumers"
        UI[UI Components]
        CACHE[Query Cache]
        LOG[Audit Logger]
        METRIC[Metrics Collector]
    end
    
    subgraph "Infrastructure"
        HTTP[HTTP Layer]
        WS[WebSocket Layer]
        STORE[Storage Layer]
    end
    
    AGENT --> EVENT
    TASK --> EVENT
    WORK --> EVENT
    PREV --> EVENT
    
    EVENT --> UI
    EVENT --> CACHE
    EVENT --> LOG
    EVENT --> METRIC
    
    EVENT <--> HTTP
    EVENT <--> WS
    EVENT <--> STORE
    
    style EVENT fill:#f9f,stroke:#333,stroke-width:4px
    style AGENT fill:#9ff,stroke:#333,stroke-width:2px
    style UI fill:#ff9,stroke:#333,stroke-width:2px
```

### Event Type Matrix

| Event Type | Producers | Consumers | Cache Impact | Persistence |
|------------|-----------|-----------|--------------|-------------|
| MESSAGE | Agent Service | Task UI, Event Log | Messages | Conditional |
| RESULT | Agent Service | Task List, Task Detail | Full refresh | Always |
| ERROR | All Services | Error Display, Logs | Status only | Always |
| TOOL | Agent Service | Tool Monitor | None | Never |
| WORKSPACE | Workspace Service | File Tree, Git UI | Workspace | Always |
| SERVER | Preview Service | Preview UI | Server status | Conditional |
