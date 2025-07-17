# Dependency Injection Pattern for TypeScript Hono Applications

## Pattern Overview

This pattern document instructs how to implement consistent dependency injection in a TypeScript Hono application. The goal is to eliminate inconsistent service creation patterns, hidden dependencies, and singleton anti-patterns while maintaining type safety and testability.

## Current Problem Analysis

The codebase currently has multiple inconsistent patterns:
- Some services use factory functions (`createXService()`)
- Some use singleton pattern with global state (`getXService()`)
- Services are created in different ways (constructor injection vs method injection vs internal creation)
- Hidden dependencies where services internally call `getXService()` instead of receiving dependencies
- Circular dependencies resolved through setter injection

## Implementation Pattern

### 1. Service Container Structure

Create a centralized service container that manages all service instances. The container should:
- Support singleton and transient lifetimes
- Provide type-safe service resolution
- Handle circular dependencies gracefully
- Allow for scoped containers (per-request)

```typescript
// services/container.ts
export interface ServiceContainer {
  // Core services - always singleton
  settings: SettingsService
  storage: StorageBackend
  repository: RepositoryService
  claude: ClaudeService
  
  // Application services - can be singleton or scoped
  event: EventService
  command: CommandService
  workspace: WorkspaceService
  previewEnvironment: PreviewEnvironmentService
  agent: AgentService
  task: TaskService
  message: MessageService
}

export interface ServiceFactory<T> {
  (container: ServiceContainer): T | Promise<T>
}

export interface ServiceRegistration<T> {
  factory: ServiceFactory<T>
  lifetime: 'singleton' | 'transient' | 'scoped'
  instance?: T
}
```

### 2. Service Creation Pattern

All services MUST follow this pattern:

```typescript
// Example: services/claude.service.ts
export interface ClaudeServiceDependencies {
  settings: SettingsService
}

export class ClaudeService {
  constructor(private dependencies: ClaudeServiceDependencies) {
    // NO singleton pattern, NO global state
    // All dependencies passed through constructor
  }
  
  // Service methods...
}

// Factory function that extracts dependencies from container
export function createClaudeService(container: ServiceContainer): ClaudeService {
  return new ClaudeService({
    settings: container.settings
  })
}
```

### 3. Removing Hidden Dependencies

Services MUST NOT:
- Call `getXService()` internally
- Access global singletons
- Create other services internally

Instead, they MUST:
- Receive all dependencies through constructor
- Use dependency interfaces, not concrete types
- Declare dependencies explicitly

```typescript
// BAD - Hidden dependency
export class MessageService {
  async readMessages() {
    const claudeService = getClaudeService() // Hidden dependency!
    return claudeService.readClaudeMessages()
  }
}

// GOOD - Explicit dependency
export interface MessageServiceDependencies {
  claude: ClaudeService
  repository?: RepositoryService
}

export class MessageService {
  constructor(private dependencies: MessageServiceDependencies) {}
  
  async readMessages() {
    return this.dependencies.claude.readClaudeMessages()
  }
}
```

### 4. Container Setup Pattern

The container setup should:
- Register all services with appropriate lifetimes
- Build services in dependency order
- Handle circular dependencies

```typescript
// services/setup.ts
export async function createServiceContainer(): Promise<ServiceContainer> {
  const container: Partial<ServiceContainer> = {}
  
  // Phase 1: Core services (no dependencies)
  container.settings = await createSettingsService()
  container.storage = await createStorageService()
  
  // Phase 2: Services with core dependencies
  container.claude = createClaudeService(container as ServiceContainer)
  container.repository = createRepositoryService({
    storage: container.storage
  })
  
  // Phase 3: Services with multiple dependencies
  container.event = createEventService({
    storage: container.storage,
    claude: container.claude,
    repository: container.repository
  })
  
  // Phase 4: Handle circular dependencies
  container.workspace = createWorkspaceService({
    repository: container.repository,
    event: container.event,
    storage: container.storage
  })
  
  container.previewEnvironment = createPreviewEnvironmentService({
    storage: container.storage,
    workspace: container.workspace
  })
  
  // Resolve circular dependency
  container.workspace.setPreviewEnvironmentService(container.previewEnvironment)
  
  return container as ServiceContainer
}
```

