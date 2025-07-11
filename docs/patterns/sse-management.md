# SSE Connection Management Pattern

## Problem

Managing Server-Sent Events (SSE) connections in React applications presents challenges:
- Multiple components need real-time updates
- Connection lifecycle management (connect, reconnect, cleanup)
- Preventing duplicate connections
- Efficient event distribution to subscribers
- Type-safe event handling

## Solution

A two-layer SSE architecture:
1. **Global Event Stream**: Single app-level connection
2. **Feature Subscribers**: Components subscribe to specific events

This ensures one connection per app while allowing granular subscriptions.

## Implementation

### Global Event Stream Provider

```typescript
// apps/frontend/src/features/events/contexts/global-event-stream-provider.tsx
export function GlobalEventStreamProvider({ children }: PropsWithChildren) {
  const eventSourceRef = useRef<EventSource | null>(null);
  const subscribersRef = useRef<Map<string, Set<EventSubscriber>>>(new Map());
  
  const connect = useCallback(() => {
    if (eventSourceRef.current?.readyState === EventSource.OPEN) {
      return;
    }
    
    const eventSource = new EventSource('/api/events/stream');
    
    eventSource.onopen = () => {
      console.log('SSE connection established');
    };
    
    eventSource.onmessage = (event) => {
      try {
        const data = JSON.parse(event.data) as StreamEvent;
        
        // Distribute to all subscribers
        const globalSubscribers = subscribersRef.current.get('*') || new Set();
        globalSubscribers.forEach(subscriber => subscriber(data));
        
        // Distribute to specific subscribers
        if (data.taskId) {
          const taskSubscribers = subscribersRef.current.get(data.taskId) || new Set();
          taskSubscribers.forEach(subscriber => subscriber(data));
        }
      } catch (error) {
        console.error('Failed to parse SSE message:', error);
      }
    };
    
    eventSource.onerror = () => {
      console.error('SSE connection error, reconnecting...');
      eventSource.close();
      
      // Exponential backoff reconnection
      setTimeout(() => {
        connect();
      }, 5000);
    };
    
    eventSourceRef.current = eventSource;
  }, []);
  
  const subscribe = useCallback((key: string, callback: EventSubscriber) => {
    if (!subscribersRef.current.has(key)) {
      subscribersRef.current.set(key, new Set());
    }
    
    subscribersRef.current.get(key)!.add(callback);
    
    // Connect on first subscription
    if (eventSourceRef.current === null) {
      connect();
    }
    
    // Return unsubscribe function
    return () => {
      const subscribers = subscribersRef.current.get(key);
      if (subscribers) {
        subscribers.delete(callback);
        if (subscribers.size === 0) {
          subscribersRef.current.delete(key);
        }
      }
      
      // Disconnect if no subscribers
      if (subscribersRef.current.size === 0 && eventSourceRef.current) {
        eventSourceRef.current.close();
        eventSourceRef.current = null;
      }
    };
  }, [connect]);
  
  // Cleanup on unmount
  useEffect(() => {
    return () => {
      eventSourceRef.current?.close();
    };
  }, []);
  
  const value = useMemo(() => ({
    subscribe,
    isConnected: eventSourceRef.current?.readyState === EventSource.OPEN,
  }), [subscribe]);
  
  return (
    <GlobalEventStreamContext.Provider value={value}>
      {children}
    </GlobalEventStreamContext.Provider>
  );
}
```

### Feature-Level Hook

```typescript
// apps/frontend/src/features/tasks/hooks/use-task-sse-connection.ts
export function useTaskSSEConnection(taskId: string) {
  const { subscribe } = useGlobalEventStream();
  const [events, setEvents] = useState<TaskEvent[]>([]);
  const [isConnected, setIsConnected] = useState(false);
  const [error, setError] = useState<Error | null>(null);
  
  useEffect(() => {
    if (!taskId) return;
    
    const handleEvent = (event: StreamEvent) => {
      if (event.type === 'task.event' && event.payload.taskId === taskId) {
        setEvents(prev => [...prev, event.payload]);
        setIsConnected(true);
        setError(null);
      }
    };
    
    // Subscribe to task-specific events
    const unsubscribe = subscribe(taskId, handleEvent);
    
    return () => {
      unsubscribe();
    };
  }, [taskId, subscribe]);
  
  return { events, isConnected, error };
}
```

### Backend Implementation

```typescript
// apps/backend/routes/events.ts
export const eventsRoutes = new Hono()
  .get('/stream', async (c) => {
    const stream = c.streamSSE();
    
    // Send heartbeat every 30 seconds
    const heartbeat = setInterval(() => {
      stream.writeSSE({
        event: 'heartbeat',
        data: JSON.stringify({ timestamp: Date.now() }),
      });
    }, 30000);
    
    // Subscribe to global events
    const unsubscribe = eventBus.subscribe((event) => {
      stream.writeSSE({
        event: event.type,
        data: JSON.stringify(event),
        id: event.id,
      });
    });
    
    // Cleanup on disconnect
    stream.onAbort(() => {
      clearInterval(heartbeat);
      unsubscribe();
    });
    
    // Keep connection alive
    await stream.sleep(Infinity);
  });
```

