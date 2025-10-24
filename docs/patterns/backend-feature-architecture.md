# Backend Feature Architecture Pattern

## Overview

Defines the target structure for `apps/backend`, shifting from service-centric directories to feature-oriented slices. Ensures routes, business logic, storage adapters, and utilities live together while shared clients and primitives remain reusable.

## When to Use

- ✅ Creating or refactoring backend features (tasks, auth, uploads, etc.).
- ✅ Designing new services that should expose HTTP APIs plus supporting infrastructure.
- ✅ Migrating legacy services into the feature-first layout.
- ❌ Implementing cross-cutting utilities (leave in `shared/` or `infrastructure/`).
- ❌ Building external packages (handled in `packages/*`).

## Directory Layout

```
apps/backend/src/
  app.ts
  shared/
    http/
      middleware/
      hono-helpers.ts
    utils/
      date.utils.ts
      logger.ts
    config/
      env.ts
  infrastructure/clients/
    github.client.ts
    auth0.client.ts
    netlify.client.ts
  infrastructure/storage/
    storage.ts
    file-storage.ts
  features/
    <feature>/
      api/
        <domain>.routes.ts
        __tests__/
      services/
        <domain>.service.ts
      storage/
        <domain>.storage.ts
      types/
        <domain>.types.ts
      utils/
        <domain>.utils.ts
      index.ts
      __tests__/
```

## Implementation Steps

1. **Create feature skeleton** – `features/<feature>/` with `api/`, `services/`, `storage/`, `types/`, `utils/`, and `index.ts`.
2. **Register routes** – place HTTP handlers in `api/<domain>.routes.ts`; consume contracts from `packages/api-contract/src/<feature>/<domain>.contract.ts` and schemas from `.schemas.ts`.
3. **Implement services** – expose use-case surfaces in `services/<domain>.service.ts`; depend on injected adapters only.
4. **Add storage adapter** – implement feature-specific persistence inside `storage/<domain>.storage.ts` using the shared `Storage` interface.
5. **Wire infrastructure** – instantiate shared clients/storage in `index.ts` (or via the DI container) and pass them into services.
6. **Export public API** – `index.ts` returns route registration helpers and facades consumed by `app.ts`.
7. **Update DI/app setup** – register feature routes and middleware via the new entry point; remove old references.

## Example Code

```typescript
// apps/backend/src/features/tasks/index.ts
import type { App } from 'hono'
import { FileStorage } from '../../infrastructure/storage/file-storage.js'
import type { Storage } from '../../infrastructure/storage/storage.js'
import { registerTaskRoutes } from './api/tasks.routes.js'
import { TaskService } from './services/task.service.js'
import { TaskStorage } from './storage/task.storage.js'

export interface TaskFeatureOptions {
  storage?: Storage
}

export function createTaskFeature(options: TaskFeatureOptions = {}) {
  const storage = options.storage ?? new FileStorage()
  const taskStorage = new TaskStorage(storage)
  const taskService = new TaskService(taskStorage)

  return {
    registerRoutes(app: App) {
      registerTaskRoutes(app, taskService)
    },
    taskService,
  }
}
```

## Testing Guidance

- Unit tests live next to implementation (`api/__tests__`, `__tests__` at feature root).
- Test services with injected doubles for storage/clients.
- Integration or contract tests should verify route registration matches `packages/api-contract` definitions.
- Shared test helpers belong in `shared/test-utils/`.

## Migration Notes

- Migrate one feature at a time; keep legacy routes/services until the new slice is ready.
- Maintain dependency flow `api -> services -> storage/clients` to avoid cross-feature coupling.
- Prefer constructor injection for easier testing and future backend swaps (e.g. S3 storage).
- Document each feature’s surface with a `README.md` or checklist during migration.

## Related Docs

- `docs/backend-feature-domains.md`
- `docs/patterns/feature-storage-adapters.md`
- `docs/patterns/writing-pattern-documents.md`