### 5. Route Handler Pattern

Routes should receive services through Hono context:

```typescript
// app.ts
export async function createApp() {
  const app = new Hono<AppBindings>()
  
  // Create container once at startup
  const container = await createServiceContainer()
  
  // Inject container into all requests
  app.use('*', async (c, next) => {
    c.set('container', container)
    await next()
  })
  
  return { app, container }
}

// routes/tasks.ts
app.post('/tasks', async (c) => {
  const container = c.get('container')
  const { task, repository } = container
  
  // Use services from container
  const result = await task.createTask(...)
  return c.json(result)
})
```

### 6. Request-Scoped Services

For services that need request-specific context:

```typescript
// Create scoped container for request
app.use('*', async (c, next) => {
  const baseContainer = c.get('container')
  
  // Create request-scoped services
  const scopedContainer = {
    ...baseContainer,
    message: createMessageService({
      claude: baseContainer.claude,
      repository: getRepositoryFromRequest(c) // Request-specific
    })
  }
  
  c.set('container', scopedContainer)
  await next()
})
```

### 7. Testing Pattern

With dependency injection, testing becomes straightforward:

```typescript
// tests/services/message.service.test.ts
describe('MessageService', () => {
  it('should read messages', async () => {
    // Create mock dependencies
    const mockClaude = {
      readClaudeMessages: vi.fn().mockResolvedValue([])
    }
    
    // Inject mocks
    const service = new MessageService({
      claude: mockClaude as any
    })
    
    // Test behavior
    await service.readMessages()
    expect(mockClaude.readClaudeMessages).toHaveBeenCalled()
  })
})
```

## Implementation Checklist

When implementing dependency injection:

1. **Remove all singleton patterns**
   - No `let instance: Service | null`
   - No `getXService()` functions
   - No module-level state

2. **Create dependency interfaces**
   - Define `XServiceDependencies` interface for each service
   - List all required dependencies

3. **Update constructors**
   - Accept dependencies interface
   - Store as private readonly property

4. **Create factory functions**
   - Named `createXService(container)`
   - Extract dependencies from container
   - Return new instance

5. **Update container setup**
   - Register services in dependency order
   - Handle circular dependencies explicitly
   - Set appropriate lifetimes

6. **Update routes**
   - Get container from context
   - Extract needed services
   - Pass configuration through container

## Anti-Patterns to Avoid

1. **Hidden Dependencies**: Never call `getXService()` inside a service
2. **Global State**: No module-level service instances
3. **Service Locator**: Don't pass the entire container to services
4. **Property Injection**: Use constructor injection only
5. **Optional Dependencies**: Make dependencies explicit, use `undefined` if truly optional

## Benefits

- **Testability**: Easy to inject mocks
- **Clarity**: All dependencies visible in constructor
- **Type Safety**: TypeScript ensures correct dependencies
- **Flexibility**: Easy to swap implementations
- **Lifecycle Management**: Clear service lifetimes
- **No Hidden Coupling**: All dependencies explicit

## Migration Strategy

1. Start with leaf services (no dependents)
2. Work up the dependency tree
3. Update one service at a time
4. Keep factory functions for backward compatibility
5. Update all callers to use container
6. Remove old singleton patterns

## Example Migration

```typescript
// BEFORE: Singleton pattern
let claudeServiceInstance: ClaudeService | null = null

export function getClaudeService(): ClaudeService {
  if (!claudeServiceInstance) {
    claudeServiceInstance = new ClaudeService()
  }
  return claudeServiceInstance
}

// AFTER: Dependency injection
export interface ClaudeServiceDependencies {
  settings: SettingsService
}

export class ClaudeService {
  constructor(private dependencies: ClaudeServiceDependencies) {}
}

export function createClaudeService(container: ServiceContainer): ClaudeService {
  return new ClaudeService({
    settings: container.settings
  })
}
```