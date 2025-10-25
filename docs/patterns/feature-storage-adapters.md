# Feature Storage Adapter Pattern

Guidance for implementing feature-scoped persistence layers using the infrastructure `Storage` interface (`apps/backend/src/infrastructure/storage/storage.ts`) and wiring different infrastructure providers (file system, S3, etc.) via InversifyJS dependency injection.

## Overview

Use this pattern whenever a feature needs to persist JSON data (sessions, tasks, uploads metadata, etc.) without coupling to a concrete storage backend. Features create their own adapter that relies on the minimal `Storage` interface; InversifyJS injects the concrete implementation (file-based, authorized, etc.) configured at application startup.

**Note:** The legacy `StorageBackend` interface (`apps/backend/src/storage/storage-backend.ts`) is being phased out. New feature modules should use the infrastructure `Storage` interface instead.

## When to Use

- ✅ Feature owns a logical collection (sessions, commands, workspace state) and needs CRUD helpers.
- ✅ Data is JSON-serializable and can be represented via plain TypeScript types.
- ✅ You want the option to switch storage backends (file, S3, database) without rewriting feature logic.
- ❌ Data is relational or requires complex querying — consider dedicated database integrations instead.

## Directory Layout

```
apps/backend/src/
  infrastructure/
    storage/
      storage.ts             # minimal Storage interface + InfrastructureStorageSymbol
  storage/
    storage-backend.ts       # legacy interface (being phased out)
    file-storage.ts          # concrete implementation (default)
    authorized-file-storage.ts
    authorized-group-file-storage.ts
    <other>.ts               # future providers (e.g. s3-storage.ts)
  features/
    <feature>/
      storage/
        <domain>.storage.ts   # feature-specific adapter (depends on infrastructure Storage)
      services/
        <domain>.service.ts   # injects the adapter via InversifyJS
      index.ts
```

## Implementation Steps

1. **Ensure infrastructure interface exists** – `infrastructure/storage/storage.ts` exports the `Storage` interface and `InfrastructureStorageSymbol`.
2. **Implement (or reuse) a provider** – e.g. `FileStorage` already satisfies the interface and is bound to `InfrastructureStorageSymbol` in the container.
3. **Create feature adapter** – add `features/<feature>/storage/<name>.storage.ts` encapsulating key naming and type safety, decorated with `@injectable()`.
4. **Inject storage** – use `@inject(InfrastructureStorageSymbol)` in the adapter constructor to receive the storage implementation.
5. **Bind adapter** – add binding in `container/container.ts` using `.toSelf().inSingletonScope()`.
6. **Use in services** – inject the feature adapter into services via `@inject(AdapterClass)`.

## Example Code

### Feature Storage Adapter

```typescript
// apps/backend/src/features/events/storage/events.storage.ts
import type { Event } from '@moatless/types'
import { inject, injectable } from 'inversify'
import type { Storage } from '../../../infrastructure/storage/storage'
import { InfrastructureStorageSymbol } from '../../../infrastructure/storage/storage'

/**
 * Storage adapter for event persistence.
 * Uses the infrastructure Storage interface to persist events as JSON arrays.
 * Events for each task are stored at key: events/{taskId}
 */
@injectable()
export class EventsStorage {
  constructor(@inject(InfrastructureStorageSymbol) private readonly storage: Storage) {}

  /**
   * Save events for a task
   * @param taskId - Task identifier
   * @param events - Array of events to save
   */
  async saveEvents(taskId: string, events: Event[]): Promise<void> {
    await this.storage.save(this.getKey(taskId), events)
  }

  /**
   * Get all events for a task
   * @param taskId - Task identifier
   * @returns Array of events (empty array if none exist)
   */
  async getEvents(taskId: string): Promise<Event[]> {
    const events = await this.storage.get<Event[]>(this.getKey(taskId))
    return events || []
  }

  /**
   * Generate storage key for task events
   * @param taskId - Task identifier
   * @returns Storage key
   */
  private getKey(taskId: string): string {
    return `events/${taskId}`
  }
}
```

### Container Binding

```typescript
// apps/backend/src/container/container.ts

// 1. Bind infrastructure Storage to InfrastructureStorageSymbol (once during setup)
function createContainer(infrastructure: {
  storage: StorageBackend
  // ... other infrastructure
}): Container {
  const container = new Container()

  // Bind storage instance to infrastructure symbol for new feature modules
  container.bind(InfrastructureStorageSymbol).toConstantValue(infrastructure.storage)

  return container
}

// 2. Bind feature storage adapters
export async function bindServices(container: Container): Promise<void> {
  // ... other imports
  const { EventsStorage } = await import('../features/events/storage/events.storage.js')
  const { EventService } = await import('../features/events/services/event.service.js')

  // Bind storage adapter
  container.bind(EventsStorage).toSelf().inSingletonScope()

  // Bind service that uses the adapter
  container.bind(EventService).toSelf().inSingletonScope()
}
```

