# Event-Driven Architecture Pattern

## Problem

In complex applications with multiple components and real-time updates, managing state synchronization becomes challenging:
- Components need to react to changes in other parts of the system
- Direct coupling between components makes the system brittle
- State management becomes complex with many interdependencies
- Real-time updates require complex coordination
- Debugging state changes across components is difficult

## Solution

Event-Driven Architecture decouples components using events as the primary communication mechanism:
- **Event Emitters**: Components publish events when state changes
- **Event Listeners**: Components subscribe to relevant events
- **Central Event Bus**: Manages event distribution
- **Type-Safe Events**: Strongly typed event payloads
- **Async Processing**: Non-blocking event handling

## Implementation

### Core Event System

```typescript
// Event type definitions
export enum EventType {
  // Task events
  MESSAGE = 'MESSAGE',
  RESULT = 'RESULT',
  ERROR = 'ERROR',
  INPUT = 'INPUT',
  
  // System events
  TASK_CREATED = 'TASK_CREATED',
  TASK_UPDATED = 'TASK_UPDATED',
  TASK_DELETED = 'TASK_DELETED',
  
  // Server events
  SERVER_STARTED = 'SERVER_STARTED',
  SERVER_STOPPED = 'SERVER_STOPPED',
  
  // Workspace events
  WORKSPACE_CREATED = 'WORKSPACE_CREATED',
  WORKSPACE_DELETED = 'WORKSPACE_DELETED',
}

// Event payload types
interface EventMap {
  [EventType.MESSAGE]: {
    taskId: string;
    iterationId: string;
    role: 'user' | 'assistant' | 'system';
    content: string;
    timestamp: string;
  };
  
  [EventType.RESULT]: {
    taskId: string;
    iterationId: string;
    status: 'completed' | 'failed';
    cost?: number;
    resultText?: string;
  };
  
  [EventType.TASK_CREATED]: {
    taskId: string;
    title: string;
    repositoryId: string;
  };
  
  [EventType.SERVER_STARTED]: {
    taskId: string;
    port: number;
    url: string;
  };
}

// Base event structure
interface BaseEvent<T extends EventType> {
  id: string;
  type: T;
  subtype?: string;
  payload: EventMap[T];
  timestamp: string;
  source: string;
}

type TaskEvent<T extends EventType = EventType> = BaseEvent<T>;
```

### Event Bus Implementation

```typescript
// Type-safe event emitter
export class TypedEventEmitter<T extends Record<string, any>> {
  private listeners = new Map<keyof T, Set<(payload: any) => void>>();
  private history: Array<{ type: keyof T; payload: any; timestamp: Date }> = [];
  
  emit<K extends keyof T>(type: K, payload: T[K]): void {
    // Add to history for debugging
    this.history.push({
      type,
      payload,
      timestamp: new Date(),
    });
    
    // Keep history bounded
    if (this.history.length > 1000) {
      this.history.shift();
    }
    
    // Emit to listeners
    const listeners = this.listeners.get(type);
    if (listeners) {
      // Use queueMicrotask for async handling
      queueMicrotask(() => {
        listeners.forEach(listener => {
          try {
            listener(payload);
          } catch (error) {
            console.error(`Error in event listener for ${String(type)}:`, error);
          }
        });
      });
    }
  }
  
  on<K extends keyof T>(
    type: K,
    listener: (payload: T[K]) => void
  ): () => void {
    if (!this.listeners.has(type)) {
      this.listeners.set(type, new Set());
    }
    
    this.listeners.get(type)!.add(listener);
    
    // Return unsubscribe function
    return () => {
      this.listeners.get(type)?.delete(listener);
    };
  }
  
  once<K extends keyof T>(
    type: K,
    listener: (payload: T[K]) => void
  ): () => void {
    const unsubscribe = this.on(type, (payload) => {
      listener(payload);
      unsubscribe();
    });
    
    return unsubscribe;
  }
  
  off<K extends keyof T>(type: K, listener?: (payload: T[K]) => void): void {
    if (!listener) {
      this.listeners.delete(type);
    } else {
      this.listeners.get(type)?.delete(listener);
    }
  }
  
  getHistory(): typeof this.history {
    return [...this.history];
  }
  
  clear(): void {
    this.listeners.clear();
    this.history = [];
  }
}

// Global event bus instance
export const eventBus = new TypedEventEmitter<EventMap>();
```

