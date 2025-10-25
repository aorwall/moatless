# Dependency Injection with InversifyJS

## Overview

This project uses [InversifyJS](https://inversify.io/) for dependency injection. InversifyJS uses decorators and explicit container binding to manage service dependencies.

## Setup Requirements

### TypeScript Configuration

InversifyJS requires these compiler options in `tsconfig.json`:

```json
{
  "compilerOptions": {
    "experimentalDecorators": true,
    "emitDecoratorMetadata": true
  }
}
```

### Application Bootstrap

Import `reflect-metadata` at the application entry point before any decorated classes:

```typescript
// apps/backend/src/app.ts
import 'reflect-metadata'
```

## Core Decorators

### @injectable()

**Required on ALL service classes** to mark them for dependency injection:

```typescript
import { injectable } from 'inversify'

@injectable()
export class TaskService {
  constructor(/* dependencies */) {}
}
```

### @inject()

**Required for EVERY constructor parameter** to specify what to inject:

```typescript
import { inject, injectable } from 'inversify'
import { StorageSymbol } from '../container/identifiers.js'
import { EventService } from './event.service.js'

@injectable()
export class TaskService {
  constructor(
    @inject(StorageSymbol) private storage: StorageBackend,      // Interface - use Symbol
    @inject(EventService) private eventService: EventService     // Class - use Class reference
  ) {}
}
```

## Symbol-Based Service Identifiers

### Using Symbols for Interfaces

TypeScript interfaces don't exist at runtime, so use Symbol identifiers for interface injection.

**Pattern**: Symbols are colocated with their interface definitions for better cohesion.

```typescript
// apps/backend/src/storage/storage-backend.ts
import type { ServiceIdentifier } from 'inversify'

/**
 * DI symbol for StorageBackend interface.
 * Use this symbol to inject storage implementations.
 */
export const StorageSymbol: ServiceIdentifier<StorageBackend> = Symbol.for('Storage')

export interface StorageBackend {
  // ... interface definition
}
```

### Adding New Symbols

When creating a new interface that needs injection:

1. Define the symbol in the same file as the interface:
```typescript
// apps/backend/src/services/my-service.interface.ts
import type { ServiceIdentifier} from 'inversify'

export const MyServiceSymbol: ServiceIdentifier<MyService> = Symbol.for('MyService')

export interface MyService {
  doSomething(): Promise<void>
}
```

2. Import and bind in `container/container.ts`:
```typescript
import { MyServiceSymbol } from '../services/my-service.interface.js'

container.bind(MyServiceSymbol).toConstantValue(myServiceInstance)
```

## Service Implementation Pattern

### Basic Service

```typescript
// apps/backend/src/services/task.service.ts
import { inject, injectable } from 'inversify'
import { StorageSymbol } from '../storage/storage-backend.js'
import type { StorageBackend } from '../storage/storage-backend.js'
import { EventService } from './event.service.js'
import { WorkspaceService } from './workspace.service.js'

@injectable()
export class TaskService {
  constructor(
    @inject(StorageSymbol) private storage: StorageBackend,           // Interface with Symbol
    @inject(EventService) private eventService: EventService,         // Concrete class
    @inject(WorkspaceService) private workspaceService: WorkspaceService  // Concrete class
  ) {}

  async createTask(input: CreateTaskInput): Promise<Task> {
    const task = await this.storage.save('tasks', input)
    await this.eventService.emit('task.created', { taskId: task.id })
    return task
  }
}
```

### Injection Rules

**For Interface Dependencies**:
```typescript
@inject(StorageSymbol) private storage: StorageBackend  // Use Symbol from interface file
```

**For Class Dependencies**:
```typescript
@inject(EventService) private eventService: EventService  // Use Class reference directly
```

**Critical**: EVERY constructor parameter needs `@inject()` - no auto-resolution.

## Container Configuration

### Container Setup

**Location**: `apps/backend/src/container/container.ts`

The container configuration has two phases:

#### Phase 1: Infrastructure Binding

```typescript
export function createContainer(infrastructure: {
  storage: StorageBackend
  githubClient: GitHubClient
  auth0Client: Auth0Client
  previewRegistry: PreviewServerRegistry
  oauthProviders: Map<string, OAuthProvider>
}): Container {
  const container = new Container()

  // Bind infrastructure with toConstantValue (pre-configured instances)
  container.bind(StorageSymbol).toConstantValue(infrastructure.storage)
  container.bind(GitHubClientSymbol).toConstantValue(infrastructure.githubClient)
  container.bind(Auth0ClientSymbol).toConstantValue(infrastructure.auth0Client)
  container.bind('PreviewServerRegistry').toConstantValue(infrastructure.previewRegistry)
  container.bind(OAuthProvidersSymbol).toConstantValue(infrastructure.oauthProviders)

  return container
}
```

#### Phase 2: Service Binding

```typescript
export async function bindServices(container: Container): Promise<void> {
  // Import service classes
  const { TaskService } = await import('../services/task.service.js')
  const { EventService } = await import('../services/event.service.js')
  // ... other imports

  // Bind services with singleton scope
  container.bind(TaskService).toSelf().inSingletonScope()
  container.bind(EventService).toSelf().inSingletonScope()
  // ... other bindings
}
```

### Binding Patterns

**Infrastructure** (pre-configured instances):
```typescript
container.bind(StorageSymbol).toConstantValue(storageInstance)
```

**Services** (managed by container):
```typescript
container.bind(TaskService).toSelf().inSingletonScope()
```

**Scopes**:
- `.inSingletonScope()` - Same instance throughout application lifecycle (default for services)
- `.inRequestScope()` - New instance per request
- `.inTransientScope()` - New instance on every resolution

## Application Bootstrap

**Container Setup Location**: `apps/backend/src/container/container.ts`
**App Entry Point**: `apps/backend/src/app.ts`

### Bootstrap Flow

The `initializeAppContainer()` function is defined in `container/container.ts` and handles all DI setup:

```typescript
// In container/container.ts
export async function initializeAppContainer(): Promise<void> {
  // 1. Create infrastructure dependencies
  const storage = await createStorageBackend()
  const githubClient = createGitHubClient()
  const auth0Client = createAuth0Client()
  const previewRegistry = new PreviewServerRegistry(/* ... */)
  const oauthProviders = new Map([
    ['github', createGitHubOAuthProvider(githubClient)],
    ['auth0', createAuth0OAuthProvider(auth0Client)]
  ])

  // 2. Initialize global container with infrastructure and services
  await initializeContainer({
    storage,
    githubClient,
    auth0Client,
    previewRegistry,
    oauthProviders
  })

  // 3. Handle post-initialization setup (setter injection for circular deps)
  const repository = appContainer.get(RepositoryService)
  const claude = appContainer.get(ClaudeService)
  const settings = appContainer.get(SettingsService)
  const github = appContainer.get(GitHubService)

  repository.setClaudeService(claude)
  repository.setSettingsService(settings)
  repository.setGitHubService(github)
  // ... other setter injections
}
```

The application entry point imports and calls this function:

```typescript
// In app.ts
import 'reflect-metadata'
import { initializeAppContainer } from './container/container.js'

export async function createApp() {
  const app = new OpenAPIHono()

  // Initialize container before registering routes
  await initializeAppContainer()

  // Register routes
  await registerRoutes(app)

  return { app }
}
```

### Service Resolution

**Resolve from global container**:
```typescript
import { appContainer } from './container/container.js'
import { TaskService } from './services/task.service.js'
import { StorageSymbol } from './container/identifiers.js'

const taskService = appContainer.get(TaskService)
const storage = appContainer.get(StorageSymbol)
```

## Route Integration

### Accessing Services in Routes

Services are resolved directly from the global `appContainer` - import it in route files:

```typescript
// apps/backend/src/features/repositories/api/repositories.routes.ts
import type { OpenAPIHono } from '@hono/zod-openapi'
import { appContainer } from '../../../container/container.js'
import { RepositoryService } from '../../../features/repositories/services/repository.service.js'

export function registerRepositoryRoutes(app: OpenAPIHono) {
  app.openapi(getRepositoryRoute, async (c) => {
    const { repositoryId } = c.req.valid('param')

    // Resolve service from container
    const repositoryService = appContainer.get(RepositoryService)
    const repository = await repositoryService.getRepositoryById(repositoryId)

    if (!repository) {
      return c.json({ error: 'Repository not found' }, 404)
    }

    return c.json({ repository })
  })
}
```

**Key Pattern**:
- Import `appContainer` from `container/container.js`
- Import service classes you need
- Resolve services with `appContainer.get(ServiceClass)`
- No Hono context pollution
- Clear dependency tracking via imports

## Testing with InversifyJS

### Creating Test Containers

Create isolated test containers with mocked dependencies:

```typescript
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import { Container } from 'inversify'
import { StorageSymbol } from '../container/identifiers.js'
import { TaskService } from './task.service.js'
import { EventService } from './event.service.js'

describe('TaskService', () => {
  let container: Container
  let taskService: TaskService
  let mockStorage: StorageBackend
  let mockEventService: EventService

  beforeEach(async () => {
    // Create isolated test container
    container = new Container()

    // Create mocks
    mockStorage = {
      save: vi.fn(),
      get: vi.fn(),
      delete: vi.fn(),
      list: vi.fn()
    } as any

    mockEventService = {
      emit: vi.fn()
    } as any

    // Bind mocks to container
    container.bind(StorageSymbol).toConstantValue(mockStorage)
    container.bind(EventService).toConstantValue(mockEventService)

    // Bind service under test
    container.bind(TaskService).toSelf()

    // Resolve service with mocked dependencies
    taskService = container.get(TaskService)
  })

  afterEach(() => {
    container.unbindAll()
  })

  it('should create a task', async () => {
    const input = { title: 'Test task' }
    const expectedTask = { id: '123', ...input }

    mockStorage.save.mockResolvedValue(expectedTask)

    const result = await taskService.createTask(input)

    expect(result).toEqual(expectedTask)
    expect(mockStorage.save).toHaveBeenCalledWith('tasks', input)
    expect(mockEventService.emit).toHaveBeenCalledWith('task.created', { taskId: '123' })
  })
})
```

### Test Container Pattern

**Setup**:
1. Create new `Container()` instance
2. Bind mocks using `.toConstantValue()` for interfaces and dependencies
3. Bind service under test using `.toSelf()`
4. Resolve service with `container.get(ServiceClass)`

**Cleanup**:
```typescript
afterEach(() => {
  container.unbindAll()  // Clean up all bindings
})
```

## Adding a New Service

### Step-by-Step Checklist

1. **Create service class** with decorators:
```typescript
// apps/backend/src/services/my-new.service.ts
import { inject, injectable } from 'inversify'
import { StorageSymbol } from '../storage/storage-backend.js'
import type { StorageBackend } from '../storage/storage-backend.js'
import { EventService } from './event.service.js'

@injectable()
export class MyNewService {
  constructor(
    @inject(StorageSymbol) private storage: StorageBackend,
    @inject(EventService) private eventService: EventService
  ) {}
}
```

2. **Add to container bindings** in `container/container.ts`:
```typescript
export async function bindServices(container: Container): Promise<void> {
  // ... existing imports
  const { MyNewService } = await import('../services/my-new.service.js')

  // ... existing bindings
  container.bind(MyNewService).toSelf().inSingletonScope()
}
```

3. **Use in routes** by importing directly:
```typescript
// apps/backend/src/routes/my-route.ts
import { appContainer } from '../container/container.js'
import { MyNewService } from '../services/my-new.service.js'

export function registerMyRoutes(app: OpenAPIHono) {
  app.openapi(myRoute, async (c) => {
    const myService = appContainer.get(MyNewService)
    // Use the service...
  })
}
```

**That's it!** No need to update ServiceContainer interface or manually resolve in app.ts.

## Critical Rules

### ✅ DO:

- **Use `@injectable()` on ALL service classes**
- **Use `@inject()` on EVERY constructor parameter** (classes AND interfaces)
- **Import symbols from interface files** (colocated with interfaces)
- **Bind services in `container/container.ts`** with explicit `.bind()` calls
- **Use `.toSelf().inSingletonScope()`** for service bindings
- **Use `.toConstantValue()`** for infrastructure instances
- **Import `appContainer` directly** in routes and resolve services
- **Import service classes** you need in route files for type safety
- **Create isolated test containers** with mocked dependencies

### ❌ DON'T:

- **Never omit `@inject()` decorator** - InversifyJS requires it for all parameters
- **Don't forget to bind services** - decorated classes are NOT auto-registered
- **Don't call `container.get()` in service constructors** - only in routes/bootstrap/tests
- **Don't use `import type` for service classes** in constructors - breaks decorator metadata
- **Don't create new instances manually** - let the container manage lifecycle
- **Don't put container in Hono context** - use global `appContainer` instead

## Common Patterns

### Service with Interface and Class Dependencies

```typescript
@injectable()
export class WorkspaceService {
  constructor(
    @inject(StorageSymbol) private storage: StorageBackend,              // Interface - Symbol
    @inject(RepositoryService) private repositoryService: RepositoryService,  // Class
    @inject(EventService) private eventService: EventService,                 // Class
    @inject(DockerService) private dockerService: DockerService               // Class
  ) {}
}
```

### Service Using Infrastructure

```typescript
@injectable()
export class GitHubService {
  constructor(
    @inject(GitHubClientSymbol) private githubClient: GitHubClient,  // Infrastructure Symbol
    @inject(SettingsService) private settingsService: SettingsService    // Service Class
  ) {}
}
```

### Service with Map Dependency

```typescript
@injectable()
export class AuthService {
  constructor(
    @inject(OAuthProvidersSymbol) private oauthProviders: Map<string, OAuthProvider>,
    @inject(SessionService) private sessionService: SessionService
  ) {}
}
```

## File Locations Reference

- **DI Symbols**: Colocated with their interfaces (see "Available Symbols" section)
- **Container Configuration**: `apps/backend/src/container/container.ts`
- **Application Bootstrap**: `apps/backend/src/app.ts`
- **Services**: `apps/backend/src/services/*.service.ts`
- **Feature Services**: `apps/backend/src/features/*/services/*.service.ts`
- **Storage Interface**: `apps/backend/src/storage/storage-backend.ts` (includes `StorageSymbol`)
- **Client Interfaces**: `apps/backend/src/infrastructure/clients/*.client.ts` (include their symbols)

## Reference

- [InversifyJS Documentation](https://inversify.io/)
- [InversifyJS GitHub](https://github.com/inversify/InversifyJS)
- [Dependency Inversion Principle](https://inversify.io/docs/introduction/dependency-inversion/)
- [Binding in InversifyJS](https://inversify.io/docs/fundamentals/binding/)
