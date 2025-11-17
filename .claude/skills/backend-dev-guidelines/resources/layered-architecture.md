# Layered Architecture

## Overview
The backend follows a strict three-layer architecture with one-way dependencies.

## Architecture Layers

```
┌─────────────────────────────────┐
│   Routes (HTTP Layer)           │  ← HTTP requests/responses only
│   - Request validation          │
│   - Response formatting          │
│   - Error handling               │
└────────────┬────────────────────┘
             │ calls
             ↓
┌─────────────────────────────────┐
│   Services (Business Logic)     │  ← Business rules & orchestration
│   - Business logic              │
│   - Transaction management       │
│   - Authorization               │
└────────────┬────────────────────┘
             │ calls
             ↓
┌─────────────────────────────────┐
│   Repositories (Data Access)    │  ← Database operations only
│   - CRUD operations             │
│   - Query building              │
│   - Data mapping                │
└─────────────────────────────────┘
```

## Layer Responsibilities

### Routes Layer
**Purpose**: Handle HTTP concerns only

**Responsibilities**:
- Receive HTTP requests
- Validate input (automatic via Zod)
- Call service methods
- Format responses
- Handle HTTP errors

**DON'T**:
- Contain business logic
- Access repositories directly
- Perform database operations
- Make external API calls

**Example**:
```tsx
// apps/backend/src/routes/tasks.ts
app.openapi(routes.tasks.create, async (c) => {
  const taskService = appContainer.get<TaskService>(TYPES.TaskService)
  const body = c.req.valid('json')

  const task = await taskService.createTask(body)
  return c.json(task, 201)
})
```

### Services Layer
**Purpose**: Implement business logic

**Responsibilities**:
- Business rule enforcement
- Transaction management
- Orchestrate multiple repositories
- Authorization checks
- External API calls

**DON'T**:
- Know about HTTP (req/res objects)
- Access database directly
- Return HTTP status codes

**Example**:
```tsx
// apps/backend/src/services/task.service.ts
@injectable()
export class TaskService {
  constructor(
    @inject(TYPES.TaskRepository) private taskRepo: ITaskRepository,
    @inject(TYPES.UserRepository) private userRepo: IUserRepository
  ) {}

  async createTask(data: CreateTaskData): Promise<Task> {
    // Business logic: validate user exists
    const user = await this.userRepo.findById(data.userId)
    if (!user) {
      throw new Error('User not found')
    }

    // Business logic: set default values
    const task = {
      ...data,
      status: 'pending',
      createdAt: new Date()
    }

    return this.taskRepo.create(task)
  }
}
```

### Repositories Layer
**Purpose**: Abstract database access

**Responsibilities**:
- CRUD operations
- Query construction
- Data mapping
- Transaction support

**DON'T**:
- Contain business logic
- Know about HTTP
- Call other services

**Example**:
```tsx
// apps/backend/src/repositories/task.repository.ts
@injectable()
export class TaskRepository implements ITaskRepository {
  constructor(
    @inject(TYPES.Database) private db: Database
  ) {}

  async create(data: NewTask): Promise<Task> {
    const [task] = await this.db
      .insert(tasks)
      .values(data)
      .returning()
    return task
  }

  async findById(id: string): Promise<Task | null> {
    const task = await this.db
      .select()
      .from(tasks)
      .where(eq(tasks.id, id))
      .limit(1)
    return task[0] || null
  }
}
```

## Dependency Flow

### ✅ Correct Dependencies
```
Route → Service → Repository  ✓
Service → Multiple Repositories  ✓
Service → Service (carefully)  ✓
```

### ❌ Wrong Dependencies
```
Route → Repository  ✗ (skips service layer)
Repository → Service  ✗ (circular dependency)
Repository → Route  ✗ (violates layering)
```

## Complete Example

```tsx
// 1. Repository (Data Access)
@injectable()
export class TaskRepository implements ITaskRepository {
  constructor(@inject(TYPES.Database) private db: Database) {}

  async findAll(): Promise<Task[]> {
    return this.db.select().from(tasks)
  }

  async create(data: NewTask): Promise<Task> {
    const [task] = await this.db.insert(tasks).values(data).returning()
    return task
  }
}

// 2. Service (Business Logic)
@injectable()
export class TaskService {
  constructor(
    @inject(TYPES.TaskRepository) private taskRepo: ITaskRepository,
    @inject(TYPES.NotificationService) private notificationService: INotificationService
  ) {}

  async getAllTasks(): Promise<Task[]> {
    return this.taskRepo.findAll()
  }

  async createTask(data: CreateTaskData): Promise<Task> {
    // Business logic: validate title
    if (data.title.length < 3) {
      throw new Error('Title too short')
    }

    // Create task
    const task = await this.taskRepo.create({
      ...data,
      status: 'pending',
      createdAt: new Date()
    })

    // Business logic: send notification
    await this.notificationService.sendTaskCreated(task)

    return task
  }
}

// 3. Route (HTTP Layer)
app.openapi(routes.tasks.list, async (c) => {
  const taskService = appContainer.get<TaskService>(TYPES.TaskService)
  const tasks = await taskService.getAllTasks()
  return c.json(tasks, 200)
})

app.openapi(routes.tasks.create, async (c) => {
  const taskService = appContainer.get<TaskService>(TYPES.TaskService)
  const body = c.req.valid('json')

  try {
    const task = await taskService.createTask(body)
    return c.json(task, 201)
  } catch (error) {
    if (error.message === 'Title too short') {
      return c.json({ error: error.message }, 400)
    }
    throw error
  }
})
```

## Anti-Patterns

❌ **Business logic in routes**
```tsx
// WRONG
app.post('/tasks', async (c) => {
  const body = await c.req.json()

  // Business logic in route!
  if (body.title.length < 3) {
    return c.json({ error: 'Title too short' }, 400)
  }

  const task = await taskRepo.create(body)
  return c.json(task, 201)
})
```

❌ **Routes calling repositories directly**
```tsx
// WRONG - Skipping service layer
app.get('/tasks', async (c) => {
  const tasks = await taskRepo.findAll()  // Should call service!
  return c.json(tasks)
})
```

❌ **Repositories with business logic**
```tsx
// WRONG
class TaskRepository {
  async create(data: NewTask): Promise<Task> {
    // Business logic in repository!
    if (data.title.length < 3) {
      throw new Error('Title too short')
    }
    return this.db.insert(tasks).values(data)
  }
}
```