### Event Service

```typescript
// apps/backend/services/event.service.ts
export class EventService {
  private eventBus = new TypedEventEmitter<EventMap>();
  private sseClients = new Map<string, Response>();
  
  // Emit events with automatic distribution
  async emitTaskEvent(event: TaskEvent): Promise<void> {
    // Validate event structure
    this.validateEvent(event);
    
    // Emit to local listeners
    this.eventBus.emit(event.type, event.payload);
    
    // Store event for persistence
    await this.storeEvent(event);
    
    // Send to SSE clients
    this.broadcastToSSE(event);
    
    // Log for debugging
    console.debug('Event emitted:', {
      type: event.type,
      taskId: event.payload.taskId,
      timestamp: event.timestamp,
    });
  }
  
  // Subscribe to events
  subscribe<T extends EventType>(
    type: T,
    listener: (payload: EventMap[T]) => void
  ): () => void {
    return this.eventBus.on(type, listener);
  }
  
  // Subscribe to all events
  subscribeAll(
    listener: (event: TaskEvent) => void
  ): () => void {
    const unsubscribers: Array<() => void> = [];
    
    // Subscribe to each event type
    Object.values(EventType).forEach(type => {
      const unsubscribe = this.eventBus.on(type, (payload) => {
        listener({
          id: generateId(),
          type,
          payload,
          timestamp: new Date().toISOString(),
          source: 'event-service',
        });
      });
      unsubscribers.push(unsubscribe);
    });
    
    // Return combined unsubscribe function
    return () => {
      unsubscribers.forEach(unsub => unsub());
    };
  }
  
  // SSE stream management
  addSSEClient(clientId: string, response: Response): void {
    this.sseClients.set(clientId, response);
    
    // Send initial heartbeat
    this.sendSSEMessage(response, {
      type: 'heartbeat',
      data: { timestamp: Date.now() },
    });
  }
  
  removeSSEClient(clientId: string): void {
    this.sseClients.delete(clientId);
  }
  
  private broadcastToSSE(event: TaskEvent): void {
    const message = {
      type: 'event',
      data: event,
    };
    
    for (const [clientId, response] of this.sseClients) {
      try {
        this.sendSSEMessage(response, message);
      } catch (error) {
        console.warn(`Failed to send SSE to client ${clientId}:`, error);
        this.sseClients.delete(clientId);
      }
    }
  }
  
  private sendSSEMessage(response: Response, message: any): void {
    const data = JSON.stringify(message);
    response.write(`data: ${data}\n\n`);
  }
  
  private async storeEvent(event: TaskEvent): Promise<void> {
    // Store event in filesystem or database
    const eventFile = path.join(
      'data/events',
      event.payload.taskId,
      `${event.id}.json`
    );
    
    await fs.ensureDir(path.dirname(eventFile));
    await fs.writeJson(eventFile, event, { spaces: 2 });
  }
  
  private validateEvent(event: TaskEvent): void {
    if (!event.id || !event.type || !event.payload) {
      throw new Error('Invalid event structure');
    }
    
    if (!event.payload.taskId) {
      throw new Error('Event payload must include taskId');
    }
  }
}
```

### Frontend Event Integration

