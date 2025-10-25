# Feature Storage Adapter Pattern

Guidance for implementing feature-scoped persistence layers using the shared `StorageBackend` interface (`apps/backend/src/storage/storage-backend.ts`) and wiring different infrastructure providers (file system, S3, etc.) via InversifyJS dependency injection.

## Overview

Use this pattern whenever a feature needs to persist JSON data (sessions, tasks, uploads metadata, etc.) without coupling to a concrete storage backend. Features create their own adapter that relies on the shared `StorageBackend` contract; InversifyJS injects the concrete implementation (file-based, authorized, etc.) configured at application startup.

## When to Use

- ✅ Feature owns a logical collection (sessions, commands, workspace state) and needs CRUD helpers.
- ✅ Data is JSON-serializable and can be represented via plain TypeScript types.
- ✅ You want the option to switch storage backends (file, S3, database) without rewriting feature logic.
- ❌ Data is relational or requires complex querying — consider dedicated database integrations instead.

## Directory Layout

```
apps/backend/src/
  storage/
    storage-backend.ts       # shared interface + StorageSymbol
    file-storage.ts          # concrete implementation (default)
    authorized-file-storage.ts
    authorized-group-file-storage.ts
    <other>.ts               # future providers (e.g. s3-storage.ts)
  features/
    <feature>/
      storage/
        <domain>.storage.ts   # feature-specific adapter (depends on StorageBackend)
      services/
        <domain>.service.ts   # injects the adapter via InversifyJS
      index.ts
```

## Implementation Steps

1. **Ensure shared interface exists** – `storage-backend.ts` exports the `StorageBackend` interface and `StorageSymbol`.
2. **Implement (or reuse) a provider** – e.g. `FileStorage` already satisfies the interface.
3. **Create feature adapter** – add `features/<feature>/storage/<name>.storage.ts` encapsulating key naming and type safety, decorated with `@injectable()`.
4. **Inject storage** – use `@inject(StorageSymbol)` in the adapter constructor to receive the storage backend.
5. **Bind adapter** – add binding in `container/container.ts` using `.toSelf().inSingletonScope()`.
6. **Use in services** – inject the feature adapter into services via `@inject(AdapterClass)`.

## Example Code

### Feature Storage Adapter

```typescript
// apps/backend/src/features/auth/storage/session.storage.ts
import { inject, injectable } from 'inversify'
import type { StorageBackend } from '../../../storage/storage-backend.js'
import { StorageSymbol } from '../../../storage/storage-backend.js'

type SessionRecord = {
  id: string
  userId: string
  expiresAt: string
  data: Record<string, unknown>
}

@injectable()
export class SessionStorage {
  constructor(
    @inject(StorageSymbol) private readonly storage: StorageBackend
  ) {}

  async save(session: SessionRecord): Promise<void> {
    await this.storage.save(this.key(session.id), session)
  }

  async get(id: string): Promise<SessionRecord | null> {
    return this.storage.get<SessionRecord>(this.key(id))
  }

  async delete(id: string): Promise<void> {
    await this.storage.delete(this.key(id))
  }

  async exists(id: string): Promise<boolean> {
    return this.storage.exists(this.key(id))
  }

  private key(id: string) {
    return `sessions/${id}`
  }
}
```

### Container Binding

```typescript
// apps/backend/src/container/container.ts
export async function bindServices(container: Container): Promise<void> {
  // ... other imports
  const { SessionStorage } = await import('../features/auth/storage/session.storage.js')
  const { SessionService } = await import('../features/auth/services/session.service.js')

  // Bind storage adapter
  container.bind(SessionStorage).toSelf().inSingletonScope()

  // Bind service that uses the adapter
  container.bind(SessionService).toSelf().inSingletonScope()
}
```

### Service Using the Adapter

```typescript
// apps/backend/src/features/auth/services/session.service.ts
import { inject, injectable } from 'inversify'
import { SessionStorage } from '../storage/session.storage.js'

@injectable()
export class SessionService {
  constructor(
    @inject(SessionStorage) private sessionStorage: SessionStorage
  ) {}

  async createSession(userId: string): Promise<Session> {
    const session = {
      id: randomUUID(),
      userId,
      expiresAt: new Date(Date.now() + 24 * 60 * 60 * 1000).toISOString(),
      data: {}
    }
    await this.sessionStorage.save(session)
    return session
  }
}
```

### Feature Exports

```typescript
// apps/backend/src/features/auth/index.ts
export { SessionStorage } from './storage/session.storage.js'
export { SessionService } from './services/session.service.js'
export { registerAuthRoutes } from './api/auth.routes.js'
```

Storage backend is switched by changing `STORAGE_TYPE` environment variable:
- `STORAGE_TYPE=user` → AuthorizedFileStorage
- `STORAGE_TYPE=group` → AuthorizedGroupFileStorage
- (default) → FileStorage

## Testing Guidance

Create isolated test containers with mocked storage:

```typescript
import { Container } from 'inversify'
import { SessionStorage } from '../storage/session.storage.js'
import { StorageSymbol } from '../../../storage/storage-backend.js'
import type { StorageBackend } from '../../../storage/storage-backend.js'

describe('SessionStorage', () => {
  let container: Container
  let sessionStorage: SessionStorage
  let mockStorage: StorageBackend

  beforeEach(() => {
    container = new Container()

    // Mock storage backend
    mockStorage = {
      save: vi.fn(),
      get: vi.fn(),
      delete: vi.fn(),
      exists: vi.fn(),
      list: vi.fn()
    } as any

    // Bind mocks
    container.bind(StorageSymbol).toConstantValue(mockStorage)
    container.bind(SessionStorage).toSelf()

    // Resolve service
    sessionStorage = container.get(SessionStorage)
  })

  afterEach(() => {
    container.unbindAll()
  })

  it('should save session with correct key', async () => {
    const session = { id: '123', userId: 'user1', expiresAt: '...', data: {} }

    await sessionStorage.save(session)

    expect(mockStorage.save).toHaveBeenCalledWith('sessions/123', session)
  })
})
```

- Unit-test the feature adapter with mocked `StorageBackend`.
- Integration-test the concrete provider separately (e.g. file storage writing to tmp dir).
- Ensure edge cases (missing keys, invalid JSON) are handled gracefully via `null` results.

## Migration Notes

- When converting an existing feature:
  1. Create storage adapter in `features/<feature>/storage/<name>.storage.ts` with `@injectable()` decorator.
  2. Inject `StorageBackend` using `@inject(StorageSymbol)` in constructor.
  3. Add binding to `container/container.ts` using `.toSelf().inSingletonScope()`.
  4. Inject adapter into services using `@inject(AdapterClass)`.
  5. Replace direct storage calls with adapter methods.

## Related Docs

- `docs/patterns/dependency-injection.md` – InversifyJS usage patterns.
- `docs/patterns/backend-feature-architecture.md` – overall feature layout & shared layers.
