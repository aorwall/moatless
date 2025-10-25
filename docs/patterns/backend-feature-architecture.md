# Backend Feature Architecture Pattern

## Overview

Defines how `apps/backend` is organised into feature-centric vertical slices while keeping reusable infrastructure (clients, storage primitives, middleware) in shared locations. Each feature owns its API routes, business logic, storage adapter, and utilities; InversifyJS manages dependency injection across all layers.

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
  container/
    container.ts          # InversifyJS container setup
    identifiers.ts        # DI symbols for external packages
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
    github.client.ts      # includes GitHubClientSymbol
    auth0.client.ts       # includes Auth0ClientSymbol
    netlify.client.ts
  storage/
    storage-backend.ts    # interface + StorageSymbol
    file-storage.ts
    authorized-file-storage.ts
    authorized-group-file-storage.ts
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
  - Resolve services from `appContainer` directly.
  - Avoid direct storage/client usage.

- **Services (`services/`)**
  - Implement application/business logic for the feature.
  - Decorated with `@injectable()` for InversifyJS auto-wiring.
  - Receive dependencies (storage adapters, other services) via constructor injection using `@inject()`.
  - Maintain orchestration rules, domain validations, side-effect coordination.
  - Provide a clear public surface consumed by routes, jobs, or other features.

- **Storage adapters (`storage/`)**
  - Wrap the shared `StorageBackend` interface with feature-specific key structure and type safety.
  - Decorated with `@injectable()` and inject `StorageBackend` via `@inject(StorageSymbol)`.
  - Hide raw key strings and serialization details from services.
  - Remain thin: focus on persistence, not business rules.

- **Types (`types/`)**
  - Store domain-specific TypeScript interfaces and value objects not exposed via API contracts.
  - Importable by services, storage, or utils; never coupled to infrastructure.

- **Utils (`utils/`)**
  - Contain small, feature-scoped helpers (formatters, calculators) that do not fit in services.
  - Stay pure and stateless where possible.

- **Feature entry (`index.ts`)**
  - Export service classes and storage adapters (all decorated with `@injectable()`).
  - Export route registration function that receives resolved services.
  - No manual instantiation or factory functions needed.

- **Shared layers**
  - `shared/`: cross-cutting middleware, logger setup, env parsing, pure utilities.
  - `infrastructure/clients/`: reusable third-party clients (GitHub, Auth0, Netlify) with colocated DI symbols.
  - `storage/`: concrete implementations of the `StorageBackend` interface with DI symbol.

## Implementation Steps

1. **Create feature skeleton** – add `features/<feature>/` with subdirectories and `index.ts`.
2. **Define contracts** – ensure `packages/api-contract/src/<feature>/<domain>.contract.ts` and `.schemas.ts` mirror the API you need; update `packages/types` if new schema types are required.
3. **Implement services** – create `services/<domain>.service.ts` with `@injectable()` decorator. Declare dependencies as constructor parameters with `@inject()` decorators.
4. **Create storage adapter** – implement `storage/<name>.storage.ts` using the shared `StorageBackend` interface (`docs/patterns/feature-storage-adapters.md`). Decorate with `@injectable()` and inject `StorageBackend` via `@inject(StorageSymbol)`.
5. **Bind services** – add bindings in `container/container.ts` using `container.bind(ServiceClass).toSelf().inSingletonScope()`.
6. **Implement routes** – create `api/<domain>.routes.ts` with a `register<Feature>Routes` function that receives the Hono app. Import `appContainer` and resolve services using `appContainer.get(ServiceClass)`. Import contracts, validate input, call service methods.
7. **Export from index.ts** – export service classes, storage adapters, and route registration function.
8. **Register feature** – in `app.ts`, import and call route registration function (services are auto-resolved by routes).
9. **Document & test** – add README snippets or notes about the feature, write unit tests for services/storage using isolated containers, ensure route tests align with contracts.

## Example Code

### Storage Adapter

```typescript
// apps/backend/src/features/auth/storage/session.storage.ts
import { inject, injectable } from 'inversify'
import type { StorageBackend } from '../../../storage/storage-backend.js'
import { StorageSymbol } from '../../../storage/storage-backend.js'

type SessionRecord = {
  id: string
  userId: string
  expiresAt: string
}

@injectable()
export class SessionStorage {
  constructor(
    @inject(StorageSymbol) private readonly storage: StorageBackend
  ) {}

  async save(session: SessionRecord): Promise<void> {
    await this.storage.save(`sessions/${session.id}`, session)
  }

  async get(id: string): Promise<SessionRecord | null> {
    return this.storage.get<SessionRecord>(`sessions/${id}`)
  }

  async delete(id: string): Promise<void> {
    await this.storage.delete(`sessions/${id}`)
  }
}
```