```typescript
// React hook for event subscriptions
export function useEventSubscription<T extends EventType>(
  eventType: T,
  handler: (payload: EventMap[T]) => void,
  deps: React.DependencyList = []
): void {
  useEffect(() => {
    const unsubscribe = eventBus.subscribe(eventType, handler);
    return unsubscribe;
  }, [eventType, ...deps]);
}

// Hook for multiple event types
export function useEventSubscriptions(
  subscriptions: Array<{
    type: EventType;
    handler: (payload: any) => void;
  }>,
  deps: React.DependencyList = []
): void {
  useEffect(() => {
    const unsubscribers = subscriptions.map(({ type, handler }) =>
      eventBus.subscribe(type, handler)
    );
    
    return () => {
      unsubscribers.forEach(unsub => unsub());
    };
  }, [subscriptions, ...deps]);
}

// Global event listener for React Query invalidation
export function EventDrivenCacheInvalidation() {
  const queryClient = useQueryClient();
  
  useEventSubscriptions([
    {
      type: EventType.TASK_CREATED,
      handler: () => {
        queryClient.invalidateQueries({
          queryKey: queryKeys.tasksList(),
        });
      },
    },
    {
      type: EventType.RESULT,
      handler: (payload) => {
        queryClient.invalidateQueries({
          queryKey: queryKeys.task(payload.taskId),
        });
        queryClient.invalidateQueries({
          queryKey: queryKeys.tasksList(),
        });
      },
    },
    {
      type: EventType.SERVER_STARTED,
      handler: (payload) => {
        queryClient.invalidateQueries({
          queryKey: queryKeys.taskServerStatus(payload.taskId),
        });
      },
    },
  ]);
  
  return null; // This is a data-only component
}
```

## Advanced Patterns

### Event Middleware

```typescript
interface EventMiddleware {
  (event: TaskEvent, next: (event: TaskEvent) => void): void;
}

class EventBusWithMiddleware extends TypedEventEmitter<EventMap> {
  private middleware: EventMiddleware[] = [];
  
  use(middleware: EventMiddleware): void {
    this.middleware.push(middleware);
  }
  
  emit<K extends keyof EventMap>(type: K, payload: EventMap[K]): void {
    const event: TaskEvent = {
      id: generateId(),
      type,
      payload,
      timestamp: new Date().toISOString(),
      source: 'application',
    };
    
    this.executeMiddleware(event, 0);
  }
  
  private executeMiddleware(event: TaskEvent, index: number): void {
    if (index >= this.middleware.length) {
      // All middleware executed, emit the event
      super.emit(event.type, event.payload);
      return;
    }
    
    const middleware = this.middleware[index];
    middleware(event, (modifiedEvent) => {
      this.executeMiddleware(modifiedEvent, index + 1);
    });
  }
}

// Middleware examples
const loggingMiddleware: EventMiddleware = (event, next) => {
  console.log('Event emitted:', event.type, event.payload);
  next(event);
};

const validationMiddleware: EventMiddleware = (event, next) => {
  if (!event.payload.taskId) {
    console.error('Event missing taskId:', event);
    return; // Don't continue
  }
  next(event);
};

const timestampMiddleware: EventMiddleware = (event, next) => {
  next({
    ...event,
    timestamp: new Date().toISOString(),
  });
};
```

### Event Aggregation

```typescript
class EventAggregator {
  private aggregatedEvents = new Map<string, TaskEvent[]>();
  private timers = new Map<string, NodeJS.Timeout>();
  
  constructor(
    private eventService: EventService,
    private aggregationWindow = 1000 // 1 second
  ) {}
  
  aggregateEvents(
    aggregationKey: string,
    events: TaskEvent[]
  ): void {
    // Add events to aggregation
    if (!this.aggregatedEvents.has(aggregationKey)) {
      this.aggregatedEvents.set(aggregationKey, []);
    }
    
    this.aggregatedEvents.get(aggregationKey)!.push(...events);
    
    // Reset timer
    if (this.timers.has(aggregationKey)) {
      clearTimeout(this.timers.get(aggregationKey)!);
    }
    
    // Set new timer
    const timer = setTimeout(() => {
      this.flushAggregatedEvents(aggregationKey);
    }, this.aggregationWindow);
    
    this.timers.set(aggregationKey, timer);
  }
  
  private flushAggregatedEvents(aggregationKey: string): void {
    const events = this.aggregatedEvents.get(aggregationKey);
    if (!events || events.length === 0) return;
    
    // Emit aggregated event
    this.eventService.emitTaskEvent({
      id: generateId(),
      type: EventType.MESSAGE,
      subtype: 'aggregated',
      payload: {
        taskId: events[0].payload.taskId,
        events,
        count: events.length,
      },
      timestamp: new Date().toISOString(),
      source: 'aggregator',
    });
    
    // Clean up
    this.aggregatedEvents.delete(aggregationKey);
    this.timers.delete(aggregationKey);
  }
}
```

