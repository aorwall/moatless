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

The event system implements a sophisticated pipeline that transforms service-level operations into UI updates:

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

The system manages SSE connections through a sophisticated state machine that ensures reliability:

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

The backend implements a sophisticated event processing pipeline that ensures reliable delivery and proper ordering:

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

The SSE Manager handles the complexity of maintaining persistent connections:

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

### Performance Optimization

The backend employs several strategies to handle high event throughput:

```mermaid
graph TB
    subgraph "Event Batching"
        B1[Collect Events]
        B2[Batch Window]
        B3[Flush Batch]
    end
    
    subgraph "Compression"
        C1[Gzip Events]
        C2[Binary Protocol]
        C3[Delta Encoding]
    end
    
    subgraph "Caching Strategy"
        CACHE1[Memory Cache]
        CACHE2[Redis Cache]
        CACHE3[Disk Cache]
    end
    
    B1 --> B2
    B2 -->|10ms window| B3
    B3 --> C1
    
    C1 --> CACHE1
    CACHE1 -->|Overflow| CACHE2
    CACHE2 -->|Archive| CACHE3
    
    style B2 fill:#9f9,stroke:#333,stroke-width:2px
    style C1 fill:#99f,stroke:#333,stroke-width:2px
    style CACHE1 fill:#ff9,stroke:#333,stroke-width:2px
```

## Frontend Implementation

### Event Reception Architecture

The frontend implements a sophisticated event reception and distribution system:

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

The complete event flow demonstrates the sophistication of the real-time architecture:

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

The system handles connection failures gracefully with automatic recovery:

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

### Cross-System Event Flows

Events enable complex cross-system interactions:

```mermaid
sequenceDiagram
    participant A as Agent
    participant E as Events
    participant T as Tasks
    participant W as Workspace
    participant U as UI
    
    Note over A,U: Code Generation Flow
    A->>E: TOOL event (file write)
    E->>W: Update notification
    W->>E: WORKSPACE event
    E->>U: File tree update
    
    Note over A,U: Error Recovery Flow
    A->>E: ERROR event
    E->>T: Mark iteration failed
    T->>E: TASK event (status)
    E->>U: Show error state
    
    Note over A,U: Preview Update Flow
    W->>E: SERVER event (started)
    E->>U: Enable preview
    A->>E: WORKSPACE event (file)
    E->>U: Hot reload preview
```

## Configuration

### Configuration Architecture

The Event system supports flexible configuration at multiple levels:

```mermaid
graph TB
    subgraph "Configuration Layers"
        ENV[Environment Variables]
        FEAT[Feature Flags]
        RUN[Runtime Config]
        DYN[Dynamic Config]
    end
    
    subgraph "Backend Config"
        BC1[Connection Limits]
        BC2[Buffer Sizes]
        BC3[Timeouts]
        BC4[Persistence Rules]
    end
    
    subgraph "Frontend Config"
        FC1[Reconnection Strategy]
        FC2[Cache Strategy]
        FC3[Debug Options]
        FC4[Stream Selection]
    end
    
    ENV --> BC1
    ENV --> BC2
    FEAT --> FC4
    RUN --> BC3
    RUN --> FC1
    DYN --> BC4
    DYN --> FC2
    
    style ENV fill:#9f9,stroke:#333,stroke-width:2px
    style FEAT fill:#99f,stroke:#333,stroke-width:2px
    style DYN fill:#ff9,stroke:#333,stroke-width:2px
```

### Performance Tuning Parameters

| Parameter | Purpose | Default | Impact |
|-----------|---------|---------|--------|
| Connection Pool Size | Max concurrent SSE connections | 1000 | Memory usage |
| Event Buffer Size | Events to buffer during disconnect | 100 | Recovery speed |
| Batch Window | Time to collect events for batching | 10ms | Latency vs throughput |
| Cache TTL | How long to cache events | 5min | Memory vs performance |
| Ping Interval | Connection health check frequency | 30s | Network usage |
| Reconnect Backoff | Delay multiplier for reconnection | 2x | Recovery time |

## Best Practices

### Architectural Principles

The Event system follows key principles for reliability and performance:

```mermaid
graph TD
    subgraph "Core Principles"
        P1[Event Immutability]
        P2[Ordered Delivery]
        P3[At-Least-Once]
        P4[Graceful Degradation]
    end
    
    subgraph "Design Patterns"
        D1[Event Sourcing]
        D2[CQRS Pattern]
        D3[Pub-Sub Model]
        D4[Circuit Breaker]
    end
    
    subgraph "Implementation"
        I1[Sequence Numbers]
        I2[Event Buffer]
        I3[Retry Logic]
        I4[Fallback Polling]
    end
    
    P1 --> D1
    P2 --> I1
    P3 --> I2
    P4 --> I4
    
    D1 --> I1
    D3 --> I2
    D4 --> I3
    
    style P1 fill:#f9f,stroke:#333,stroke-width:2px
    style D1 fill:#9ff,stroke:#333,stroke-width:2px
    style I1 fill:#ff9,stroke:#333,stroke-width:2px
```