### Service

```typescript
// apps/backend/src/features/auth/services/auth.service.ts
import { inject, injectable } from 'inversify'
import { SessionStorage } from '../storage/session.storage.js'
import type { GitHubClient } from '../../../infrastructure/clients/github.client.js'
import { GitHubClientSymbol } from '../../../infrastructure/clients/github.client.js'

@injectable()
export class AuthService {
  constructor(
    @inject(SessionStorage) private sessionStorage: SessionStorage,
    @inject(GitHubClientSymbol) private githubClient: GitHubClient
  ) {}

  async login(credentials: LoginCredentials): Promise<Session> {
    // Business logic here
    const userId = await this.authenticateUser(credentials)
    const session = {
      id: randomUUID(),
      userId,
      expiresAt: new Date(Date.now() + 24 * 60 * 60 * 1000).toISOString()
    }
    await this.sessionStorage.save(session)
    return session
  }

  private async authenticateUser(credentials: LoginCredentials): Promise<string> {
    // Use githubClient for authentication
    // ...
  }
}
```

### Container Binding

```typescript
// apps/backend/src/container/container.ts
export async function bindServices(container: Container): Promise<void> {
  // ... other imports
  const { AuthService } = await import('../features/auth/services/auth.service.js')
  const { SessionStorage } = await import('../features/auth/storage/session.storage.js')

  // Bind storage adapter and service with singleton scope
  container.bind(SessionStorage).toSelf().inSingletonScope()
  container.bind(AuthService).toSelf().inSingletonScope()
}
```

### Routes

```typescript
// apps/backend/src/features/auth/api/auth.routes.ts
import type { OpenAPIHono } from '@hono/zod-openapi'
import { authLoginRoute } from '@moatless/api-contract/auth'
import { appContainer } from '../../../container/container.js'
import { AuthService } from '../services/auth.service.js'
import type { AppBindings } from '../../../app.js'

export function registerAuthRoutes(app: OpenAPIHono<AppBindings>) {
  app.openapi(authLoginRoute, async (c) => {
    const body = c.req.valid('json')

    // Resolve service from container
    const authService = appContainer.get(AuthService)
    const session = await authService.login(body)

    return c.json(session)
  })
}
```

### Feature Exports

```typescript
// apps/backend/src/features/auth/index.ts
export { AuthService } from './services/auth.service.js'
export { SessionStorage } from './storage/session.storage.js'
export { registerAuthRoutes } from './api/auth.routes.js'
```

### App Registration

```typescript
// apps/backend/src/app.ts
import { registerAuthRoutes } from './features/auth/index.js'

export async function createApp() {
  const app = new OpenAPIHono()

  // Initialize InversifyJS container with all services
  await initializeAppContainer()

  // Register routes (services are auto-resolved inside route handlers)
  registerAuthRoutes(app)

  return { app }
}
```

## Testing Guidance

### Service Tests

Unit-test business logic using isolated containers with mocked dependencies:

```typescript
import { Container } from 'inversify'
import { AuthService } from '../services/auth.service.js'
import { SessionStorage } from '../storage/session.storage.js'
import { GitHubClientSymbol } from '../../../infrastructure/clients/github.client.js'

describe('AuthService', () => {
  let container: Container
  let authService: AuthService
  let mockSessionStorage: SessionStorage
  let mockGitHubClient: GitHubClient

  beforeEach(() => {
    container = new Container()

    // Create mocks
    mockSessionStorage = {
      save: vi.fn(),
      get: vi.fn(),
      delete: vi.fn()
    } as any

    mockGitHubClient = {
      getUser: vi.fn()
    } as any

    // Bind mocks
    container.bind(SessionStorage).toConstantValue(mockSessionStorage)
    container.bind(GitHubClientSymbol).toConstantValue(mockGitHubClient)

    // Bind service under test
    container.bind(AuthService).toSelf()

    // Resolve service
    authService = container.get(AuthService)
  })

  afterEach(() => {
    container.unbindAll()
  })

  it('should create session on login', async () => {
    mockGitHubClient.getUser.mockResolvedValue({ id: 'user123' })

    const session = await authService.login({ username: 'test', password: 'pass' })

    expect(session).toBeDefined()
    expect(mockSessionStorage.save).toHaveBeenCalledWith(
      expect.objectContaining({ userId: 'user123' })
    )
  })
})
```

### Storage Adapter Tests