### Event Replay

```typescript
class EventReplay {
  constructor(private eventService: EventService) {}
  
  async replayEvents(
    taskId: string,
    fromTimestamp?: string
  ): Promise<void> {
    const events = await this.loadStoredEvents(taskId, fromTimestamp);
    
    for (const event of events) {
      // Add replay marker
      const replayEvent = {
        ...event,
        subtype: 'replay',
        replay: {
          originalTimestamp: event.timestamp,
          replayTimestamp: new Date().toISOString(),
        },
      };
      
      await this.eventService.emitTaskEvent(replayEvent);
      
      // Small delay to maintain order
      await new Promise(resolve => setTimeout(resolve, 10));
    }
  }
  
  private async loadStoredEvents(
    taskId: string,
    fromTimestamp?: string
  ): Promise<TaskEvent[]> {
    const eventsDir = path.join('data/events', taskId);
    
    if (!await fs.pathExists(eventsDir)) {
      return [];
    }
    
    const eventFiles = await fs.readdir(eventsDir);
    const events: TaskEvent[] = [];
    
    for (const file of eventFiles) {
      const event = await fs.readJson(path.join(eventsDir, file));
      
      if (!fromTimestamp || event.timestamp >= fromTimestamp) {
        events.push(event);
      }
    }
    
    // Sort by timestamp
    return events.sort((a, b) => 
      new Date(a.timestamp).getTime() - new Date(b.timestamp).getTime()
    );
  }
}
```

## Benefits

### Decoupling
- **Loose Coupling**: Components don't need direct references
- **Independent Development**: Teams can work on different components
- **Easy Testing**: Mock events for isolated testing
- **Flexible Architecture**: Easy to add/remove event handlers

### Scalability
- **Horizontal Scaling**: Event distribution can be scaled
- **Performance**: Async event handling doesn't block
- **Monitoring**: Centralized event logging and metrics
- **Debugging**: Event history for troubleshooting

### Real-time Capabilities
- **Live Updates**: Components react to changes immediately
- **Consistent State**: All components stay synchronized
- **Audit Trail**: Complete history of system changes

## Trade-offs

### Complexity
- More complex debugging (events can be hard to trace)
- Potential for event ordering issues
- Need to handle event replay scenarios
- Memory usage for event history

### Performance Considerations
- Event overhead for high-frequency updates
- Memory usage for large event payloads
- Network overhead for distributed events

## Best Practices

### Do's
- **Use strong typing** for events and payloads
- **Keep events focused** on single concerns
- **Include sufficient context** in event payloads
- **Handle errors gracefully** in event listeners
- **Log events** for debugging and auditing
- **Version events** for backward compatibility

### Don'ts
- Don't make events too granular (creates noise)
- Don't include sensitive data in event payloads
- Don't create circular event dependencies
- Don't block event handlers with long operations
- Don't rely on event ordering across different types

### Event Design Guidelines
```typescript
// Good: Focused, well-structured event
{
  type: EventType.TASK_COMPLETED,
  payload: {
    taskId: 'task-123',
    duration: 5000,
    cost: 0.15,
    resultSummary: 'Task completed successfully'
  }
}

// Bad: Too much unrelated data
{
  type: EventType.TASK_COMPLETED,
  payload: {
    taskId: 'task-123',
    userEmail: 'user@example.com', // Sensitive
    entireTaskHistory: [...], // Too much data
    serverStats: {...} // Unrelated
  }
}
```

## Related Patterns
- [SSE Connection Management](./sse-management.md) - Delivers events to frontend
- [Smart Cache Invalidation](./cache-invalidation.md) - Uses events for cache updates
- [Tri-Context Pattern](./tri-context.md) - Separates event streams from other data

## Further Reading
- [Event-Driven Architecture](https://martinfowler.com/articles/201701-event-driven.html)
- [Domain Events Pattern](https://docs.microsoft.com/en-us/dotnet/architecture/microservices/microservice-ddd-cqrs-patterns/domain-events-design-implementation)
- [Event Sourcing](https://martinfowler.com/eaaDev/EventSourcing.html)