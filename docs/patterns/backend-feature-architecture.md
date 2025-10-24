# Backend Feature Architecture Pattern

## Overview

Defines how `apps/backend` is organised into feature-centric vertical slices while keeping reusable infrastructure (clients, storage primitives, middleware) in shared locations. Each feature owns its API routes, business logic, storage adapter, and utilities; shared layers provide cross-cutting behaviours.

## When to Use

- ✅ Building or refactoring backend capabilities (Auth, Tasks, Workspaces, Uploads, etc.).
- ✅ Migrating legacy services/routes into the feature-first layout.
- ✅ Creating new APIs that must share contracts/schemas with the frontend.
- ❌ Implementing generic helpers or external integrations meant to be reused by multiple features (keep under `shared/` or `infrastructure/`).
- ❌ Working in packages outside the backend app (`packages/*`).

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
        __tests__/
      storage/
        <domain>.storage.ts
      types/
        <domain>.types.ts
      utils/
        <domain>.utils.ts
      index.ts
      __tests__/
```

### Layer Responsibilities

- **Routes (`api/`)**
  - Register HTTP endpoints using contracts from `packages/api-contract`.
  - Perform request/response validation and shape mapping only.
  - Delegate all business logic to services.
  - Avoid direct storage/client usage.

- **Services (`services/`)**
  - Implement application/business logic for the feature.
  - Receive dependencies (storage adapters, other feature facades) via constructor injection.
  - Maintain orchestration rules, domain validations, side-effect coordination.
  - Provide a clear public surface consumed by routes, jobs, or other features.

- **Storage adapters (`storage/`)**
  - Wrap the shared `Storage` interface with feature-specific key structure and type safety.
  - Hide raw key strings and serialization details from services.
  - Remain thin: focus on persistence, not business rules.

- **Types (`types/`)**
  - Store domain-specific TypeScript interfaces and value objects not exposed via API contracts.
  - Importable by services, storage, or utils; never coupled to infrastructure.

- **Utils (`utils/`)**
  - Contain small, feature-scoped helpers (formatters, calculators) that do not fit in services.
  - Stay pure and stateless where possible.

- **Feature entry (`index.ts`)**
  - Export service classes decorated with `@singleton()` or `@injectable()`.
  - Export route registration function that receives resolved services.
  - Services declare dependencies via constructor parameters (auto-injected by TSyringe).
  - No manual instantiation or factory functions needed.

- **Shared layers**
  - `shared/`: cross-cutting middleware, logger setup, env parsing, pure utilities.
  - `infrastructure/clients/`: reusable third-party clients (GitHub, Auth0, Netlify, messaging, telemetry).
  - `infrastructure/storage/`: concrete implementations of the `Storage` interface (File, S3, etc.).

## Implementation Steps

1. **Create feature skeleton** – add `features/<feature>/` with subdirectories and `index.ts`.
2. **Define contracts** – ensure `packages/api-contract/src/<feature>/<domain>.contract.ts` and `.schemas.ts` mirror the API you need; update `packages/types` if new schema types are required.
3. **Implement services** – create `services/<domain>.service.ts` with `@singleton()` decorator. Declare dependencies as constructor parameters (TSyringe auto-injects them).
4. **Create storage adapter** – implement `storage/<name>.storage.ts` using the shared `Storage` interface (`docs/patterns/feature-storage-adapters.md`). Decorate with `@singleton()` if stateless.
5. **Implement routes** – create `api/<domain>.routes.ts` with a `register<Feature>Routes` function that receives the Hono app and resolved service instance. Import contracts, validate input, call service methods.
6. **Export from index.ts** – export service classes and route registration function. No instantiation code needed.
7. **Register feature** – in `app.ts`, resolve service from container (`container.resolve(AuthService)`) and pass to route registration function.
8. **Document & test** – add README snippets or notes about the feature, write unit tests for services/storage using child containers for isolation, ensure route tests align with contracts.

## Example Code

```typescript
// apps/backend/src/features/auth/services/auth.service.ts
import { singleton } from 'tsyringe'
import type { Storage } from '../../../infrastructure/storage/storage.js'
import type { AuthStorage } from '../storage/auth.storage.js'

@singleton()
export class AuthService {
  constructor(
    private storage: Storage,
    private authStorage: AuthStorage
  ) {}

  async login(credentials: LoginCredentials): Promise<Session> {
    // Business logic here
    const session = await this.authStorage.createSession(userId)
    return session
  }
}

// apps/backend/src/features/auth/api/auth.routes.ts
import type { OpenAPIHono } from '@hono/zod-openapi'
import { authLoginRoute } from '@moatless/api-contract/auth'
import type { AppBindings } from '../../../shared/types/app.js'
import type { AuthService } from '../services/auth.service.js'

export function registerAuthRoutes(
  app: OpenAPIHono<AppBindings>,
  authService: AuthService
) {
  app.openapi(authLoginRoute, async (c) => {
    const body = c.req.valid('json')
    const session = await authService.login(body)
    return c.json(session)
  })
}

// apps/backend/src/features/auth/index.ts
export { AuthService } from './services/auth.service.js'
export { registerAuthRoutes } from './api/auth.routes.js'

// apps/backend/src/app.ts
import { container } from 'tsyringe'
import { AuthService, registerAuthRoutes } from './features/auth/index.js'

const authService = container.resolve(AuthService)
registerAuthRoutes(app, authService)
```

## Testing Guidance

- **Routes**: test handlers using Hono test utilities, asserting that contracts and validation behave as expected (use mock services).
- **Services**: unit-test business logic using child containers with mocked dependencies:
  ```typescript
  const testContainer = container.createChildContainer()
  testContainer.register(Storage, { useValue: mockStorage })
  const service = testContainer.resolve(AuthService)
  ```
- **Storage adapters**: test key generation and serialization using the shared `Storage` interface (inject `FileStorage` or in-memory stub).
- **Integration**: optional end-to-end tests can use the global container or create child containers with real storage to ensure wiring works.

## Migration Notes

- For migrating from the legacy service-container pattern, see `docs/patterns/service-container-migration.md`.
- Refactor one feature at a time; keep legacy modules until the new feature passes tests and reviews.
- Services automatically inject dependencies via TSyringe—no manual wiring needed.
- Use constructor injection for all dependencies; TSyringe handles resolution automatically.
- Update documentation (`docs/backend-feature-domains.md`, feature READMEs) once migration completes.

## Related Docs

- `docs/backend-feature-domains.md`
- `docs/patterns/feature-storage-adapters.md`
- `docs/patterns/writing-pattern-documents.md`
