import type { Command, Task } from '@moatless/types'
import { Container } from 'inversify'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { CommandService } from '../../../agents/services/command.service'
import { TaskRepository } from '../../db/task.repository'
import { TaskCommandGateway } from '../task-command-gateway.service'

describe('TaskCommandGateway', () => {
  let container: Container
  let service: TaskCommandGateway
  let mockCommandService: {
    listCommands: ReturnType<typeof vi.fn>
  }
  let mockTaskRepository: {
    get: ReturnType<typeof vi.fn>
  }

  beforeEach(() => {
    container = new Container()

    // Create mocks
    mockCommandService = {
      listCommands: vi.fn(),
    }

    mockTaskRepository = {
      get: vi.fn(),
    }

    // Bind mocks to container
    container.bind(CommandService).toConstantValue(mockCommandService as any)
    container.bind(TaskRepository).toConstantValue(mockTaskRepository as any)
    container.bind(TaskCommandGateway).toSelf()

    service = container.get(TaskCommandGateway)
  })

  afterEach(() => {
    vi.clearAllMocks()
  })

  describe('getAvailableCommands', () => {
    it('should return all commands when no conditions are specified', async () => {
      const mockTask: Task = {
        taskId: 'task-123',
        repositoryId: 'repo-456',
        agentType: 'claude-code',
        status: 'open' as const,
        createdAt: new Date(),
        lastMessageAt: new Date(),
        numTurns: 0,
        totalCostUsd: 0,

        visibility: 'private' as const,
      }

      const mockCommands: Command[] = [
        {
          name: 'test1',
          description: 'Test command 1',
          prompt: 'test prompt 1',
        },
        {
          name: 'test2',
          description: 'Test command 2',
          prompt: 'test prompt 2',
        },
      ]

      ;(mockTaskRepository.get as any).mockResolvedValue(mockTask)
      ;(mockCommandService.listCommands as any).mockResolvedValue(mockCommands)

      const result = await service.getAvailableCommands('task-123')

      expect(mockTaskRepository.get).toHaveBeenCalledWith('task-123')
      expect(mockCommandService.listCommands).toHaveBeenCalled()
      expect(result).toEqual(mockCommands)
    })

    it('should filter out inactive commands', async () => {
      const mockTask: Task = {
        taskId: 'task-123',
        repositoryId: 'repo-456',
        agentType: 'claude-code',
        status: 'open' as const,
        createdAt: new Date(),
        lastMessageAt: new Date(),
        numTurns: 0,
        totalCostUsd: 0,

        visibility: 'private' as const,
      }

      const mockCommands: Command[] = [
        {
          name: 'active-cmd',
          description: 'Active command',
          prompt: 'active prompt',
          active: true,
        },
        {
          name: 'inactive-cmd',
          description: 'Inactive command',
          prompt: 'inactive prompt',
          active: false,
        },
      ]

      ;(mockTaskRepository.get as any).mockResolvedValue(mockTask)
      ;(mockCommandService.listCommands as any).mockResolvedValue(mockCommands)

      const result = await service.getAvailableCommands('task-123')

      expect(result).toHaveLength(1)
      expect(result[0]?.name).toBe('active-cmd')
    })

    it('should filter commands based on task error status condition', async () => {
      const mockTask: Task = {
        taskId: 'task-123',
        repositoryId: 'repo-456',
        agentType: 'claude-code',
        status: 'error' as const,
        createdAt: new Date(),
        lastMessageAt: new Date(),
        numTurns: 1,
        totalCostUsd: 0.1,
        visibility: 'private' as const,
      }

      const mockCommands: Command[] = [
        {
          name: 'continue',
          description: 'Continue after error',
          prompt: 'continue with the task',
          conditions: {
            requiresLastIterationError: true,
          },
        },
      ]

      ;(mockTaskRepository.get as any).mockResolvedValue(mockTask)
      ;(mockCommandService.listCommands as any).mockResolvedValue(mockCommands)

      const result = await service.getAvailableCommands('task-123')

      expect(result).toHaveLength(1)
      expect(result[0]?.name).toBe('continue')
    })

    it('should exclude continue command when task is in open status', async () => {
      const mockTask: Task = {
        taskId: 'task-123',
        repositoryId: 'repo-456',
        agentType: 'claude-code',
        status: 'open' as const,
        createdAt: new Date(),
        lastMessageAt: new Date(),
        numTurns: 1,
        totalCostUsd: 0.1,
        visibility: 'private' as const,
      }

      const mockCommands: Command[] = [
        {
          name: 'continue',
          description: 'Continue after error',
          prompt: 'continue with the task',
          conditions: {
            requiresLastIterationError: true,
          },
        },
      ]

      ;(mockTaskRepository.get as any).mockResolvedValue(mockTask)
      ;(mockCommandService.listCommands as any).mockResolvedValue(mockCommands)

      const result = await service.getAvailableCommands('task-123')

      expect(result).toHaveLength(0)
    })

    it('should build context with task only', async () => {
      const mockTask: Task = {
        taskId: 'task-123',
        repositoryId: 'repo-456',
        agentType: 'claude-code',
        status: 'open' as const,
        createdAt: new Date(),
        lastMessageAt: new Date(),
        numTurns: 1,
        totalCostUsd: 0.05,
        visibility: 'private' as const,
      }

      const mockCommands: Command[] = []

      ;(mockTaskRepository.get as any).mockResolvedValue(mockTask)
      ;(mockCommandService.listCommands as any).mockResolvedValue(mockCommands)

      await service.getAvailableCommands('task-123')

      expect(mockTaskRepository.get).toHaveBeenCalledWith('task-123')
      expect(mockCommandService.listCommands).toHaveBeenCalled()
    })
  })
})
