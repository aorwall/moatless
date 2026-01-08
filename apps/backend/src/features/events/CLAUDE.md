# Events Feature

This module handles event emission, persistence, and real-time delivery to SSE clients.

## Architecture Overview

```
                    EVENT SOURCES
    ┌─────────────────────────────────────────────┐
    │  Local Mode          │   Kubernetes Mode    │
    │  (in-process)        │   (worker pods)      │
    │                      │                      │
    │  Agent, Task,        │   Agent events       │
    │  Preview events      │   Preview events     │
    └──────────┬───────────┴──────────┬───────────┘
               │                      │
               │                      │ INSERT status='pending'
               │                      ▼
               │              ┌───────────────┐
               │              │  PostgreSQL   │
               │              │  task_events  │
               │              └───────┬───────┘
               │                      │ NOTIFY trigger
               │                      ▼
               │              ┌───────────────────┐
               │              │ NotifyListener    │
               │              │ Service           │
               │              └───────┬───────────┘
               │                      │
               └──────────┬───────────┘
                          ▼
                 ┌─────────────────┐
                 │  EventService   │
                 │  (Central Hub)  │
                 └────────┬────────┘
                          │
          ┌───────────────┼───────────────┐
          ▼               ▼               ▼
    ┌──────────┐   ┌──────────┐   ┌──────────────┐
    │ Database │   │  Cache   │   │ SSE Broadcast│
    │ Storage  │   │ (Memory) │   │ (Clients)    │
    └──────────┘   └──────────┘   └──────────────┘
```

## Key Files

| File | Purpose |
|------|---------|
| `services/event.service.ts` | Central event hub - emission, caching, subscriptions |
| `services/notify-listener.service.ts` | PostgreSQL NOTIFY/LISTEN for cross-process delivery |
| `services/event-poller.service.ts` | Fallback poller for pending events (if NOTIFY fails) |
| `db/event.repository.ts` | Database operations for task_events table |
| `storage/events.storage.ts` | File-based storage (SQLite mode fallback) |
| `utils/sse-manager.ts` | Server-Sent Events connection management |

## EventService Modes

### Local Mode (SQLite or in-process PostgreSQL)
```typescript
// Events are saved and broadcast immediately
await eventService.emitEvent(event)
// → Saves to storage
// → Updates cache
// → Broadcasts via super.emit('global', event)
// → SSE clients receive event
```

### Worker Mode (Kubernetes pods)
```typescript
// Worker pod: saves as pending, triggers NOTIFY
await eventService.emitEvent(event)
// → savePendingEvent() with status='pending'
// → PostgreSQL trigger fires pg_notify('task_events', {...})

// Main app: NotifyListenerService receives NOTIFY
// → Fetches full event from DB
// → Calls eventService.emitEvent(event)
// → Marks as delivered
// → SSE clients receive event
```

## Event Flow (PostgreSQL + Kubernetes)

1. **Worker inserts event** → `eventRepository.savePendingEvent()`
2. **Database trigger fires** → `pg_notify('task_events', {eventId, taskId, type})`
3. **NotifyListenerService receives** → `sql.listen('task_events', callback)`
4. **Fetches full event** → `eventRepository.getEventById(eventId)`
5. **Broadcasts locally** → `eventService.emitEvent(event)`
6. **Marks delivered** → `eventRepository.markAsDelivered(eventId)`
7. **SSE clients receive** → via `super.emit('global', event)`

## Important Behaviors

### Deduplication
EventService tracks recently emitted eventIds (60s TTL) to prevent infinite loops:
```typescript
if (this.recentEventIds.has(event.eventId)) {
  return // Skip duplicate
}
```

### markDeliveredOnSave Flag
- `true` (main app): Events saved as already delivered
- `false` (worker): Events saved as pending for NOTIFY delivery

```typescript
this.markDeliveredOnSave = process.env['WORKER_MODE'] !== 'true'
```

### Fallback Poller
If NOTIFY fails, `EventPollerService` polls for pending events every 30s:
```typescript
// Queries: WHERE delivered_at IS NULL
const pendingEvents = await eventRepository.getPendingEvents()
```

## Subscribing to Events

```typescript
// Subscribe to specific task
const unsubscribe = eventService.subscribe(taskId, (event) => {
  console.log('Task event:', event)
})

// Subscribe to all events (global)
const unsubscribe = eventService.subscribeAll((event) => {
  console.log('Global event:', event)
})
```

## Event Types

Events are typed in `@moatless/types`:
- `agent` - Agent lifecycle (started, stopped, message, completed, error)
- `task` - Task lifecycle (created, open, closed, updated)
- `taskIteration` - Iteration lifecycle (created, active, completed, error)
- `server` - Preview server (starting, started, stopped, failed)
- `workspace` - Workspace (provisioning, ready, error)
- `preview` - Preview UI (started, stopped, failed, screenshot)

## Database Schema

```sql
CREATE TABLE task_events (
  id SERIAL PRIMARY KEY,
  event_id VARCHAR UNIQUE NOT NULL,
  task_id VARCHAR NOT NULL,
  type VARCHAR NOT NULL,
  sub_type VARCHAR,
  payload JSONB,
  is_error BOOLEAN DEFAULT FALSE,
  timestamp TIMESTAMP NOT NULL,
  delivered_at TIMESTAMP,  -- NULL = pending
  created_at TIMESTAMP DEFAULT NOW()
);

-- Trigger for NOTIFY
CREATE TRIGGER task_events_notify_trigger
  AFTER INSERT ON task_events
  FOR EACH ROW
  EXECUTE FUNCTION notify_task_event();
```

## Common Operations

### Emit an event
```typescript
await eventService.emitEvent({
  eventId: randomUUID(),
  taskId: 'task-123',
  type: 'server',
  subType: 'started',
  timestamp: new Date(),
  isError: false,
  payload: { port: 3000, pid: 1234 },
})
```

### Get events for a task
```typescript
const events = await eventService.getEvents(taskId, limit, offset, eventType)
```

### Delete events for a task
```typescript
await eventService.deleteEvents(taskId)
```

## Testing Considerations

- Mock `EventService` for unit tests
- Use real `EventService` with in-memory cache for integration tests
- For NOTIFY testing, need real PostgreSQL connection
- Deduplication can cause test issues if same eventId reused within 60s
