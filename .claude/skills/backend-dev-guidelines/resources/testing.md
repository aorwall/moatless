# Backend Testing

## Overview
Backend tests use Vitest with mandatory unit tests for services and integration tests for critical flows.

## Unit Testing Services

```tsx
// apps/backend/src/services/__tests__/task.service.test.ts
import { describe, test, expect, vi, beforeEach } from 'vitest'
import { Container } from 'inversify'
import { TYPES } from '../../types'
import { TaskService } from '../task.service'
import type { ITaskRepository } from '../../repositories/task.repository.interface'

describe('TaskService', () => {
  let container: Container
  let taskService: TaskService
  let mockTaskRepo: ITaskRepository

  beforeEach(() => {
    container = new Container()

    // Mock repository
    mockTaskRepo = {
      findAll: vi.fn(),
      findById: vi.fn(),
      create: vi.fn(),
      update: vi.fn(),
      delete: vi.fn()
    }

    container.bind<ITaskRepository>(TYPES.TaskRepository).toConstantValue(mockTaskRepo)
    container.bind<TaskService>(TYPES.TaskService).to(TaskService)

    taskService = container.get<TaskService>(TYPES.TaskService)
  })

  test('getAllTasks returns all tasks', async () => {
    const mockTasks = [
      { id: '1', title: 'Task 1', status: 'pending' },
      { id: '2', title: 'Task 2', status: 'running' }
    ]
    mockTaskRepo.findAll.mockResolvedValue(mockTasks)

    const tasks = await taskService.getAllTasks()

    expect(tasks).toEqual(mockTasks)
    expect(mockTaskRepo.findAll).toHaveBeenCalled()
  })

  test('createTask validates title length', async () => {
    await expect(
      taskService.createTask({ title: 'ab' })
    ).rejects.toThrow('Title must be at least 3 characters')

    expect(mockTaskRepo.create).not.toHaveBeenCalled()
  })

  test('createTask sets default status', async () => {
    mockTaskRepo.create.mockResolvedValue({ id: '1', title: 'Task', status: 'pending' })

    await taskService.createTask({ title: 'Task' })

    expect(mockTaskRepo.create).toHaveBeenCalledWith(
      expect.objectContaining({
        title: 'Task',
        status: 'pending'
      })
    )
  })
})
```

## Integration Testing

```tsx
// apps/backend/src/routes/__tests__/tasks.integration.test.ts
import { describe, test, expect, beforeAll, afterAll } from 'vitest'
import { testClient } from 'hono/testing'
import app from '../../app'
import { db } from '../../db'
import { tasks } from '../../schema'

describe('Tasks API Integration', () => {
  beforeAll(async () => {
    // Setup test database
    await db.delete(tasks)
  })

  afterAll(async () => {
    // Cleanup
    await db.delete(tasks)
  })

  test('POST /api/v1/tasks creates task', async () => {
    const client = testClient(app)

    const res = await client.api.v1.tasks.$post({
      json: {
        title: 'Test Task',
        description: 'Test description'
      }
    })

    expect(res.status).toBe(201)

    const task = await res.json()
    expect(task.title).toBe('Test Task')
    expect(task.status).toBe('pending')
  })

  test('GET /api/v1/tasks returns all tasks', async () => {
    const client = testClient(app)

    const res = await client.api.v1.tasks.$get()
    expect(res.status).toBe(200)

    const tasks = await res.json()
    expect(Array.isArray(tasks)).toBe(true)
  })
})
```

## Testing Repositories

```tsx
// apps/backend/src/repositories/__tests__/task.repository.test.ts
import { describe, test, expect, beforeEach, afterEach } from 'vitest'
import { TaskRepository } from '../task.repository'
import { db } from '../../db'
import { tasks } from '../../schema'

describe('TaskRepository', () => {
  let repo: TaskRepository

  beforeEach(async () => {
    repo = new TaskRepository(db)
    await db.delete(tasks)
  })

  afterEach(async () => {
    await db.delete(tasks)
  })

  test('create inserts task', async () => {
    const task = await repo.create({
      title: 'Test Task',
      status: 'pending'
    })

    expect(task.id).toBeDefined()
    expect(task.title).toBe('Test Task')
  })

  test('findById returns task', async () => {
    const created = await repo.create({ title: 'Test', status: 'pending' })
    const found = await repo.findById(created.id)

    expect(found).toEqual(created)
  })
})
```

## Test Naming

```bash
# Unit tests (fast, <200ms)
task.service.test.ts
user.service.test.ts

# Integration tests (moderate, 200ms-2s)
tasks.integration.test.ts
auth.integration.test.ts

# E2E tests (slow, >2s)
task-workflow.e2e.test.ts
```
