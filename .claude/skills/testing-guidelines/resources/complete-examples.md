# Complete Testing Examples

## Frontend Component Test

```tsx
// src/features/tasks/components/__tests__/task-list.test.tsx
import { describe, test, expect, vi, beforeEach } from 'vitest'
import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { TaskList } from '../task-list'
import { useTasksQuery } from '../../api/use-tasks-query'

vi.mock('../../api/use-tasks-query')

function renderWithProviders(ui: React.ReactElement) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } }
  })
  return render(
    <QueryClientProvider client={queryClient}>
      {ui}
    </QueryClientProvider>
  )
}

describe('TaskList', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  test('displays loading state', () => {
    useTasksQuery.mockReturnValue({
      data: undefined,
      isLoading: true,
      error: null
    })

    renderWithProviders(<TaskList />)

    expect(screen.getByRole('status')).toBeInTheDocument()
    expect(screen.getByText('Loading tasks...')).toBeInTheDocument()
  })

  test('displays error state', () => {
    useTasksQuery.mockReturnValue({
      data: undefined,
      isLoading: false,
      error: new Error('Failed to load tasks')
    })

    renderWithProviders(<TaskList />)

    expect(screen.getByText(/Failed to load tasks/)).toBeInTheDocument()
  })

  test('displays empty state when no tasks', () => {
    useTasksQuery.mockReturnValue({
      data: [],
      isLoading: false,
      error: null
    })

    renderWithProviders(<TaskList />)

    expect(screen.getByText('No tasks yet')).toBeInTheDocument()
  })

  test('displays list of tasks', () => {
    const tasks = [
      { id: '1', title: 'Task 1', status: 'pending' },
      { id: '2', title: 'Task 2', status: 'running' },
      { id: '3', title: 'Task 3', status: 'completed' }
    ]

    useTasksQuery.mockReturnValue({
      data: tasks,
      isLoading: false,
      error: null
    })

    renderWithProviders(<TaskList />)

    expect(screen.getByText('Task 1')).toBeInTheDocument()
    expect(screen.getByText('Task 2')).toBeInTheDocument()
    expect(screen.getByText('Task 3')).toBeInTheDocument()
  })

  test('navigates to task detail on click', async () => {
    const user = userEvent.setup()
    const mockNavigate = vi.fn()

    vi.mock('@tanstack/react-router', () => ({
      useNavigate: () => mockNavigate,
      Link: ({ to, params, children }) => (
        <a href="#" onClick={() => mockNavigate({ to, params })}>
          {children}
        </a>
      )
    }))

    const tasks = [{ id: '1', title: 'Task 1', status: 'pending' }]
    useTasksQuery.mockReturnValue({
      data: tasks,
      isLoading: false,
      error: null
    })

    renderWithProviders(<TaskList />)

    await user.click(screen.getByText('Task 1'))

    expect(mockNavigate).toHaveBeenCalledWith({
      to: '/tasks/$taskId',
      params: { taskId: '1' }
    })
  })
})
```

## Backend Service Test

