# Dependency Injection Pattern

## Overview

This project uses a centralized service container pattern for dependency injection. All services are created through factory functions and wired together in a single service container, eliminating hidden dependencies and improving testability.

## Key Concepts

### Service Container
The application uses a single `ServiceContainer` interface that holds all service instances. Services are created once at application startup and injected into the Hono context for use in routes.

### No Singletons
Services MUST NOT use singleton patterns. All state is managed through the service container, and dependencies are explicitly passed through constructors.

### Factory Functions
Every service has a corresponding factory function that creates instances with proper dependencies injected.

## Implementation Reference

### 1. Service Container Location
The service container is defined in:
```
/apps/backend/services/service-container.ts
```

This file contains:
- `ServiceContainer` interface with all application services
- `createServiceContainer()` function that wires all dependencies
- Service creation in proper dependency order

### 2. Service Pattern

All services follow this pattern:

```typescript
// service.ts
export class MyService {
  constructor(
    private dependency1: Dependency1Service,
    private dependency2: Dependency2Service
  ) {}
  
  // Service methods...
}

// Factory function
export function createMyService(
  dependency1: Dependency1Service,
  dependency2: Dependency2Service
): MyService {
  return new MyService(dependency1, dependency2)
}
```

### 3. Application Bootstrap

In `/apps/backend/app.ts`:
- Service container is created once at startup
- Container is injected into Hono context
- Routes access services through `c.get('services')`

### 4. Route Usage

Routes access services from the injected container:

```typescript
app.get('/example', async (c) => {
  const services = c.get('services')
  const { task, repository } = services
  
  // Use services...
})
```

## Working with Services

### Adding a New Service

1. **Define the service class** with constructor injection:
   ```typescript
   export class NewService {
     constructor(
       private storage: StorageBackend,
       private settings: SettingsService
     ) {}
   }
   ```

2. **Create a factory function**:
   ```typescript
   export function createNewService(
     storage: StorageBackend,
     settings: SettingsService
   ): NewService {
     return new NewService(storage, settings)
   }
   ```

3. **Add to ServiceContainer interface** in `service-container.ts`:
   ```typescript
   export interface ServiceContainer {
     // ... existing services
     newService: NewService
   }
   ```

4. **Wire in createServiceContainer**:
   ```typescript
   container.newService = createNewService(
     container.storage,
     container.settings
   )
   ```

### Service Creation Order

Services must be created in dependency order. Here's the current order in `createServiceContainer`:

```typescript
// 1. Core services with no dependencies
container.storage = await createStorageService()
container.github = getGitHubService()
container.settings = await createSettingsService(container.github)

// 2. Services with single dependencies  
container.repository = createRepositoryService(container.storage)
container.claude = createClaudeService(container.settings)
container.command = createCommandService(container.storage)

// 3. Services with multiple dependencies
container.event = createEventService(container.storage)
container.workspace = createWorkspaceService(/* ... dependencies */, previewLifecycleSink)

// 4. Agent service (depends on workspace, repository, storage)
container.agent = createAgentService(
  container.workspace,
  container.repository,
  container.storage
)

// 5. Task service (depends on agent + other services)
container.task = createTaskService(
  container.storage,
  container.event,
  container.workspace,
  container.command,
  container.repository,
  container.agent  // AgentService dependency
)
```

### Modifying Existing Services

When adding dependencies to an existing service:

1. **Update the constructor** to accept the new dependency
2. **Update the factory function** parameters
3. **Update the wiring** in `createServiceContainer`
4. **Update any tests** to provide the new dependency

### Handling Circular Dependencies

If services have circular dependencies:

1. Use setter injection for one direction:
   ```typescript
   // In createServiceContainer
   container.workspace = createWorkspaceService(container.repository, container.storage, previewLifecycleSink)
   container.previewEnvironment = createPreviewEnvironmentService(container.storage, container.workspace)
   
   // Resolve circular dependency
   container.workspace.setPreviewEnvironmentService(container.previewEnvironment)
   ```

2. Document why the circular dependency exists

## Testing

Services are easily testable with dependency injection:

```typescript
const mockStorage = { /* mock methods */ }
const mockSettings = { /* mock methods */ }

const service = new MyService(mockStorage as any, mockSettings as any)
// Test service methods...
```

## Important Rules

### DO:
- Accept all dependencies through constructor
- Use factory functions for service creation
- Access services through the injected container in routes
- Keep service creation order in `createServiceContainer` based on dependencies

### DON'T:
- Use singleton patterns (`getInstance()`, module-level instances)
- Call `getXService()` functions inside services
- Create services inside other services
- Access services before they're initialized in the container
- **Cast services to `any` to access properties** - All needed data should be passed properly
- **Access private or non-existent properties** - Services should only use their public interface

## Common Services

The following core services are available in the container:

- `storage`: File and data persistence
- `settings`: Application settings management
- `claude`: Claude AI integration
- `repository`: Repository management
- `task`: Task management and orchestration
- `agent`: AI agent execution and lifecycle management
- `event`: Event handling
- `message`: Message processing
- `workspace`: Workspace management
- `command`: Command execution
- `previewEnvironment`: Preview environment management

## Common Anti-Patterns to Avoid

### Type Casting to Access Properties
```typescript
// ❌ BAD - Don't cast to any
const settings = (service as any).privateSettings
if ((response as any).someProperty) { }

// ✅ GOOD - Use proper interfaces
const settings = service.getSettings()
if (response.someProperty) { }
```

### Accessing Non-Existent Properties
```typescript
// ❌ BAD - Property doesn't exist
const claudeSettings = (eventService as any).claudeCodeSettings

// ✅ GOOD - Pass data through proper channels
const claudeSettings = settingsService.getClaudeSettings()
```

### Unnecessary Type Casts in Responses
```typescript
// ❌ BAD - Unnecessary cast
return c.json(workspaceStatus as any)

// ✅ GOOD - Types are already correct
return c.json(workspaceStatus)
```

## Migration Notes

When encountering old singleton patterns like `getSettingsService()`:
1. Check if the code is part of initialization (may need special handling)
2. If in a route, use the injected container instead
3. If in a service, add as a constructor dependency
4. For utilities/helpers, consider if they should be services or accept parameters

## File Locations

- Service implementations: `/apps/backend/services/*.service.ts`
- Service container: `/apps/backend/services/service-container.ts`
- Application bootstrap: `/apps/backend/app.ts`
- Route handlers: `/apps/backend/routes/*.ts`
