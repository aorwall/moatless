# Dependency Injection with InversifyJS

## Overview
All backend services use InversifyJS for dependency injection, enabling testability and loose coupling.

## Key Decorators

### @injectable()
Mark classes as injectable:
```tsx
import { injectable } from 'inversify'

@injectable()
export class TaskService {
  // Service implementation
}
```

### @inject()
Inject dependencies (REQUIRED on ALL constructor parameters):
```tsx
import { injectable, inject } from 'inversify'
import { TYPES } from '../types'

@injectable()
export class TaskService {
  constructor(
    @inject(TYPES.TaskRepository) private taskRepo: ITaskRepository,
    @inject(TYPES.Logger) private logger: ILogger
  ) {}
}
```

## Container Setup

### Define Type Symbols
```tsx
// apps/backend/src/types.ts
export const TYPES = {
  // Repositories
  TaskRepository: Symbol.for('TaskRepository'),
  UserRepository: Symbol.for('UserRepository'),

  // Services
  TaskService: Symbol.for('TaskService'),
  UserService: Symbol.for('UserService'),

  // Infrastructure
  Database: Symbol.for('Database'),
  Logger: Symbol.for('Logger')
}
```

### Configure Container
```tsx
// apps/backend/src/container.ts
import { Container } from 'inversify'
import { TYPES } from './types'
import { TaskService } from './services/task.service'
import { TaskRepository } from './repositories/task.repository'

export const appContainer = new Container()

// Bind repositories
appContainer.bind<ITaskRepository>(TYPES.TaskRepository).to(TaskRepository)
appContainer.bind<IUserRepository>(TYPES.UserRepository).to(UserRepository)

// Bind services
appContainer.bind<TaskService>(TYPES.TaskService).to(TaskService)
appContainer.bind<UserService>(TYPES.UserService).to(UserService)

// Bind infrastructure
appContainer.bind<Database>(TYPES.Database).toConstantValue(db)
appContainer.bind<ILogger>(TYPES.Logger).to(Logger).inSingletonScope()
```

## Usage in Routes

```tsx
// apps/backend/src/routes/tasks.ts
import { appContainer } from '../container'
import { TYPES } from '../types'
import type { TaskService } from '../services/task.service'

app.openapi(routes.tasks.list, async (c) => {
  const taskService = appContainer.get<TaskService>(TYPES.TaskService)
  const tasks = await taskService.getAllTasks()
  return c.json(tasks, 200)
})
```

## Interface-Based Injection

### Define Interface
```tsx
// apps/backend/src/repositories/task.repository.interface.ts
export interface ITaskRepository {
  findAll(): Promise<Task[]>
  findById(id: string): Promise<Task | null>
  create(data: NewTask): Promise<Task>
  update(id: string, data: Partial<Task>): Promise<Task>
  delete(id: string): Promise<void>
}
```

### Implement Interface
```tsx
// apps/backend/src/repositories/task.repository.ts
@injectable()
export class TaskRepository implements ITaskRepository {
  constructor(@inject(TYPES.Database) private db: Database) {}

  async findAll(): Promise<Task[]> {
    return this.db.select().from(tasks)
  }

  // ... other methods
}
```

### Inject Interface
```tsx
@injectable()
export class TaskService {
  constructor(
    @inject(TYPES.TaskRepository) private taskRepo: ITaskRepository  // Interface!
  ) {}
}
```

## Testing with DI

### Create Test Container
```tsx
// __tests__/test-container.ts
import { Container } from 'inversify'
import { TYPES } from '../types'

export function createTestContainer() {
  const container = new Container()

  // Bind mocks
  const mockTaskRepo: ITaskRepository = {
    findAll: vi.fn(),
    findById: vi.fn(),
    create: vi.fn(),
    update: vi.fn(),
    delete: vi.fn()
  }

  container.bind<ITaskRepository>(TYPES.TaskRepository).toConstantValue(mockTaskRepo)

  return { container, mockTaskRepo }
}
```

### Unit Test with DI
```tsx
describe('TaskService', () => {
  test('creates task', async () => {
    const { container, mockTaskRepo } = createTestContainer()
    container.bind<TaskService>(TYPES.TaskService).to(TaskService)

    const taskService = container.get<TaskService>(TYPES.TaskService)

    mockTaskRepo.create.mockResolvedValue({ id: '1', title: 'Test' })

    const task = await taskService.createTask({ title: 'Test' })

    expect(task.id).toBe('1')
    expect(mockTaskRepo.create).toHaveBeenCalled()
  })
})
```

## Scope Management

### Transient (default)
New instance each time:
```tsx
appContainer.bind<TaskService>(TYPES.TaskService).to(TaskService)
```

### Singleton
Single instance shared:
```tsx
appContainer.bind<Logger>(TYPES.Logger).to(Logger).inSingletonScope()
```

### Request Scope
Per-request instance (for HTTP contexts):
```tsx
appContainer.bind<RequestContext>(TYPES.RequestContext).to(RequestContext).inRequestScope()
```

## Anti-Patterns

❌ **Missing @inject() decorator**
```tsx
// WRONG - Will fail at runtime!
@injectable()
class TaskService {
  constructor(private taskRepo: TaskRepository) {}  // Missing @inject!
}

// CORRECT
@injectable()
class TaskService {
  constructor(
    @inject(TYPES.TaskRepository) private taskRepo: ITaskRepository
  ) {}
}
```

❌ **Direct instantiation**
```tsx
// WRONG
const taskService = new TaskService(new TaskRepository(db))

// CORRECT
const taskService = appContainer.get<TaskService>(TYPES.TaskService)
```

❌ **Concrete class injection instead of interface**
```tsx
// WRONG - Tightly coupled
@injectable()
class TaskService {
  constructor(
    @inject(TYPES.TaskRepository) private taskRepo: TaskRepository  // Concrete class!
  ) {}
}

// CORRECT - Loosely coupled
@injectable()
class TaskService {
  constructor(
    @inject(TYPES.TaskRepository) private taskRepo: ITaskRepository  // Interface!
  ) {}
}
```