```tsx
// apps/backend/src/services/__tests__/task.service.test.ts
import { describe, test, expect, vi, beforeEach } from 'vitest'
import { Container } from 'inversify'
import { TaskService } from '../task.service'
import { TYPES } from '../../types'
import type { ITaskRepository } from '../../repositories/task.repository.interface'
import type { INotificationService } from '../notification.service.interface'

describe('TaskService', () => {
  let container: Container
  let taskService: TaskService
  let mockTaskRepo: ITaskRepository
  let mockNotificationService: INotificationService

  beforeEach(() => {
    container = new Container()

    mockTaskRepo = {
      findAll: vi.fn(),
      findById: vi.fn(),
      create: vi.fn(),
      update: vi.fn(),
      delete: vi.fn()
    }

    mockNotificationService = {
      sendTaskCreated: vi.fn(),
      sendTaskUpdated: vi.fn()
    }

    container.bind<ITaskRepository>(TYPES.TaskRepository).toConstantValue(mockTaskRepo)
    container.bind<INotificationService>(TYPES.NotificationService).toConstantValue(mockNotificationService)
    container.bind<TaskService>(TYPES.TaskService).to(TaskService)

    taskService = container.get<TaskService>(TYPES.TaskService)
  })

  describe('getAllTasks', () => {
    test('returns all tasks from repository', async () => {
      const mockTasks = [
        { id: '1', title: 'Task 1', status: 'pending' },
        { id: '2', title: 'Task 2', status: 'running' }
      ]
      mockTaskRepo.findAll.mockResolvedValue(mockTasks)

      const tasks = await taskService.getAllTasks()

      expect(tasks).toEqual(mockTasks)
      expect(mockTaskRepo.findAll).toHaveBeenCalledOnce()
    })
  })

  describe('createTask', () => {
    test('creates task with valid data', async () => {
      const taskData = { title: 'New Task', description: 'Description' }
      const createdTask = { id: '1', ...taskData, status: 'pending' }

      mockTaskRepo.create.mockResolvedValue(createdTask)

      const task = await taskService.createTask(taskData)

      expect(task).toEqual(createdTask)
      expect(mockTaskRepo.create).toHaveBeenCalledWith(
        expect.objectContaining({
          title: 'New Task',
          description: 'Description',
          status: 'pending'
        })
      )
    })

    test('throws error when title is too short', async () => {
      const taskData = { title: 'ab' }

      await expect(taskService.createTask(taskData)).rejects.toThrow(
        'Title must be at least 3 characters'
      )

      expect(mockTaskRepo.create).not.toHaveBeenCalled()
    })

    test('sends notification after creation', async () => {
      const taskData = { title: 'New Task' }
      const createdTask = { id: '1', ...taskData, status: 'pending' }

      mockTaskRepo.create.mockResolvedValue(createdTask)

      await taskService.createTask(taskData)

      expect(mockNotificationService.sendTaskCreated).toHaveBeenCalledWith(createdTask)
    })
  })

  describe('updateTask', () => {
    test('updates existing task', async () => {
      const existingTask = { id: '1', title: 'Old Title', status: 'pending' }
      const updates = { title: 'New Title' }
      const updatedTask = { ...existingTask, ...updates }

      mockTaskRepo.findById.mockResolvedValue(existingTask)
      mockTaskRepo.update.mockResolvedValue(updatedTask)

      const task = await taskService.updateTask('1', updates)

      expect(task.title).toBe('New Title')
      expect(mockTaskRepo.update).toHaveBeenCalledWith('1', updates)
    })

    test('throws error when task not found', async () => {
      mockTaskRepo.findById.mockResolvedValue(null)

      await expect(taskService.updateTask('1', { title: 'New' })).rejects.toThrow(
        'Task not found'
      )

      expect(mockTaskRepo.update).not.toHaveBeenCalled()
    })

    test('prevents status change for completed tasks', async () => {
      const completedTask = { id: '1', title: 'Task', status: 'completed' }
      mockTaskRepo.findById.mockResolvedValue(completedTask)

      await expect(
        taskService.updateTask('1', { status: 'running' })
      ).rejects.toThrow('Cannot change status of completed task')

      expect(mockTaskRepo.update).not.toHaveBeenCalled()
    })
  })

  describe('deleteTask', () => {
    test('deletes task successfully', async () => {
      const task = { id: '1', title: 'Task', status: 'pending' }
      mockTaskRepo.findById.mockResolvedValue(task)

      await taskService.deleteTask('1')

      expect(mockTaskRepo.delete).toHaveBeenCalledWith('1')
    })

    test('throws error when task not found', async () => {
      mockTaskRepo.findById.mockResolvedValue(null)

      await expect(taskService.deleteTask('1')).rejects.toThrow('Task not found')

      expect(mockTaskRepo.delete).not.toHaveBeenCalled()
    })

    test('prevents deletion of running tasks', async () => {
      const runningTask = { id: '1', title: 'Task', status: 'running' }
      mockTaskRepo.findById.mockResolvedValue(runningTask)

      await expect(taskService.deleteTask('1')).rejects.toThrow(
        'Cannot delete running task'
      )

      expect(mockTaskRepo.delete).not.toHaveBeenCalled()
    })
  })
})
```

## Integration Test

```tsx
// apps/backend/src/routes/__tests__/tasks.integration.test.ts
import { describe, test, expect, beforeAll, afterAll, beforeEach } from 'vitest'
import { testClient } from 'hono/testing'
import app from '../../app'
import { db } from '../../db'
import { tasks } from '../../schema'

describe('Tasks API Integration', () => {
  beforeAll(async () => {
    // Setup test database
  })

  beforeEach(async () => {
    // Clean database before each test
    await db.delete(tasks)
  })

  afterAll(async () => {
    // Cleanup
  })

  test('GET /api/v1/tasks returns empty array initially', async () => {
    const client = testClient(app)

    const res = await client.api.v1.tasks.$get()
    expect(res.status).toBe(200)

    const data = await res.json()
    expect(data).toEqual([])
  })

  test('POST /api/v1/tasks creates new task', async () => {
    const client = testClient(app)

    const res = await client.api.v1.tasks.$post({
      json: {
        title: 'Test Task',
        description: 'Test description'
      }
    })

    expect(res.status).toBe(201)

    const task = await res.json()
    expect(task).toMatchObject({
      title: 'Test Task',
      description: 'Test description',
      status: 'pending'
    })
    expect(task.id).toBeDefined()
  })

  test('GET /api/v1/tasks/:id returns task', async () => {
    const client = testClient(app)

    // Create task
    const createRes = await client.api.v1.tasks.$post({
      json: { title: 'Test Task' }
    })
    const created = await createRes.json()

    // Get task
    const getRes = await client.api.v1.tasks[':id'].$get({
      param: { id: created.id }
    })

    expect(getRes.status).toBe(200)

    const task = await getRes.json()
    expect(task).toEqual(created)
  })

  test('returns 404 for non-existent task', async () => {
    const client = testClient(app)

    const res = await client.api.v1.tasks[':id'].$get({
      param: { id: 'non-existent' }
    })

    expect(res.status).toBe(404)
  })
})
```

## Related Resources
- [Frontend Testing](../../frontend-dev-guidelines/resources/testing.md) - Frontend patterns
- [Backend Testing](../../backend-dev-guidelines/resources/testing.md) - Backend patterns
- [Testing Philosophy](./testing-philosophy.md) - Core principles
