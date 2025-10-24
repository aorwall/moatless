# Feature Storage Adapter Pattern

Guidance for implementing feature-scoped persistence layers using the shared `Storage` interface (`apps/backend/src/infrastructure/storage/storage.ts`) and wiring different infrastructure providers (file system, S3, etc.).

## Overview

Use this pattern whenever a feature needs to persist JSON data (sessions, tasks, uploads metadata, etc.) without coupling to a concrete storage backend. Features create their own adapter that relies on the shared `Storage` contract; the DI layer decides which concrete implementation (file-based, S3, in-memory) to inject.

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
      storage.ts          # shared interface
      file-storage.ts     # concrete implementation (default)
      <other>.ts          # future providers (e.g. s3-storage.ts)
  features/
    <feature>/
      storage/
        <domain>.storage.ts   # feature-specific adapter (depends on Storage)
      services/
        ... injects the adapter ...
      index.ts
```

## Implementation Steps

1. **Ensure shared interface exists** – `storage.ts` exports the minimal contract.
2. **Implement (or reuse) a provider** – e.g. `FileStorage` already satisfies the interface.
3. **Create feature adapter** – add `features/<feature>/storage/<name>.storage.ts` encapsulating key naming and type safety.
4. **Inject adapter** – in `features/<feature>/index.ts`, instantiate the adapter with a `Storage` implementation.
5. **Expose through services** – pass the feature adapter into services/use-cases instead of raw `Storage`.
6. **Allow overrides** – accept an optional `Storage` in the feature factory for tests or alternative backends.

## Example Code

```typescript
// apps/backend/src/features/auth/storage/session.storage.ts
import type { Storage } from '../../infrastructure/storage/storage.js'

type SessionRecord = {
  id: string
  userId: string
  expiresAt: string
  data: Record<string, unknown>
}

export class SessionStorage {
  constructor(private readonly storage: Storage) {}

  async save(session: SessionRecord) {
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

```typescript
// apps/backend/src/features/auth/index.ts
import { FileStorage } from '../../infrastructure/storage/file-storage.js'
import type { Storage } from '../../infrastructure/storage/storage.js'
import { SessionStorage } from './storage/session.storage.js'

export interface AuthFeatureOptions {
  storage?: Storage
}

export function createAuthFeature(options: AuthFeatureOptions = {}) {
  const storage = options.storage ?? new FileStorage()
  const sessionStorage = new SessionStorage(storage)

  // pass sessionStorage into services, register routes, etc.
  return {
    sessionStorage,
    registerRoutes: (app: App) => { /* ... */ },
  }
}
```

To swap implementations:

```typescript
import { S3Storage } from '../../infrastructure/storage/s3-storage.js'

createAuthFeature({ storage: new S3Storage(s3Config) })
```

## Testing Guidance

- Unit-test the feature adapter with an in-memory `Storage` stub that records calls.
- Integration-test the concrete provider separately (e.g. file storage writing to tmp dir).
- Ensure edge cases (missing keys, invalid JSON) are handled gracefully via `null` results.

## Migration Notes

- Legacy modules can continue using `apps/backend/src/storage` until migrated.
- When converting an existing feature:
  1. Move persistence helpers into `features/<feature>/storage` using this pattern.
  2. Replace direct calls to legacy storage APIs with the new adapter.
  3. Update the service container to provide the adapter via the feature factory.
- Keep both storage systems running during migration by injecting the new adapter only where refactored.

## Related Docs

- `docs/backend-architecture-pattern.md` – overall feature layout & shared layers.
- `docs/backend-feature-domains.md` – current domains and proposed feature boundaries.