Test key generation and serialization using mocked `StorageBackend`:

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

    mockStorage = {
      save: vi.fn(),
      get: vi.fn(),
      delete: vi.fn(),
      exists: vi.fn(),
      list: vi.fn()
    } as any

    container.bind(StorageSymbol).toConstantValue(mockStorage)
    container.bind(SessionStorage).toSelf()

    sessionStorage = container.get(SessionStorage)
  })

  afterEach(() => {
    container.unbindAll()
  })

  it('should use correct key format', async () => {
    const session = { id: '123', userId: 'user1', expiresAt: '...' }

    await sessionStorage.save(session)

    expect(mockStorage.save).toHaveBeenCalledWith('sessions/123', session)
  })
})
```

### Route Tests

Test handlers using Hono test utilities with mock services:

```typescript
import { describe, it, expect, beforeEach } from 'vitest'
import { OpenAPIHono } from '@hono/zod-openapi'
import { registerAuthRoutes } from '../api/auth.routes.js'
import { appContainer } from '../../../container/container.js'
import { AuthService } from '../services/auth.service.js'

// Mock the container before importing routes
vi.mock('../../../container/container.js', () => ({
  appContainer: {
    get: vi.fn()
  }
}))

describe('Auth Routes', () => {
  let app: OpenAPIHono
  let mockAuthService: AuthService

  beforeEach(() => {
    app = new OpenAPIHono()

    mockAuthService = {
      login: vi.fn()
    } as any

    vi.mocked(appContainer.get).mockReturnValue(mockAuthService)

    registerAuthRoutes(app)
  })

  it('should call authService.login', async () => {
    mockAuthService.login.mockResolvedValue({ id: 'session123', userId: 'user1', expiresAt: '...' })

    const res = await app.request('/auth/login', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ username: 'test', password: 'pass' })
    })

    expect(res.status).toBe(200)
    expect(mockAuthService.login).toHaveBeenCalledWith({ username: 'test', password: 'pass' })
  })
})
```

### Integration Tests

Optional end-to-end tests can use real container with test infrastructure:

```typescript
import { Container } from 'inversify'
import { initializeContainer } from '../../../container/container.js'
import { AuthService } from '../services/auth.service.js'
import { FileStorage } from '../../../storage/file-storage.ts'

describe('Auth Feature Integration', () => {
  let container: Container

  beforeAll(async () => {
    // Create test storage in tmp directory
    const testStorage = new FileStorage('/tmp/test-auth')

    // Initialize container with test infrastructure
    container = await initializeContainer({
      storage: testStorage,
      // ... other test infrastructure
    })
  })

  it('should persist session to storage', async () => {
    const authService = container.get(AuthService)

    const session = await authService.login({ username: 'test', password: 'pass' })

    // Verify session was actually saved
    expect(session.id).toBeDefined()
  })
})
```

## Dependency Injection Patterns

### Service Dependencies

**For Interface Dependencies** (use Symbol):
```typescript
@injectable()
export class MyService {
  constructor(
    @inject(StorageSymbol) private storage: StorageBackend,
    @inject(GitHubClientSymbol) private github: GitHubClient
  ) {}
}
```

**For Class Dependencies** (use Class reference):
```typescript
@injectable()
export class MyService {
  constructor(
    @inject(EventService) private events: EventService,
    @inject(TaskService) private tasks: TaskService
  ) {}
}
```

**Critical**: EVERY constructor parameter needs `@inject()` - InversifyJS does not auto-resolve.

### Adding New Symbols

When creating a new interface that needs injection, colocate the symbol with the interface:

```typescript
// apps/backend/src/services/my-service.interface.ts
import type { ServiceIdentifier } from 'inversify'

export const MyServiceSymbol: ServiceIdentifier<MyService> = Symbol.for('MyService')

export interface MyService {
  doSomething(): Promise<void>
}
```

Then bind in `container/container.ts`:
```typescript
container.bind(MyServiceSymbol).toConstantValue(myServiceInstance)
```

## Migration Notes

- Refactor one feature at a time; keep legacy modules until the new feature passes tests and reviews.
- Services automatically inject dependencies via InversifyJS—no manual wiring needed.
- Use constructor injection for all dependencies; InversifyJS handles resolution automatically.
- Each service must be bound in `container/container.ts` using `.toSelf().inSingletonScope()`.
- Update documentation (`docs/backend-feature-domains.md`, feature READMEs) once migration completes.

## Related Docs

- `docs/patterns/dependency-injection.md` - Complete InversifyJS usage guide
- `docs/patterns/feature-storage-adapters.md` - Storage adapter implementation
- `docs/patterns/writing-pattern-documents.md` - Pattern documentation guidelines
