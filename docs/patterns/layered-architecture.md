# Layered Architecture Pattern

## Prerequisites

Before reading this pattern:
1. Review [API Best Practices](api-best-practices.md) for route handler implementation details
2. Understand [Dependency Injection Pattern](dependency-injection.md) for service container setup
3. See [Error Handling Pattern](error-handling.md) for layer-appropriate error handling

## Problem

Without clear layer boundaries:
- Business logic mixed with HTTP handling in routes
- Database queries scattered across the codebase
- Hard to test components in isolation
- AI agents struggle with "Where should I put this code?"

## Solution

**Strict layer separation with explicit responsibilities.**

Three layers with clear boundaries:
1. **Route Layer** - HTTP handling only
2. **Service Layer** - Business logic only
3. **Data/External Layer** - Storage and external systems

## TypeScript Module Rules

- MUST use named exports: `export class TaskService {}` NEVER `export default`
- MUST use relative imports: `import { foo } from '../services/task.service'` NEVER absolute paths
- MUST place decorators immediately above the symbol they decorate, with JSDoc above the decorator
- MUST avoid mutable exports: use getters instead of `export let`
- MUST NOT use namespaces or `import type`; use standard ECMAScript modules

See implementation examples at `apps/backend/src/routes/workspace.routes.ts` and `apps/backend/src/services/task.service.ts`

## Layer Responsibilities

### Route Layer (`apps/backend/src/routes/*.ts`)

**Responsibilities:**
- HTTP request/response handling
- Request validation (via OpenAPI/Zod)
- Authentication/authorization checks
- Error handling and HTTP status codes
- Calling services and returning results

**NEVER:**
- Business logic
- Database queries
- External API calls
- Complex calculations
- File system operations

### Service Layer (`apps/backend/src/services/*.service.ts`)

**Responsibilities:**
- ALL business logic
- Orchestrating multiple operations
- Data validation beyond HTTP schema
- Calling other services
- Interacting with storage/external systems
- State management
- Event emission

**NEVER:**
- HTTP-specific code (status codes, headers)
- Direct access to request/response objects
- Import from routes

### Data/External Layer

This layer handles all external interactions including database access, file operations, and third-party services.

#### Repository Pattern (Database Access)

**Location:** `apps/backend/src/features/*/repository/*.repository.ts`

**Responsibilities:**
- Type-safe database operations using Drizzle ORM
- CRUD operations for domain entities
- Query building and execution
- Transaction management

**See:** [Repository Pattern](repository-pattern.md) for complete implementation guide

#### Storage Service (File Operations)

**Location:** `apps/backend/src/services/storage.ts`

**Responsibilities:**
- JSON file-based persistence
- File system operations
- Directory management
- Data serialization/deserialization

#### External Services

**Location:** `apps/backend/src/services/<service>.service.ts` (e.g., `github.service.ts`)

**Responsibilities:**
- External API calls
- Third-party service integration
- API response transformation

**NEVER in Data/External Layer:**
- Business rules or validation
- HTTP handling
- Event emission (return data, let caller emit)

### Utility Layer (`apps/backend/src/utils/*.ts`)

**Responsibilities:**
- Pure functions (no side effects)
- Reusable helpers
- Data transformations
- Format conversions

**NEVER:**
- State management
- Database access
- HTTP handling
- Business logic

## Decision Tree: Where Does This Code Belong?

```
START: I need to add code that...

├─ Handles HTTP requests/responses?
│  └─ YES → Route Layer
│      Files: apps/backend/src/routes/*.ts
│
├─ Contains business logic or orchestration?
│  └─ YES → Service Layer
│      Files: apps/backend/src/services/*.service.ts
│
├─ Accesses database or external systems?
│  └─ YES → Data/External Layer
│      Files: apps/backend/src/services/storage.ts
│             apps/backend/src/services/github.service.ts
│
└─ Is a reusable pure function?
   └─ YES → Utility Layer
       Files: apps/backend/src/utils/*.ts
```

## Dependency Rules

### RULE: Dependencies flow in one direction only

```
Routes → Services → Data/External
         ↓
       Utils (can be used by any layer)
```

#### Dependency Examples

```typescript
// ✅ CORRECT - Dependencies flow down layers
// Route → Service
import { TaskService } from '../services/task.service'

// Service → Storage
import { StorageBackend } from './storage'

// Service → Service (same layer)
import { EventService } from './event.service'

// Any layer → Utils
import { getCurrentCommitHash } from '../utils/git-utils'
import { handleServiceError } from '../utils/error-handler'

// ❌ WRONG - Dependencies flow up (creates circular deps)
import { registerTaskRoutes } from '../routes/tasks' // ❌ Service importing Route
import { TaskService } from './task.service' // ❌ Storage importing Service
import { StorageBackend } from '../services/storage' // ❌ Utils importing Service
```

