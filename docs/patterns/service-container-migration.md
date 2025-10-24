# Service Container to TSyringe Migration Guide

## Overview

This guide provides step-by-step instructions for migrating from the legacy service-container pattern to TSyringe dependency injection. The migration can be done incrementally, one feature at a time, without disrupting the existing system.

## Why Migrate?

### Problems with the Old Pattern

**Manual Dependency Wiring**
```typescript
// ❌ Old: Manual instantiation and ordering
container.storage = await createStorageService()
container.repository = createRepositoryService(container.storage)
container.workspace = createWorkspaceService(
  container.repository,
  container.storage,
  container.event,
  container.docker,
  container.claude,
  container.previewRegistry,
  container.github
)
```

**Factory Function Boilerplate**
```typescript
// ❌ Old: Factory functions for every service
export function createTaskService(
  storage: StorageBackend,
  event: EventService,
  workspace: WorkspaceService,
  // ... 5 more parameters
): TaskService {
  return new TaskService(storage, event, workspace, /* ... */)
}
```

**Dependency Order Issues**
- Services must be created in exact dependency order
- Easy to introduce circular dependencies
- Hard to refactor without breaking initialization
- Difficult to trace dependency chains

**Testing Complexity**
```typescript
// ❌ Old: Manual mock wiring
const mockStorage = createMockStorage()
const mockEvent = createMockEvent()
const service = createTaskService(mockStorage, mockEvent, /* ... */)
```

### Benefits of TSyringe

**Automatic Resolution**
```typescript
// ✅ New: Automatic dependency injection
@singleton()
export class WorkspaceService {
  constructor(
    private storage: Storage,
    private repository: RepositoryService,
    private event: EventService,
    private docker: DockerService,
    private claude: ClaudeService,
    private previewRegistry: PreviewServerRegistry,
    private github: GitHubService
  ) {}
}
```

**No Factory Boilerplate**
```typescript
// ✅ New: Just resolve the service
const taskService = container.resolve(TaskService)
```

**Order Independent**
- TSyringe resolves dependencies automatically
- No manual ordering required
- Circular dependencies detected at runtime

**Easy Testing**
```typescript
// ✅ New: Child containers with mocked dependencies
const testContainer = container.createChildContainer()
testContainer.register(Storage, { useValue: mockStorage })
const service = testContainer.resolve(TaskService)
```

## Migration Strategy

### Phase 1: Setup TSyringe

1. **Install Dependencies**
```bash
bun add tsyringe reflect-metadata
```

2. **Update TypeScript Configuration**
```json
// apps/backend/tsconfig.json
{
  "compilerOptions": {
    "experimentalDecorators": true,
    "emitDecoratorMetadata": true,
    // ... existing options
  }
}
```

3. **Initialize in App Entry Point**
```typescript
// apps/backend/src/app.ts
import 'reflect-metadata'
import { container } from 'tsyringe'

// Keep existing service-container code for now
```

### Phase 2: Migrate Core Infrastructure

Start with foundational services that have few dependencies.

#### Old Pattern
```typescript
// apps/backend/src/storage/index.ts
export function createStorageService(): StorageBackend {
  return new FileStorage(getDataDir())
}

// apps/backend/src/services/service-container.ts
container.storage = await createStorageService()
```

#### New Pattern
```typescript
// apps/backend/src/infrastructure/storage/file-storage.ts
import { singleton } from 'tsyringe'

@singleton()
export class FileStorage implements Storage {
  constructor() {
    this.dataDir = getDataDir()
  }
  // ... implementation
}

// apps/backend/src/app.ts
import 'reflect-metadata'
import { container } from 'tsyringe'

// Register infrastructure
container.register('Storage', { useClass: FileStorage })
```

### Phase 3: Migrate Services One by One

Migrate services incrementally, starting with those that have few dependencies.

#### Example: Migrating EventService

