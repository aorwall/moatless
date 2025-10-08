# Layered Architecture Pattern

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

### Data/External Layer (`apps/backend/src/services/storage.ts`, etc.)

**Responsibilities:**
- Database/file system operations
- External API calls
- Third-party service integration
- Data serialization/deserialization

**NEVER:**
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

#### ✅ ALLOWED Dependencies

```typescript
// Route imports Service ✅
import { TaskService } from '../services/task.service.js'

// Service imports Storage ✅
import { StorageBackend } from './storage.js'

// Service imports other Service ✅
import { EventService } from './event.service.js'

// Service imports Utils ✅
import { getCurrentCommitHash } from '../utils/git-utils.js'

// Route imports Utils ✅
import { handleServiceError } from '../utils/error-handler.js'
```

#### ❌ FORBIDDEN Dependencies

```typescript
// Service imports Route ❌
import { registerTaskRoutes } from '../routes/tasks.js' // ❌ NO

// Storage imports Service ❌
import { TaskService } from './task.service.js' // ❌ NO

// Utils import Services ❌
import { StorageBackend } from '../services/storage.js' // ❌ NO
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
import { storage } from './storage.js'  // ❌ NO
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
