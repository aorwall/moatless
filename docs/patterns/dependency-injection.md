# Dependency Injection with TSyringe

## Overview

This project uses [TSyringe](https://github.com/microsoft/tsyringe), a lightweight dependency injection container for TypeScript, to manage service dependencies. TSyringe uses decorators to automatically resolve and inject dependencies at runtime, eliminating boilerplate and improving testability.

## Key Benefits

- **Automatic dependency resolution**: Declare dependencies in constructors; TSyringe handles instantiation
- **Type-safe**: Full TypeScript support with compile-time type checking
- **Testable**: Easy to create isolated test containers with mocked dependencies
- **No singletons**: Services are managed by the container, not through global state
- **Clear dependencies**: All dependencies explicitly declared in constructor

## Setup

### Prerequisites

TSyringe requires specific TypeScript compiler options:

```json
// tsconfig.json
{
  "compilerOptions": {
    "experimentalDecorators": true,
    "emitDecoratorMetadata": true
  }
}
```

### Installation

```bash
bun add tsyringe reflect-metadata
```

### Bootstrap

Import `reflect-metadata` at your application entry point (before any decorated classes):

```typescript
// apps/backend/src/app.ts
import 'reflect-metadata'
import { container } from 'tsyringe'
// ... rest of app
```

## Core Decorators

### @singleton()

Registers a class as a singleton—the same instance is returned for all resolutions throughout the application lifecycle.

**Use for**: Stateless services, shared resources, application-wide state managers.

```typescript
import { singleton } from 'tsyringe'

@singleton()
export class AuthService {
  constructor(
    private storage: Storage,
    private githubClient: GitHubClient
  ) {}

  async login(credentials: Credentials): Promise<Session> {
    // Implementation
  }
}
```

### @injectable()

Registers a class for dependency injection. Creates a new instance on each resolution (transient lifecycle).

**Use for**: Stateful services, request-scoped operations, or when you need fresh instances.

```typescript
import { injectable } from 'tsyringe'

@injectable()
export class RequestHandler {
  constructor(private logger: Logger) {}
}
```

**Note**: In this project, we primarily use `@singleton()` for services since most are stateless and benefit from being reused.

### @inject()

Specifies the token to use for injecting a dependency. Required when injecting interfaces or when the parameter type can't be inferred.

```typescript
import { singleton, inject } from 'tsyringe'

@singleton()
export class TaskService {
  constructor(
    @inject('Storage') private storage: Storage,
    @inject('Logger') private logger: Logger
  ) {}
}
```

**When to use**:
- Injecting interface types (TypeScript interfaces are erased at runtime)
- Using injection tokens instead of class references
- Resolving ambiguous dependencies

## Service Implementation Pattern

### Basic Service

```typescript
// apps/backend/src/features/tasks/services/task.service.ts
import { singleton } from 'tsyringe'
import type { Storage } from '../../../infrastructure/storage/storage.js'
import type { EventService } from '../../events/services/event.service.js'

@singleton()
export class TaskService {
  constructor(
    private storage: Storage,
    private eventService: EventService,
    private workspaceService: WorkspaceService
  ) {}

  async createTask(input: CreateTaskInput): Promise<Task> {
    const task = await this.storage.save('tasks', input)
    await this.eventService.emit('task.created', { taskId: task.id })
    return task
  }

  async getTask(id: string): Promise<Task | null> {
    return this.storage.get('tasks', id)
  }
}
```

### Service with Interface Dependencies

When depending on interfaces (not concrete classes), use `@inject()` with a token:

```typescript
import { singleton, inject } from 'tsyringe'

// Define token for the interface
export const STORAGE_TOKEN = 'Storage'

@singleton()
export class RepositoryService {
  constructor(
    @inject(STORAGE_TOKEN) private storage: Storage
  ) {}
}

// Register the implementation in app.ts
container.register(STORAGE_TOKEN, { useClass: FileStorage })
```

## Application Bootstrap

### Registering Infrastructure

For infrastructure components (storage, clients, etc.), use programmatic registration:

```typescript
// apps/backend/src/app.ts
import 'reflect-metadata'
import { container } from 'tsyringe'
import { FileStorage } from './infrastructure/storage/file-storage.js'
import { Storage } from './infrastructure/storage/storage.js'

// Register storage implementation
container.register(Storage, { useClass: FileStorage })

// Or register with token
container.register('Storage', { useClass: FileStorage })
```

### Resolving Services

Services decorated with `@singleton()` are automatically registered. Simply resolve them:

```typescript
// apps/backend/src/app.ts
import { container } from 'tsyringe'
import { TaskService, registerTaskRoutes } from './features/tasks/index.js'
import { AuthService, registerAuthRoutes } from './features/auth/index.js'

// Resolve services from container
const taskService = container.resolve(TaskService)
const authService = container.resolve(AuthService)

// Register routes with resolved services
registerTaskRoutes(app, taskService)
registerAuthRoutes(app, authService)
```

## Route Registration

Routes receive resolved service instances as parameters:

```typescript
// apps/backend/src/features/tasks/api/tasks.routes.ts
import type { OpenAPIHono } from '@hono/zod-openapi'
import { createTaskRoute, getTaskRoute } from '@moatless/api-contract/tasks'
import type { TaskService } from '../services/task.service.js'

export function registerTaskRoutes(
  app: OpenAPIHono<AppBindings>,
  taskService: TaskService
) {
  app.openapi(createTaskRoute, async (c) => {
    const body = c.req.valid('json')
    const task = await taskService.createTask(body)
    return c.json(task, 201)
  })

  app.openapi(getTaskRoute, async (c) => {
    const { id } = c.req.valid('param')
    const task = await taskService.getTask(id)
    if (!task) {
      return c.json({ error: 'Task not found' }, 404)
    }
    return c.json(task)
  })
}
```

## Testing with TSyringe

### Child Containers for Test Isolation

Create child containers to isolate tests with mocked dependencies:

```typescript
// apps/backend/src/features/tasks/services/__tests__/task.service.test.ts
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { container } from 'tsyringe'
import { TaskService } from '../task.service.js'
import type { Storage } from '../../../../infrastructure/storage/storage.js'

describe('TaskService', () => {
  let testContainer: DependencyContainer
  let taskService: TaskService
  let mockStorage: Storage

  beforeEach(() => {
    // Create isolated test container
    testContainer = container.createChildContainer()

    // Create mocks
    mockStorage = {
      save: vi.fn(),
      get: vi.fn(),
      delete: vi.fn()
    }

    // Register mocks in test container
    testContainer.register('Storage', { useValue: mockStorage })

    // Resolve service with mocked dependencies
    taskService = testContainer.resolve(TaskService)
  })

  afterEach(() => {
    // Clean up child container
    testContainer.reset()
  })

  it('should create a task', async () => {
    const input = { title: 'Test task', description: 'Description' }
    const expectedTask = { id: '123', ...input }

    mockStorage.save.mockResolvedValue(expectedTask)

    const result = await taskService.createTask(input)

    expect(result).toEqual(expectedTask)
    expect(mockStorage.save).toHaveBeenCalledWith('tasks', input)
  })
})
```

### Testing Route Handlers

Test routes by passing mock service instances:

```typescript
// apps/backend/src/features/tasks/api/__tests__/tasks.routes.test.ts
import { describe, expect, it, vi } from 'vitest'
import { Hono } from 'hono'
import { registerTaskRoutes } from '../tasks.routes.js'
import type { TaskService } from '../../services/task.service.js'

describe('Task Routes', () => {
  it('should create a task', async () => {
    const app = new Hono()
    const mockTaskService: TaskService = {
      createTask: vi.fn().mockResolvedValue({ id: '123', title: 'Test' })
    } as any

    registerTaskRoutes(app, mockTaskService)

    const res = await app.request('/tasks', {
      method: 'POST',
      body: JSON.stringify({ title: 'Test', description: 'Desc' }),
      headers: { 'Content-Type': 'application/json' }
    })

    expect(res.status).toBe(201)
    expect(await res.json()).toEqual({ id: '123', title: 'Test' })
  })
})
```

## Advanced Patterns

### Circular Dependencies

TSyringe handles circular dependencies automatically in most cases. If needed, use `delay()` for explicit resolution:

```typescript
import { singleton, inject, delay } from 'tsyringe'

@singleton()
export class ServiceA {
  constructor(
    @inject(delay(() => ServiceB)) private serviceB: ServiceB
  ) {}
}

@singleton()
export class ServiceB {
  constructor(private serviceA: ServiceA) {}
}
```

### Lifecycle Scopes

TSyringe supports different lifecycle scopes:

```typescript
import { scoped, Lifecycle } from 'tsyringe'

// Container-scoped: new instance per child container
@scoped(Lifecycle.ContainerScoped)
export class RequestScopedService {}

// Resolution-scoped: same instance within resolution chain
@scoped(Lifecycle.ResolutionScoped)
export class TransientService {}
```

**Note**: We primarily use `@singleton()` in this project. Use other scopes only when you have specific requirements.

### Factory Registration

Register services using factory functions when complex initialization is needed:

```typescript
container.register('ClaudeClient', {
  useFactory: (c) => {
    const settings = c.resolve(SettingsService)
    return new ClaudeClient({
      apiKey: settings.getApiKey(),
      model: settings.getModel()
    })
  }
})
```

## Best Practices

### DO:

- **Use `@singleton()` for stateless services** - Most application services should be singletons
- **Declare all dependencies in constructor** - Make dependencies explicit and testable
- **Use interfaces with `@inject()` tokens** - For maximum flexibility and testability
- **Create child containers for tests** - Isolate test dependencies
- **Register infrastructure programmatically** - Storage, clients, and external dependencies
- **Keep services focused** - Single responsibility principle

### DON'T:

- **Never use `getInstance()` or global singletons** - Let the container manage instances
- **Don't call `container.resolve()` in services** - Only in app bootstrap or tests
- **Don't use circular dependencies** - Indicates poor service boundaries
- **Don't mix singleton patterns with DI** - Choose one approach
- **Don't resolve services in constructors** - Only declare dependencies as parameters

## Common Patterns

### Infrastructure Registration

```typescript
// apps/backend/src/app.ts
import 'reflect-metadata'
import { container } from 'tsyringe'

// Register storage backend
container.register('Storage', { useClass: FileStorage })

// Register external clients
container.register('GitHubClient', {
  useFactory: () => new GitHubClient(process.env.GITHUB_TOKEN)
})

// Register configuration
container.register('Config', {
  useValue: {
    port: process.env.PORT || 8000,
    nodeEnv: process.env.NODE_ENV || 'development'
  }
})
```

### Service with Multiple Dependencies

```typescript
@singleton()
export class WorkspaceService {
  constructor(
    private storage: Storage,
    private repositoryService: RepositoryService,
    private eventService: EventService,
    private dockerService: DockerService,
    private claudeService: ClaudeService
  ) {}
}
```

### Feature Entry Point

```typescript
// apps/backend/src/features/auth/index.ts

// Export service classes (decorated with @singleton)
export { AuthService } from './services/auth.service.js'
export { AuthStorage } from './storage/auth.storage.js'

// Export route registration function
export { registerAuthRoutes } from './api/auth.routes.js'

// Export types
export type { AuthConfig, Session, Credentials } from './types/index.js'
```

## Migration from Service Container

If you're migrating from the legacy service-container pattern, see [Service Container Migration Guide](./service-container-migration.md).

## Reference

- [TSyringe GitHub Repository](https://github.com/microsoft/tsyringe)
- [TSyringe Documentation](https://github.com/microsoft/tsyringe#readme)
- [TypeScript Decorators](https://www.typescriptlang.org/docs/handbook/decorators.html)

## Related Documentation

- [Backend Feature Architecture](./backend-feature-architecture.md)
- [Service Container Migration Guide](./service-container-migration.md)
- [Feature Storage Adapters](./feature-storage-adapters.md)