**Old Pattern**
```typescript
// apps/backend/src/services/event.service.ts
export class EventService {
  constructor(private storage: StorageBackend) {}

  async emit(event: Event): Promise<void> {
    await this.storage.save('events', event)
  }
}

export function createEventService(storage: StorageBackend): EventService {
  return new EventService(storage)
}

// In service-container.ts
container.event = createEventService(container.storage)
```

**New Pattern**
```typescript
// apps/backend/src/features/events/services/event.service.ts
import { singleton, inject } from 'tsyringe'
import type { Storage } from '../../../infrastructure/storage/storage.js'

@singleton()
export class EventService {
  constructor(
    @inject('Storage') private storage: Storage
  ) {}

  async emit(event: Event): Promise<void> {
    await this.storage.save('events', event)
  }
}

// apps/backend/src/app.ts
import { EventService } from './features/events/services/event.service.js'

// Just resolve when needed - no manual wiring
const eventService = container.resolve(EventService)
```

#### Example: Migrating Complex Service (TaskService)

**Old Pattern**
```typescript
// apps/backend/src/services/task.service.ts
export class TaskService {
  constructor(
    private storage: StorageBackend,
    private event: EventService,
    private workspace: WorkspaceService,
    private command: CommandService,
    private repository: RepositoryService,
    private agent: AgentService,
    private screenshot: ScreenshotService,
    private message: MessageService
  ) {}
}

export function createTaskService(
  storage: StorageBackend,
  event: EventService,
  workspace: WorkspaceService,
  command: CommandService,
  repository: RepositoryService,
  agent: AgentService,
  screenshot: ScreenshotService,
  message: MessageService
): TaskService {
  return new TaskService(
    storage,
    event,
    workspace,
    command,
    repository,
    agent,
    screenshot,
    message
  )
}

// In service-container.ts
container.task = createTaskService(
  container.storage,
  container.event,
  container.workspace,
  container.command,
  container.repository,
  container.agent,
  container.screenshot,
  container.message
)
```

**New Pattern**
```typescript
// apps/backend/src/features/tasks/services/task.service.ts
import { singleton, inject } from 'tsyringe'
import type { Storage } from '../../../infrastructure/storage/storage.js'
import type { EventService } from '../../events/services/event.service.js'
// ... other imports

@singleton()
export class TaskService {
  constructor(
    @inject('Storage') private storage: Storage,
    private event: EventService,
    private workspace: WorkspaceService,
    private command: CommandService,
    private repository: RepositoryService,
    private agent: AgentService,
    private screenshot: ScreenshotService,
    private message: MessageService
  ) {}
}

// apps/backend/src/app.ts
import { TaskService } from './features/tasks/services/task.service.js'

// TSyringe resolves all dependencies automatically
const taskService = container.resolve(TaskService)
```

### Phase 4: Update Route Registration

**Old Pattern**
```typescript
// apps/backend/src/features/tasks/index.ts
export function registerTasksFeature(
  storage: StorageBackend,
  eventService: EventService,
  // ... more dependencies
) {
  const taskService = createTaskService(storage, eventService, /* ... */)

  return {
    registerRoutes: (app: Hono) => {
      registerTaskRoutes(app, taskService)
    },
    taskService
  }
}

// apps/backend/src/app.ts
const tasksFeature = registerTasksFeature(
  container.storage,
  container.event,
  // ... more dependencies
)
tasksFeature.registerRoutes(app)
```

**New Pattern**
```typescript
// apps/backend/src/features/tasks/index.ts
export { TaskService } from './services/task.service.js'
export { registerTaskRoutes } from './api/tasks.routes.js'

// apps/backend/src/app.ts
import { container } from 'tsyringe'
import { TaskService, registerTaskRoutes } from './features/tasks/index.js'

const taskService = container.resolve(TaskService)
registerTaskRoutes(app, taskService)
```

### Phase 5: Handle Special Cases

#### Circular Dependencies

**Old Pattern** (using setter injection):
```typescript
// In service-container.ts
container.workspace = createWorkspaceService(/* ... */)
container.preview = createPreviewService(/* ... */)
container.workspace.setPreviewService(container.preview)
```