## Rules for AI Agents

### RULE 1: Routes = HTTP Only
```typescript
// ✅ CORRECT
app.openapi(route, async (c) => {
  const params = c.req.valid('json')
  const result = await c.get('services').task.doSomething(params)
  return c.json(result)
})

// ❌ WRONG
app.openapi(route, async (c) => {
  const taskId = randomUUID()  // ❌ Business logic in route
  await storage.save(...)       // ❌ Direct storage access
})
```

### RULE 2: Services = Business Logic Only
```typescript
// ✅ CORRECT
export class TaskService {
  async createTask(params: CreateTaskRequest): Promise<Task> {
    // Validation, orchestration, business rules
  }
}

// ❌ WRONG
export class TaskService {
  async createTask(c: Context) {  // ❌ HTTP context in service
    return c.json(...)            // ❌ HTTP response in service
  }
}
```

### RULE 3: Follow Dependency Direction
```typescript
// ✅ CORRECT - Dependencies flow down
Route → Service → Storage

// ❌ WRONG - Dependencies flow up
Service → Route  // ❌ NEVER
Storage → Service  // ❌ NEVER
```

### RULE 4: Use Dependency Injection
```typescript
// ✅ CORRECT - Dependencies injected
export class TaskService {
  constructor(
    private storage: StorageBackend,
    private eventService: EventService,
  ) {}
}

// ❌ WRONG - Import singletons
import { storage } from './storage'  // ❌ NO
```

## Examples by Layer

### Route Layer Example
```typescript
// ✅ CORRECT Route Implementation
app.openapi(createTaskRoute, async (c) => {
  try {
    const services = c.get('services')
    const body = c.req.valid('json')
    const task = await services.task.createTask(body)
    return c.json({ taskId: task.taskId })
  } catch (error) {
    handleServiceError(error, 'Create task')
  }
})
```

### Service Layer Example
```typescript
// ✅ CORRECT Service Implementation
export class TaskService {
  constructor(
    private storage: StorageBackend,
    private eventService: EventService,
  ) {}

  async createTask(params: CreateTaskRequest): Promise<Task> {
    const task = { taskId: randomUUID(), ...params }

    await this.storage.saveTask(task)
    await this.eventService.emitEvent({ taskId: task.taskId })

    return task
  }
}
```

### Storage Layer Example
```typescript
// ✅ CORRECT Storage Implementation
export class StorageBackend {
  async getTask(taskId: string): Promise<Task | null> {
    const filePath = path.join(this.basePath, 'tasks', `${taskId}.json`)
    return fs.pathExists(filePath) ? await fs.readJson(filePath) : null
  }
}
```

### Utility Layer Example
```typescript
// ✅ CORRECT Utility Implementation
export async function getCurrentCommitHash(workspacePath: string): Promise<string | null> {
  try {
    const result = execSync('git rev-parse HEAD', { cwd: workspacePath })
    return result.trim()
  } catch {
    return null
  }
}
```

## Quick Reference

| Code Type | Layer | Location |
|-----------|-------|----------|
| HTTP handling | Route | `routes/*.ts` |
| Business logic | Service | `services/*.service.ts` |
| Database access | Storage | `services/storage.ts` |
| External API calls | Service | `services/<name>.service.ts` |
| Pure functions | Utility | `utils/*.ts` |
| Validation logic | Service | `services/*.service.ts` |
| Error handling | Route | catch → Service throw |

## Testing Benefits

### Service Tests (Easy - Mock dependencies)
```typescript
const mockStorage = { saveTask: vi.fn() }
const service = new TaskService(mockStorage, ...)
```

### Route Tests (Easy - Mock services)
```typescript
const mockTaskService = { createTask: vi.fn() }
const res = await app.request('/api/v1/tasks', { ... })
```

## Implementation Checklist

Before implementing new features:
1. **Identify layer** using decision tree (lines 95-106)
2. **Verify dependency direction** - dependencies MUST flow down (Routes → Services → Data)
3. **Use dependency injection** - NEVER import singletons
4. **Validate layer boundaries**:
   - Routes: Only HTTP handling, validation, and service calls
   - Services: Only business logic and orchestration
   - Storage/External: Only data access and external API calls
5. **Run validation**:
   - `bun run type:check` to verify import structure
   - `bun run biome:check` for code formatting
   - `bun run test:unit` to ensure layer isolation

See [API Best Practices](api-best-practices.md) for route implementation patterns and [Dependency Injection Pattern](dependency-injection.md) for service setup.
