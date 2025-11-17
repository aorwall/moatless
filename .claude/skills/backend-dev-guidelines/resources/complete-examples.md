# Complete Backend Examples

## Complete CRUD Feature

### 1. API Contract
```tsx
// packages/api-contract/src/routes/tasks.ts
import { createRoute, z } from '@hono/zod-openapi'

const TaskSchema = z.object({
  id: z.string(),
  title: z.string(),
  status: z.enum(['pending', 'running', 'completed'])
})

export const routes = {
  tasks: {
    list: createRoute({
      method: 'get',
      path: '/api/v1/tasks',
      responses: {
        200: {
          content: { 'application/json': { schema: z.array(TaskSchema) } }
        }
      }
    }),
    create: createRoute({
      method: 'post',
      path: '/api/v1/tasks',
      request: {
        body: {
          content: {
            'application/json': {
              schema: z.object({
                title: z.string().min(1)
              })
            }
          }
        }
      },
      responses: {
        201: {
          content: { 'application/json': { schema: TaskSchema } }
        }
      }
    })
  }
}
```

### 2. Repository
```tsx
// apps/backend/src/repositories/task.repository.ts
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
```

### 3. Service
```tsx
// apps/backend/src/services/task.service.ts
@injectable()
export class TaskService {
  constructor(
    @inject(TYPES.TaskRepository) private taskRepo: ITaskRepository
  ) {}

  async getAllTasks(): Promise<Task[]> {
    return this.taskRepo.findAll()
  }

  async createTask(data: CreateTaskData): Promise<Task> {
    if (data.title.length < 3) {
      throw new ValidationError('Title must be at least 3 characters')
    }

    return this.taskRepo.create({
      ...data,
      status: 'pending',
      createdAt: new Date()
    })
  }
}
```

### 4. Routes
```tsx
// apps/backend/src/routes/tasks.ts
import { OpenAPIHono } from '@hono/zod-openapi'
import { routes } from '@moatless/api-contract'
import { appContainer } from '../container'
import { TYPES } from '../types'
import { handleServiceError } from '../utils/error-handler'

const app = new OpenAPIHono()

app.openapi(routes.tasks.list, async (c) => {
  try {
    const taskService = appContainer.get<TaskService>(TYPES.TaskService)
    const tasks = await taskService.getAllTasks()
    return c.json(tasks, 200)
  } catch (error) {
    return handleServiceError(c, error)
  }
})

app.openapi(routes.tasks.create, async (c) => {
  try {
    const taskService = appContainer.get<TaskService>(TYPES.TaskService)
    const body = c.req.valid('json')
    const task = await taskService.createTask(body)
    return c.json(task, 201)
  } catch (error) {
    return handleServiceError(c, error)
  }
})

export default app
```

### 5. Container Registration
```tsx
// apps/backend/src/container.ts
import { Container } from 'inversify'
import { TYPES } from './types'
import { TaskService } from './services/task.service'
import { TaskRepository } from './repositories/task.repository'

export const appContainer = new Container()

appContainer.bind<ITaskRepository>(TYPES.TaskRepository).to(TaskRepository)
appContainer.bind<TaskService>(TYPES.TaskService).to(TaskService)
```

### 6. Unit Tests
```tsx
// apps/backend/src/services/__tests__/task.service.test.ts
describe('TaskService', () => {
  let container: Container
  let taskService: TaskService
  let mockTaskRepo: ITaskRepository

  beforeEach(() => {
    container = new Container()
    mockTaskRepo = {
      findAll: vi.fn(),
      create: vi.fn()
    }
    container.bind<ITaskRepository>(TYPES.TaskRepository).toConstantValue(mockTaskRepo)
    container.bind<TaskService>(TYPES.TaskService).to(TaskService)
    taskService = container.get<TaskService>(TYPES.TaskService)
  })

  test('createTask validates title', async () => {
    await expect(
      taskService.createTask({ title: 'ab' })
    ).rejects.toThrow('Title must be at least 3 characters')
  })

  test('createTask sets default status', async () => {
    mockTaskRepo.create.mockResolvedValue({ id: '1', title: 'Task', status: 'pending' })

    await taskService.createTask({ title: 'Task' })

    expect(mockTaskRepo.create).toHaveBeenCalledWith(
      expect.objectContaining({
        status: 'pending'
      })
    )
  })
})
```

## Related Resources
- [API Contract Pattern](./api-contract-pattern.md) - Contract definition
- [Layered Architecture](./layered-architecture.md) - Layer organization
- [Dependency Injection](./dependency-injection.md) - DI setup
- [Service Layer](./service-layer.md) - Business logic
- [Repository Pattern](./repository-pattern.md) - Data access
- [Testing](./testing.md) - Test patterns