**New Pattern** (using `delay()`):
```typescript
import { singleton, inject, delay } from 'tsyringe'

@singleton()
export class WorkspaceService {
  constructor(
    @inject(delay(() => PreviewService)) private preview: PreviewService
  ) {}
}
```

#### Services Requiring Complex Initialization

**Old Pattern**:
```typescript
export function createClaudeService(settings: SettingsService): ClaudeService {
  const apiKey = settings.getApiKey()
  const config = settings.getClaudeConfig()
  return new ClaudeService(apiKey, config)
}
```

**New Pattern** (using factory registration):
```typescript
// apps/backend/src/app.ts
container.register(ClaudeService, {
  useFactory: (c) => {
    const settings = c.resolve(SettingsService)
    return new ClaudeService(
      settings.getApiKey(),
      settings.getClaudeConfig()
    )
  }
})
```

#### External Clients (GitHub, Auth0, etc.)

**Old Pattern**:
```typescript
export function createGitHubService(
  client: GitHubClient,
  storage: StorageBackend,
  authService?: AuthService
): GitHubService {
  return new GitHubService(client, storage, authService)
}
```

**New Pattern**:
```typescript
// Register client first
container.register('GitHubClient', {
  useFactory: () => createGitHubClient()
})

@singleton()
export class GitHubService {
  constructor(
    @inject('GitHubClient') private client: GitHubClient,
    @inject('Storage') private storage: Storage,
    @inject('AuthService', { optional: true }) private auth?: AuthService
  ) {}
}
```

### Phase 6: Update Tests

**Old Pattern**:
```typescript
describe('TaskService', () => {
  it('should create a task', async () => {
    const mockStorage = { save: vi.fn(), get: vi.fn() }
    const mockEvent = { emit: vi.fn() }

    const service = createTaskService(
      mockStorage as any,
      mockEvent as any,
      // ... 6 more mocked dependencies
    )

    await service.createTask(input)
    expect(mockStorage.save).toHaveBeenCalled()
  })
})
```

**New Pattern**:
```typescript
import { container, DependencyContainer } from 'tsyringe'

describe('TaskService', () => {
  let testContainer: DependencyContainer
  let taskService: TaskService

  beforeEach(() => {
    testContainer = container.createChildContainer()

    // Mock only what you need
    testContainer.register('Storage', { useValue: mockStorage })
    testContainer.register(EventService, { useValue: mockEvent })

    taskService = testContainer.resolve(TaskService)
  })

  afterEach(() => {
    testContainer.reset()
  })

  it('should create a task', async () => {
    await taskService.createTask(input)
    expect(mockStorage.save).toHaveBeenCalled()
  })
})
```

### Phase 7: Remove Legacy Service Container

Once all services are migrated:

1. **Remove service-container.ts**
```bash
git rm apps/backend/src/services/service-container.ts
```

2. **Remove factory functions**
```typescript
// Delete all createXService() functions
```

3. **Clean up app.ts**
```typescript
// Before
const container = await createServiceContainer()
app.use('*', async (c, next) => {
  c.set('services', container)
  await next()
})

// After
// Services resolved directly as needed
const taskService = container.resolve(TaskService)
registerTaskRoutes(app, taskService)
```

## Migration Checklist

### Infrastructure Layer
- [ ] Install tsyringe and reflect-metadata
- [ ] Update tsconfig.json with decorator support
- [ ] Import reflect-metadata in app.ts
- [ ] Register Storage backend
- [ ] Register external clients (GitHub, Auth0, etc.)

### Service Migration (per service)
- [ ] Add @singleton() decorator to service class
- [ ] Add @inject() decorators for interface dependencies
- [ ] Remove factory function
- [ ] Update feature index.ts exports
- [ ] Update route registration in app.ts
- [ ] Migrate service tests to use child containers

### Cleanup
- [ ] Remove service-container.ts
- [ ] Remove all createXService() factory functions
- [ ] Update documentation references
- [ ] Update CLAUDE.md patterns
- [ ] Remove legacy container injection from app middleware