### Service Using the Adapter

```typescript
// apps/backend/src/features/events/services/event.service.ts
import { inject, injectable } from 'inversify'
import { EventsStorage } from '../storage/events.storage'

@injectable()
export class EventService {
  constructor(
    @inject(EventsStorage) private readonly eventsStorage: EventsStorage
  ) {}

  async emitEvent(event: Event): Promise<void> {
    // Get existing events from storage (via cache)
    const events = await this.getEvents(event.taskId)

    // Add new event
    events.push(event)

    // Save back to storage
    await this.eventsStorage.saveEvents(event.taskId, events)
  }
}
```

### Feature Exports

```typescript
// apps/backend/src/features/events/index.ts
export { EventsStorage } from './storage/events.storage'
export { EventService } from './services/event.service'
export { registerEventRoutes } from './api/events.routes'
```

Storage backend is switched by changing `STORAGE_TYPE` environment variable:
- `STORAGE_TYPE=user` → AuthorizedFileStorage
- `STORAGE_TYPE=group` → AuthorizedGroupFileStorage
- (default) → FileStorage

All storage backends implement the minimal `Storage` interface and are bound to `InfrastructureStorageSymbol` in the container.

## Testing Guidance

Create isolated test containers with mocked storage:

```typescript
import { Container } from 'inversify'
import { EventsStorage } from '../storage/events.storage'
import type { Storage } from '../../../infrastructure/storage/storage'
import { InfrastructureStorageSymbol } from '../../../infrastructure/storage/storage'

describe('EventsStorage', () => {
  let container: Container
  let eventsStorage: EventsStorage
  let mockStorage: Storage

  beforeEach(() => {
    container = new Container()

    // Mock storage with minimal interface
    mockStorage = {
      save: vi.fn(),
      get: vi.fn(),
      delete: vi.fn(),
      exists: vi.fn(),
      list: vi.fn()
    } as Storage

    // Bind mocks
    container.bind(InfrastructureStorageSymbol).toConstantValue(mockStorage)
    container.bind(EventsStorage).toSelf()

    // Resolve adapter
    eventsStorage = container.get(EventsStorage)
  })

  afterEach(() => {
    container.unbindAll()
  })

  it('should save events with correct key structure', async () => {
    const taskId = 'task-123'
    const events = [{ eventId: '1', type: 'task', subType: 'created', taskId, timestamp: new Date() }]

    await eventsStorage.saveEvents(taskId, events)

    expect(mockStorage.save).toHaveBeenCalledWith('events/task-123', events)
  })
})
```

- Unit-test the feature adapter with mocked `Storage` interface.
- Integration-test the concrete provider separately (e.g. file storage writing to tmp dir).
- Ensure edge cases (missing keys, null results) are handled gracefully via `null` checks.

## Migration Notes

### Migrating from Legacy StorageBackend

When converting an existing feature from the legacy `StorageBackend` to the infrastructure `Storage` interface:

1. **Update imports** – Replace `StorageBackend` and `StorageSymbol` imports with `Storage` and `InfrastructureStorageSymbol` from `infrastructure/storage/storage.ts`.
2. **Update constructor injection** – Change `@inject(StorageSymbol)` to `@inject(InfrastructureStorageSymbol)`.
3. **Update method calls** – If using legacy methods like `saveEvents()` or `getEvents()`, replace with generic `save()` and `get()` methods with proper key structure.
4. **Update tests** – Use mocked `Storage` interface instead of `StorageBackend`.
5. **No container changes needed** – The infrastructure symbol is already bound in `createContainer()`.

### Creating New Feature Storage

When creating a new feature storage adapter:
  1. Create storage adapter in `features/<feature>/storage/<name>.storage.ts` with `@injectable()` decorator.
  2. Inject `Storage` using `@inject(InfrastructureStorageSymbol)` in constructor.
  3. Implement key structure with private `getKey()` helper method.
  4. Add binding to `container/container.ts` using `.toSelf().inSingletonScope()`.
  5. Inject adapter into services using `@inject(AdapterClass)`.

## Related Docs

- `docs/patterns/dependency-injection.md` – InversifyJS usage patterns.
- `docs/patterns/backend-feature-architecture.md` – overall feature layout & shared layers.