### Performance Architecture

Optimize event handling for scale:

```mermaid
graph LR
    subgraph "Optimization Strategies"
        O1[Event Batching]
        O2[Selective Subscription]
        O3[Smart Caching]
        O4[Connection Pooling]
    end
    
    subgraph "Monitoring Points"
        M1[Event Throughput]
        M2[Connection Count]
        M3[Buffer Usage]
        M4[Latency Metrics]
    end
    
    subgraph "Scaling Tactics"
        S1[Horizontal Scaling]
        S2[Event Sharding]
        S3[Regional Streams]
        S4[CDN Integration]
    end
    
    O1 --> M1
    O2 --> M2
    O3 --> M3
    O4 --> M4
    
    M1 --> S1
    M2 --> S2
    M3 --> S3
    M4 --> S4
    
    style O1 fill:#9f9,stroke:#333,stroke-width:2px
    style M1 fill:#99f,stroke:#333,stroke-width:2px
    style S1 fill:#ff9,stroke:#333,stroke-width:2px
```

### Security Considerations

- **Event Validation**: Schema validation on all events
- **Rate Limiting**: Per-connection and per-IP limits
- **Authentication**: Optional auth for sensitive streams
- **Encryption**: TLS for all SSE connections
- **Sanitization**: Clean user content in events
- **Monitoring**: Track suspicious patterns

## Troubleshooting

### Diagnostic Architecture

Systematic troubleshooting through layered diagnostics:

```mermaid
graph TD
    subgraph "Symptom Detection"
        S1[No Events]
        S2[Delayed Events]
        S3[Missing Events]
        S4[Connection Drops]
    end
    
    subgraph "Diagnostic Tools"
        D1[Event Inspector]
        D2[Connection Monitor]
        D3[Sequence Validator]
        D4[Performance Profiler]
    end
    
    subgraph "Root Causes"
        R1[Network Issues]
        R2[Server Overload]
        R3[Client Errors]
        R4[Config Problems]
    end
    
    subgraph "Resolutions"
        F1[Reconnect Logic]
        F2[Scale Resources]
        F3[Fix Subscriptions]
        F4[Update Config]
    end
    
    S1 --> D1
    S2 --> D4
    S3 --> D3
    S4 --> D2
    
    D1 --> R3
    D2 --> R1
    D3 --> R2
    D4 --> R4
    
    R1 --> F1
    R2 --> F2
    R3 --> F3
    R4 --> F4
    
    style S1 fill:#f99,stroke:#333,stroke-width:2px
    style D1 fill:#ff9,stroke:#333,stroke-width:2px
    style F1 fill:#9f9,stroke:#333,stroke-width:2px
```

### Health Monitoring Dashboard

Key metrics to track system health:

```mermaid
graph LR
    subgraph "Real-time Metrics"
        RT1[Active Connections]
        RT2[Events/Second]
        RT3[Error Rate]
        RT4[Avg Latency]
    end
    
    subgraph "Aggregate Metrics"
        AG1[Daily Event Volume]
        AG2[Peak Connections]
        AG3[Reconnection Rate]
        AG4[Success Rate]
    end
    
    subgraph "Alerts"
        A1[Connection Spike]
        A2[High Error Rate]
        A3[Memory Pressure]
        A4[Sequence Gaps]
    end
    
    RT1 --> AG2
    RT2 --> AG1
    RT3 --> AG4
    RT4 --> AG3
    
    AG2 --> A1
    AG4 --> A2
    RT1 --> A3
    RT2 --> A4
    
    style RT2 fill:#9f9,stroke:#333,stroke-width:2px
    style AG1 fill:#99f,stroke:#333,stroke-width:2px
    style A2 fill:#f99,stroke:#333,stroke-width:2px
```

### Debug Workflow

1. **Check Connection Status**
   ```bash
   curl http://localhost:8000/api/v1/events/stream/status
   ```

2. **Monitor Event Stream**
   ```bash
   curl -N http://localhost:8000/api/v1/events/stream | jq .
   ```

3. **Validate Event Sequence**
   ```javascript
   // In browser console
   window.DEBUG_EVENT_STREAM = true;
   ```

4. **Analyze Performance**
   - Check Network tab for SSE timing
   - Monitor Memory profiler for leaks
   - Review Performance timeline

### Common Patterns

| Issue | Pattern | Solution |
|-------|---------|----------|
| Proxy timeout | Drops after 60s | Add keep-alive headers |
| Buffer overflow | Events lost on reconnect | Increase buffer size |
| Sequence gaps | Missing events | Check event cache |
| Memory leak | Growing heap | Clean up subscriptions |

## Related Documentation
- [Architecture Overview](../architecture-overview.md)
- [Task Management Architecture](./tasks.md)
- [Agent Architecture](./agents.md)
- [Event-Driven Pattern](../patterns/event-driven.md)
- [SSE Management Pattern](../patterns/sse-management.md)
- [Cache Invalidation Pattern](../patterns/cache-invalidation.md)
- [Real-time Architecture](../event-architecture.md)