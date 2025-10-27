import type { Task } from '@moatless/types'
import { TaskNotFoundError, ValidationError } from '@moatless/types'
import { Container } from 'inversify'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { RepositoryService } from '../../../repositories/index'
import { TaskMetadataStorage } from '../../storage/task-metadata.storage'
import { TaskLifecycleService } from '../task-lifecycle.service'

describe('TaskLifecycleService', () => {
  let container: Container
  let service: TaskLifecycleService
  let mockTaskStorage: {
    get: ReturnType<typeof vi.fn>
    save: ReturnType<typeof vi.fn>
    delete: ReturnType<typeof vi.fn>
    exists: ReturnType<typeof vi.fn>
    listTaskKeys: ReturnType<typeof vi.fn>
  }
  let mockRepositoryService: {
    getRepositoryById: ReturnType<typeof vi.fn>
  }

  beforeEach(() => {
    container = new Container()

    // Create mock storage
    mockTaskStorage = {
      get: vi.fn(),
      save: vi.fn(),
      delete: vi.fn(),
      exists: vi.fn(),
      listTaskKeys: vi.fn(),
    }

    // Create mock repository service
    mockRepositoryService = {
      getRepositoryById: vi.fn(),
    }

    // Bind mocks to container
    container.bind(TaskMetadataStorage).toConstantValue(mockTaskStorage as any)
    container.bind(RepositoryService).toConstantValue(mockRepositoryService as any)
    container.bind(TaskLifecycleService).toSelf()

    service = container.get(TaskLifecycleService)
  })

  afterEach(() => {
    vi.clearAllMocks()
  })

  describe('getTask', () => {
    it('should get task and enrich with title and repository name', async () => {
      const mockTask: Task = {
        taskId: 'task-123',
        repositoryId: 'repo-456',
        agentType: 'claude-code',
        status: 'open' as const,
        createdAt: new Date(),
        lastMessageAt: new Date(),
        numTurns: 0,
        totalCostUsd: 0,
        iterations: [],
      }

      const mockRepository = {
        id: 'repo-456',
        name: 'test-repo',
      }

      ;(mockTaskStorage.get as any).mockResolvedValue(mockTask)
      ;(mockRepositoryService.getRepositoryById as any).mockResolvedValue(mockRepository)

      const result = await service.getTask('task-123')

      expect(mockTaskStorage.get).toHaveBeenCalledWith('task-123')
      expect(mockRepositoryService.getRepositoryById).toHaveBeenCalledWith('repo-456')
      expect(result).toMatchObject({
        ...mockTask,
        title: 'claude-code', // falls back to agentType
        repositoryName: 'test-repo',
      })
    })

    it('should throw TaskNotFoundError when task does not exist', async () => {
      ;(mockTaskStorage.get as any).mockResolvedValue(null)

      await expect(service.getTask('non-existent')).rejects.toThrow(TaskNotFoundError)
    })

    it('should derive running status when last iteration is active', async () => {
      const mockTask: Task = {
        taskId: 'task-123',
        repositoryId: 'repo-456',
        agentType: 'claude-code',
        status: 'open' as const,
        createdAt: new Date(),
        lastMessageAt: new Date(),
        numTurns: 0,
        totalCostUsd: 0,
        iterations: [
          {
            iterationNumber: 1,
            status: 'active' as const,
            startTime: new Date(),
            messageCount: 1,
            costUsd: 0,
          },
        ],
      }

      ;(mockTaskStorage.get as any).mockResolvedValue(mockTask)
      ;(mockRepositoryService.getRepositoryById as any).mockResolvedValue({ name: 'test-repo' })

      const result = await service.getTask('task-123')

      expect(result.status).toBe('running')
    })
  })

  describe('listTasks', () => {
    it('should list tasks with pagination and filtering', async () => {
      const mockTasks: Task[] = [
        {
          taskId: 'task-1',
          repositoryId: 'repo-1',
          agentType: 'claude-code',
          status: 'open' as const,
          createdAt: new Date('2024-01-01'),
          lastMessageAt: new Date('2024-01-03'),
          numTurns: 0,
          totalCostUsd: 0,
          iterations: [],
        },
        {
          taskId: 'task-2',
          repositoryId: 'repo-2',
          agentType: 'claude-code',
          status: 'open' as const,
          createdAt: new Date('2024-01-02'),
          lastMessageAt: new Date('2024-01-02'),
          numTurns: 0,
          totalCostUsd: 0,
          iterations: [],
        },
        {
          taskId: 'task-3',
          repositoryId: 'repo-3',
          agentType: 'claude-code',
          status: 'closed' as const,
          createdAt: new Date('2024-01-01'),
          lastMessageAt: new Date('2024-01-01'),
          numTurns: 0,
          totalCostUsd: 0,
          iterations: [],
        },
      ]

      ;(mockTaskStorage.listTaskKeys as any).mockResolvedValue([
        'tasks/task-1',
        'tasks/task-2',
        'tasks/task-3',
      ])
      ;(mockTaskStorage.get as any).mockImplementation(async (taskId: string) => {
        return mockTasks.find((t) => t.taskId === taskId)
      })

      const result = await service.listTasks(50, 0)

      // Should exclude closed tasks by default and sort by lastMessageAt desc
      expect(result.tasks).toHaveLength(2)
      expect(result.tasks[0]?.taskId).toBe('task-1') // Most recent lastMessageAt
      expect(result.tasks[1]?.taskId).toBe('task-2')
      expect(result.total).toBe(2)
    })

    it('should include closed tasks when status filter is provided', async () => {
      const closedTask: Task = {
        taskId: 'task-closed',
        repositoryId: 'repo-1',
        agentType: 'claude-code',
        status: 'closed' as const,
        createdAt: new Date(),
        lastMessageAt: new Date(),
        numTurns: 0,
        totalCostUsd: 0,
        iterations: [],
      }

      ;(mockTaskStorage.listTaskKeys as any).mockResolvedValue(['tasks/task-closed'])
      ;(mockTaskStorage.get as any).mockResolvedValue(closedTask)

      const result = await service.listTasks(50, 0, 'closed')

      expect(result.tasks).toHaveLength(1)
      expect(result.tasks[0]?.status).toBe('closed')
    })
  })

  describe('updateTaskMetadata', () => {
    it('should update task name and description', async () => {
      const mockTask: Task = {
        taskId: 'task-123',
        repositoryId: 'repo-456',
        agentType: 'claude-code',
        status: 'open' as const,
        createdAt: new Date(),
        lastMessageAt: new Date(),
        numTurns: 0,
        totalCostUsd: 0,
        iterations: [],
      }

      ;(mockTaskStorage.get as any).mockResolvedValue(mockTask)
      ;(mockRepositoryService.getRepositoryById as any).mockResolvedValue({ name: 'test-repo' })
      ;(mockTaskStorage.save as any).mockResolvedValue(undefined)

      await service.updateTaskMetadata('task-123', {
        name: 'my-task',
        description: 'Test description',
      })

      expect(mockTaskStorage.save).toHaveBeenCalledWith(
        expect.objectContaining({
          name: 'my-task',
          description: 'Test description',
        }),
      )
    })

    it('should throw ValidationError for invalid task name', async () => {
      const mockTask: Task = {
        taskId: 'task-123',
        repositoryId: 'repo-456',
        agentType: 'claude-code',
        status: 'open' as const,
        createdAt: new Date(),
        lastMessageAt: new Date(),
        numTurns: 0,
        totalCostUsd: 0,
        iterations: [],
      }

      ;(mockTaskStorage.get as any).mockResolvedValue(mockTask)
      ;(mockRepositoryService.getRepositoryById as any).mockResolvedValue({ name: 'test-repo' })

      await expect(
        service.updateTaskMetadata('task-123', {
          name: 'invalid name with spaces',
        }),
      ).rejects.toThrow(ValidationError)
    })
  })

  describe('updateTask', () => {
    it('should update task with partial updates', async () => {
      const mockTask: Task = {
        taskId: 'task-123',
        repositoryId: 'repo-456',
        agentType: 'claude-code',
        status: 'open' as const,
        createdAt: new Date(),
        lastMessageAt: new Date(),
        numTurns: 0,
        totalCostUsd: 0,
        iterations: [],
      }

      ;(mockTaskStorage.get as any).mockResolvedValue(mockTask)
      ;(mockRepositoryService.getRepositoryById as any).mockResolvedValue({ name: 'test-repo' })
      ;(mockTaskStorage.save as any).mockResolvedValue(undefined)

      await service.updateTask('task-123', {
        status: 'closed' as const,
        totalCostUsd: 1.5,
      })

      expect(mockTaskStorage.save).toHaveBeenCalledWith(
        expect.objectContaining({
          taskId: 'task-123',
          status: 'closed',
          totalCostUsd: 1.5,
        }),
      )
    })
  })
})