## Common Migration Issues

### Issue: "Cannot resolve dependency"

**Problem**: TSyringe can't find a registered implementation.

**Solution**: Ensure the dependency is either:
1. Decorated with `@singleton()` or `@injectable()`
2. Registered programmatically: `container.register(Token, { useClass: Implementation })`

```typescript
// Make sure infrastructure is registered
container.register('Storage', { useClass: FileStorage })
```

### Issue: Circular dependency detected

**Problem**: Services depend on each other in a cycle.

**Solution**: Use `delay()` for one direction of the dependency.

```typescript
import { singleton, inject, delay } from 'tsyringe'

@singleton()
export class ServiceA {
  constructor(
    @inject(delay(() => ServiceB)) private serviceB: ServiceB
  ) {}
}
```

### Issue: Tests failing with "No registered value"

**Problem**: Tests using global container instead of child containers.

**Solution**: Create child containers for test isolation.

```typescript
const testContainer = container.createChildContainer()
testContainer.register('Storage', { useValue: mockStorage })
const service = testContainer.resolve(TaskService)
```

### Issue: Interface injection not working

**Problem**: TypeScript interfaces are erased at runtime.

**Solution**: Use string tokens or abstract classes.

```typescript
// Define token
export const STORAGE_TOKEN = 'Storage'

// Use in service
@singleton()
export class MyService {
  constructor(
    @inject(STORAGE_TOKEN) private storage: Storage
  ) {}
}

// Register implementation
container.register(STORAGE_TOKEN, { useClass: FileStorage })
```

## Incremental Migration Example

You can run both patterns simultaneously during migration:

```typescript
// apps/backend/src/app.ts
import 'reflect-metadata'
import { container } from 'tsyringe'
import { createServiceContainer } from './services/service-container.js'

// Setup TSyringe infrastructure
container.register('Storage', { useClass: FileStorage })

// Old services still using service-container
const legacyContainer = await createServiceContainer()

// New services using TSyringe
const eventService = container.resolve(EventService) // Migrated
const taskService = container.resolve(TaskService)   // Migrated

// Routes can use either
registerTaskRoutes(app, taskService) // New way
registerRepoRoutes(app, legacyContainer.repository) // Old way
```

This allows you to migrate features incrementally without a big-bang rewrite.

## Best Practices During Migration

1. **Migrate leaf services first** - Services with few dependencies
2. **Update tests immediately** - Don't leave broken tests
3. **Keep both patterns working** - Ensure the app runs after each migration
4. **Commit frequently** - One service per commit
5. **Update documentation** - Keep CLAUDE.md and pattern docs current

## Rollback Plan

If you need to rollback a service:

1. **Restore factory function**
```typescript
export function createTaskService(/* dependencies */): TaskService {
  return new TaskService(/* ... */)
}
```

2. **Remove decorator**
```typescript
// Remove @singleton()
export class TaskService { /* ... */ }
```

3. **Restore service-container.ts wiring**
```typescript
container.task = createTaskService(/* dependencies */)
```

4. **Restore route registration**
```typescript
registerTaskRoutes(app, legacyContainer.task)
```

## Next Steps

After completing the migration:

1. **Review dependency graph** - Visualize service relationships
2. **Optimize service boundaries** - Look for opportunities to simplify
3. **Add integration tests** - Ensure the wired system works end-to-end
4. **Document patterns** - Update team documentation with examples

## Related Documentation

- [Dependency Injection Pattern](./dependency-injection.md) - TSyringe usage patterns
- [Backend Feature Architecture](./backend-feature-architecture.md) - Feature structure with TSyringe
- [Feature Storage Adapters](./feature-storage-adapters.md) - Storage pattern integration

## References

- [TSyringe GitHub](https://github.com/microsoft/tsyringe)
- [TypeScript Decorators](https://www.typescriptlang.org/docs/handbook/decorators.html)