## Advanced Features

### Automatic Reconnection

```typescript
interface ReconnectionConfig {
  maxRetries: number;
  baseDelay: number;
  maxDelay: number;
  backoffFactor: number;
}

function useSSEWithReconnection(url: string, config: ReconnectionConfig) {
  const [retryCount, setRetryCount] = useState(0);
  const [isConnecting, setIsConnecting] = useState(false);
  
  const connect = useCallback(() => {
    if (isConnecting) return;
    
    setIsConnecting(true);
    const eventSource = new EventSource(url);
    
    eventSource.onerror = () => {
      eventSource.close();
      
      if (retryCount < config.maxRetries) {
        const delay = Math.min(
          config.baseDelay * Math.pow(config.backoffFactor, retryCount),
          config.maxDelay
        );
        
        setTimeout(() => {
          setRetryCount(prev => prev + 1);
          connect();
        }, delay);
      }
    };
    
    eventSource.onopen = () => {
      setRetryCount(0);
      setIsConnecting(false);
    };
    
    return eventSource;
  }, [url, retryCount, config, isConnecting]);
  
  // ... rest of implementation
}
```

### Event Deduplication

```typescript
function useDeduplicatedEvents(events: Event[]) {
  const seenIds = useRef(new Set<string>());
  
  return useMemo(() => {
    return events.filter(event => {
      if (seenIds.current.has(event.id)) {
        return false;
      }
      seenIds.current.add(event.id);
      return true;
    });
  }, [events]);
}
```

### Type-Safe Events

```typescript
// Shared event types
type EventMap = {
  'task.created': { taskId: string; title: string };
  'task.updated': { taskId: string; changes: Partial<Task> };
  'task.deleted': { taskId: string };
  'task.event': { taskId: string; event: TaskEvent };
};

type EventType = keyof EventMap;
type EventPayload<T extends EventType> = EventMap[T];

// Type-safe event emitter
class TypedEventEmitter {
  emit<T extends EventType>(type: T, payload: EventPayload<T>) {
    // Implementation
  }
  
  on<T extends EventType>(
    type: T, 
    handler: (payload: EventPayload<T>) => void
  ) {
    // Implementation
  }
}
```

## Best Practices

### Connection Management
- **Single Connection**: One SSE connection per app
- **Lazy Connection**: Connect only when needed
- **Cleanup**: Always close connections on unmount
- **Heartbeat**: Keep connections alive with periodic pings

### Error Handling
- **Automatic Reconnection**: With exponential backoff
- **Error Boundaries**: Catch and display connection errors
- **Fallback**: Provide polling fallback for SSE-unsupported browsers

### Performance
- **Event Filtering**: Subscribe only to needed events
- **Debouncing**: Batch rapid updates
- **Memory Management**: Limit event history size

## Common Patterns

### 1. Task-Specific Events

```typescript
// Subscribe to events for a specific task
function TaskEventStream({ taskId }: { taskId: string }) {
  const { events } = useTaskSSEConnection(taskId);
  
  return (
    <div>
      {events.map(event => (
        <EventItem key={event.id} event={event} />
      ))}
    </div>
  );
}
```

### 2. Global Notifications

```typescript
// Subscribe to all events for notifications
function NotificationCenter() {
  const { subscribe } = useGlobalEventStream();
  const [notifications, setNotifications] = useState<Notification[]>([]);
  
  useEffect(() => {
    return subscribe('*', (event) => {
      if (event.type === 'task.completed' || event.type === 'task.failed') {
        setNotifications(prev => [...prev, createNotification(event)]);
      }
    });
  }, [subscribe]);
  
  return <NotificationList notifications={notifications} />;
}
```

### 3. Connection Status

```typescript
// Show connection status to user
function ConnectionIndicator() {
  const { isConnected } = useGlobalEventStream();
  
  return (
    <div className={cn(
      'connection-indicator',
      isConnected ? 'connected' : 'disconnected'
    )}>
      {isConnected ? 'Live' : 'Reconnecting...'}
    </div>
  );
}
```

## Troubleshooting

### Common Issues

1. **Multiple Connections**: Check for duplicate providers
2. **Memory Leaks**: Ensure cleanup in useEffect
3. **Lost Events**: Implement event ID tracking
4. **CORS Issues**: Configure SSE endpoint correctly

### Debug Tools

```typescript
// SSE connection debugger
if (process.env.NODE_ENV === 'development') {
  window.__SSE_DEBUG__ = {
    connections: eventSourceRef.current,
    subscribers: subscribersRef.current,
    reconnectCount: retryCount,
  };
}
```
