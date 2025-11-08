import type { Command, GitStatus, Task, TaskIteration } from '@moatless/types'
import { Container } from 'inversify'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { CommandService } from '../../../agents/services/command.service'
import { FileGateway } from '../../../workspace/services/file-gateway.service'
import { TaskRepository } from '../../db/task.repository'
import { TaskCommandGateway } from '../task-command-gateway.service'

describe('TaskCommandGateway', () => {
  let container: Container
  let service: TaskCommandGateway
  let mockCommandService: {
    listCommands: ReturnType<typeof vi.fn>
  }
  let mockFileGateway: {
    getGitStatus: ReturnType<typeof vi.fn>
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

    mockFileGateway = {
      getGitStatus: vi.fn(),
    }

    mockTaskRepository = {
      get: vi.fn(),
    }

    // Bind mocks to container
    container.bind(CommandService).toConstantValue(mockCommandService as any)
    container.bind(FileGateway).toConstantValue(mockFileGateway as any)
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
        iterations: [],
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

      const mockGitStatus: GitStatus = {
        currentBranch: 'main',
        baseBranch: 'main',
        hasUncommittedChanges: false,
        hasConflicts: false,
        commitsAhead: 0,
        commitsBehind: 0,
        status: 'synced',
        originExists: true,
      }

      ;(mockTaskRepository.get as any).mockResolvedValue(mockTask)
      ;(mockCommandService.listCommands as any).mockResolvedValue(mockCommands)
      ;(mockFileGateway.getGitStatus as any).mockResolvedValue(mockGitStatus)

      const result = await service.getAvailableCommands('task-123')

      expect(mockTaskRepository.get).toHaveBeenCalledWith('task-123')
      expect(mockCommandService.listCommands).toHaveBeenCalled()
      expect(mockFileGateway.getGitStatus).toHaveBeenCalledWith('task-123')
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
        iterations: [],
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

      const mockGitStatus: GitStatus = {
        currentBranch: 'main',
        baseBranch: 'main',
        hasUncommittedChanges: false,
        hasConflicts: false,
        commitsAhead: 0,
        commitsBehind: 0,
        status: 'synced',
        originExists: true,
      }

      ;(mockTaskRepository.get as any).mockResolvedValue(mockTask)
      ;(mockCommandService.listCommands as any).mockResolvedValue(mockCommands)
      ;(mockFileGateway.getGitStatus as any).mockResolvedValue(mockGitStatus)

      const result = await service.getAvailableCommands('task-123')

      expect(result).toHaveLength(1)
      expect(result[0]?.name).toBe('active-cmd')
    })

    it('should filter commands based on uncommitted changes condition', async () => {
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

      const mockCommands: Command[] = [
        {
          name: 'commit',
          description: 'Commit changes',
          prompt: 'commit the changes',
          conditions: {
            requiresUncommittedChanges: true,
          },
        },
        {
          name: 'other',
          description: 'Other command',
          prompt: 'other prompt',
        },
      ]

      const mockGitStatusWithChanges: GitStatus = {
        currentBranch: 'main',
        baseBranch: 'main',
        hasUncommittedChanges: true,
        hasConflicts: false,
        commitsAhead: 0,
        commitsBehind: 0,
        status: 'uncommitted_changes',
        originExists: true,
      }

      ;(mockTaskRepository.get as any).mockResolvedValue(mockTask)
      ;(mockCommandService.listCommands as any).mockResolvedValue(mockCommands)
      ;(mockFileGateway.getGitStatus as any).mockResolvedValue(mockGitStatusWithChanges)

      const result = await service.getAvailableCommands('task-123')

      expect(result).toHaveLength(2)
      expect(result.map((c) => c.name)).toContain('commit')
      expect(result.map((c) => c.name)).toContain('other')
    })

    it('should exclude commit command when no uncommitted changes', async () => {
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

      const mockCommands: Command[] = [
        {
          name: 'commit',
          description: 'Commit changes',
          prompt: 'commit the changes',
          conditions: {
            requiresUncommittedChanges: true,
          },
        },
        {
          name: 'other',
          description: 'Other command',
          prompt: 'other prompt',
        },
      ]

      const mockGitStatusNoChanges: GitStatus = {
        currentBranch: 'main',
        baseBranch: 'main',
        hasUncommittedChanges: false,
        hasConflicts: false,
        commitsAhead: 0,
        commitsBehind: 0,
        status: 'synced',
        originExists: true,
      }

      ;(mockTaskRepository.get as any).mockResolvedValue(mockTask)
      ;(mockCommandService.listCommands as any).mockResolvedValue(mockCommands)
      ;(mockFileGateway.getGitStatus as any).mockResolvedValue(mockGitStatusNoChanges)

      const result = await service.getAvailableCommands('task-123')

      expect(result).toHaveLength(1)
      expect(result[0]?.name).toBe('other')
    })

    it('should filter commands based on commits ahead condition', async () => {
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

      const mockCommands: Command[] = [
        {
          name: 'create-pr',
          description: 'Create PR',
          prompt: 'create a pull request',
          conditions: {
            requiresAheadOfMain: true,
          },
        },
      ]

      const mockGitStatusAhead: GitStatus = {
        currentBranch: 'feature-branch',
        baseBranch: 'main',
        hasUncommittedChanges: false,
        hasConflicts: false,
        commitsAhead: 2,
        commitsBehind: 0,
        status: 'ahead',
        originExists: true,
      }

      ;(mockTaskRepository.get as any).mockResolvedValue(mockTask)
      ;(mockCommandService.listCommands as any).mockResolvedValue(mockCommands)
      ;(mockFileGateway.getGitStatus as any).mockResolvedValue(mockGitStatusAhead)

      const result = await service.getAvailableCommands('task-123')

      expect(result).toHaveLength(1)
      expect(result[0]?.name).toBe('create-pr')
    })

    it('should filter commands based on commits behind condition', async () => {
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

      const mockCommands: Command[] = [
        {
          name: 'sync-with-main',
          description: 'Sync with main',
          prompt: 'sync with main branch',
          conditions: {
            requiresBehindMain: true,
          },
        },
      ]

      const mockGitStatusBehind: GitStatus = {
        currentBranch: 'feature-branch',
        baseBranch: 'main',
        hasUncommittedChanges: false,
        hasConflicts: false,
        commitsAhead: 0,
        commitsBehind: 3,
        status: 'behind',
        originExists: true,
      }

      ;(mockTaskRepository.get as any).mockResolvedValue(mockTask)
      ;(mockCommandService.listCommands as any).mockResolvedValue(mockCommands)
      ;(mockFileGateway.getGitStatus as any).mockResolvedValue(mockGitStatusBehind)

      const result = await service.getAvailableCommands('task-123')

      expect(result).toHaveLength(1)
      expect(result[0]?.name).toBe('sync-with-main')
    })

    it('should filter commands based on last iteration error condition', async () => {
      const errorIteration: TaskIteration = {
        iterationNumber: 1,
        startTime: new Date(),
        status: 'failed',
        messageCount: 5,
        costUsd: 0.1,
      }

      const mockTask: Task = {
        taskId: 'task-123',
        repositoryId: 'repo-456',
        agentType: 'claude-code',
        status: 'error' as const,
        createdAt: new Date(),
        lastMessageAt: new Date(),
        numTurns: 0,
        totalCostUsd: 0,
        iterations: [errorIteration],
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

      const mockGitStatus: GitStatus = {
        currentBranch: 'main',
        baseBranch: 'main',
        hasUncommittedChanges: false,
        hasConflicts: false,
        commitsAhead: 0,
        commitsBehind: 0,
        status: 'synced',
        originExists: true,
      }

      ;(mockTaskRepository.get as any).mockResolvedValue(mockTask)
      ;(mockCommandService.listCommands as any).mockResolvedValue(mockCommands)
      ;(mockFileGateway.getGitStatus as any).mockResolvedValue(mockGitStatus)

      const result = await service.getAvailableCommands('task-123')

      expect(result).toHaveLength(1)
      expect(result[0]?.name).toBe('continue')
    })

    it('should exclude continue command when last iteration succeeded', async () => {
      const successIteration: TaskIteration = {
        iterationNumber: 1,
        startTime: new Date(),
        status: 'completed',
        messageCount: 5,
        costUsd: 0.1,
      }

      const mockTask: Task = {
        taskId: 'task-123',
        repositoryId: 'repo-456',
        agentType: 'claude-code',
        status: 'open' as const,
        createdAt: new Date(),
        lastMessageAt: new Date(),
        numTurns: 0,
        totalCostUsd: 0,
        iterations: [successIteration],
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

      const mockGitStatus: GitStatus = {
        currentBranch: 'main',
        baseBranch: 'main',
        hasUncommittedChanges: false,
        hasConflicts: false,
        commitsAhead: 0,
        commitsBehind: 0,
        status: 'synced',
        originExists: true,
      }

      ;(mockTaskRepository.get as any).mockResolvedValue(mockTask)
      ;(mockCommandService.listCommands as any).mockResolvedValue(mockCommands)
      ;(mockFileGateway.getGitStatus as any).mockResolvedValue(mockGitStatus)

      const result = await service.getAvailableCommands('task-123')

      expect(result).toHaveLength(0)
    })

    it('should handle multiple conditions correctly', async () => {
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

      const mockCommands: Command[] = [
        {
          name: 'merge-to-main',
          description: 'Merge to main',
          prompt: 'merge to main branch',
          conditions: {
            requiresCommittedChanges: true,
            allowedGitSyncStatuses: ['ahead'],
          },
        },
      ]

      const mockGitStatusAheadWithCommits: GitStatus = {
        currentBranch: 'feature-branch',
        baseBranch: 'main',
        hasUncommittedChanges: false,
        hasConflicts: false,
        commitsAhead: 2,
        commitsBehind: 0,
        status: 'ahead',
        originExists: true,
      }

      ;(mockTaskRepository.get as any).mockResolvedValue(mockTask)
      ;(mockCommandService.listCommands as any).mockResolvedValue(mockCommands)
      ;(mockFileGateway.getGitStatus as any).mockResolvedValue(mockGitStatusAheadWithCommits)

      const result = await service.getAvailableCommands('task-123')

      expect(result).toHaveLength(1)
      expect(result[0]?.name).toBe('merge-to-main')
    })

    it('should build context with task, lastIteration, workspaceHasChanges, and gitStatus', async () => {
      const iteration: TaskIteration = {
        iterationNumber: 1,
        startTime: new Date(),
        status: 'completed',
        messageCount: 3,
        costUsd: 0.05,
      }

      const mockTask: Task = {
        taskId: 'task-123',
        repositoryId: 'repo-456',
        agentType: 'claude-code',
        status: 'open' as const,
        createdAt: new Date(),
        lastMessageAt: new Date(),
        numTurns: 0,
        totalCostUsd: 0,
        iterations: [iteration],
      }

      const mockCommands: Command[] = []

      const mockGitStatus: GitStatus = {
        currentBranch: 'feature-branch',
        baseBranch: 'main',
        hasUncommittedChanges: true,
        hasConflicts: false,
        commitsAhead: 1,
        commitsBehind: 0,
        status: 'uncommitted_changes',
        originExists: true,
      }

      ;(mockTaskRepository.get as any).mockResolvedValue(mockTask)
      ;(mockCommandService.listCommands as any).mockResolvedValue(mockCommands)
      ;(mockFileGateway.getGitStatus as any).mockResolvedValue(mockGitStatus)

      await service.getAvailableCommands('task-123')

      // Verify all context data was gathered
      expect(mockTaskRepository.get).toHaveBeenCalledWith('task-123')
      expect(mockFileGateway.getGitStatus).toHaveBeenCalledWith('task-123')
      expect(mockCommandService.listCommands).toHaveBeenCalled()
    })
  })
})
