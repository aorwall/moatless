import type { Event, MessageContext, SDKResultMessage, Task, TaskIteration } from '@moatless/types'
import { ValidationError } from '@moatless/types'
import { Container } from 'inversify'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { EventService } from '../../../events/index'
import { PlaywrightScreenshotService } from '../../../preview/services/playwright-screenshot.service'
import { RepositoryService } from '../../../repositories/index'
import { WorkspaceCoordinator } from '../../../workspace/services/workspace-coordinator.service'
import { WorkspaceRuntimeManager } from '../../../workspace/services/workspace-runtime-manager.service'
import { TaskRepository } from '../../db/task.repository'
import { TaskIterationService } from '../task-iteration.service'

describe('TaskIterationService', () => {
  let container: Container
  let service: TaskIterationService
  let mockRuntimeManager: any
  let mockDriver: any
  let mockScreenshotService: any
  let mockEventService: any
  let mockRepositoryService: any
  let mockTaskRepository: any
  let mockWorkspaceCoordinator: any

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
    gitCheckpointEnabled: false,
    gitPushEnabled: false,
    visibility: 'private' as const,
  }

  beforeEach(() => {
    container = new Container()

    mockDriver = {
      getGitStatus: vi.fn(),
      getGitChanges: vi.fn(),
      getCurrentCommitHash: vi.fn(),
    }

    mockRuntimeManager = {
      ensureWorkspace: vi
        .fn()
        .mockResolvedValue({ driver: mockDriver, workspacePath: '/workspace' }),
      getWorkspacePath: vi.fn().mockResolvedValue('/workspace'),
    }

    mockScreenshotService = {
      captureIterationScreenshotIfEnabled: vi.fn(),
    }

    mockEventService = {
      emitEvent: vi.fn(),
    }

    mockRepositoryService = {
      getRepositoryById: vi.fn(),
    }

    mockTaskRepository = {
      get: vi.fn(),
      save: vi.fn(),
    }

    mockWorkspaceCoordinator = {
      deleteWorkspace: vi.fn().mockResolvedValue(undefined),
    }

    container.bind(WorkspaceCoordinator).toConstantValue(mockWorkspaceCoordinator)
    container.bind(WorkspaceRuntimeManager).toConstantValue(mockRuntimeManager)
    container.bind(PlaywrightScreenshotService).toConstantValue(mockScreenshotService)
    container.bind(EventService).toConstantValue(mockEventService)
    container.bind(RepositoryService).toConstantValue(mockRepositoryService)
    container.bind(TaskRepository).toConstantValue(mockTaskRepository)
    container.bind(TaskIterationService).toSelf()

    service = container.get(TaskIterationService)
  })

  afterEach(() => {
    vi.clearAllMocks()
  })

  describe('createIteration', () => {
    it('should create iteration with message', () => {
      const iteration = service.createIteration(mockTask, 'Test message')

      expect(iteration).toMatchObject({
        iterationNumber: 1,
        status: 'active',
        messageCount: 1,
        costUsd: 0,
        initialMessage: 'Test message',
      })
      expect(iteration.startTime).toBeInstanceOf(Date)
    })

    it('should create iteration with command', () => {
      const iteration = service.createIteration(mockTask, undefined, 'format-code')

      expect(iteration).toMatchObject({
        iterationNumber: 1,
        commandName: 'format-code',
      })
    })

    it('should create iteration with contexts', () => {
      const contexts: MessageContext[] = [
        {
          type: 'element',
          referenceId: 'path:10:5',
          data: {
            componentId: 'path:10:5',
            path: '/path/to/file.tsx',
            line: 10,
            column: 5,
            componentName: 'Button',
          },
        },
      ]
      const iteration = service.createIteration(
        mockTask,
        undefined,
        undefined,
        undefined,
        'active',
        undefined,
        contexts,
      )

      expect(iteration.iterationNumber).toBe(1)
    })

    it('should throw ValidationError when no message, command, or context', () => {
      expect(() => service.createIteration(mockTask)).toThrow(ValidationError)
    })

    it('should increment iteration number', () => {
      const taskWithIteration = {
        ...mockTask,
        iterations: [{ iterationNumber: 1 } as TaskIteration],
      }
      const iteration = service.createIteration(taskWithIteration, 'Message')

      expect(iteration.iterationNumber).toBe(2)
    })
  })

  describe('applyResult', () => {
    let iteration: TaskIteration
    let event: Event

    beforeEach(() => {
      iteration = service.createIteration(mockTask, 'Test')
      event = {
        eventId: 'event-1',
        taskId: 'task-123',
        type: 'agent',
        subType: 'completed',
        agentType: 'claude-code',
        timestamp: new Date(),
        isError: false,
        payload: {},
      }
    })

    it('should apply successful result', () => {
      const resultPayload = {
        type: 'result',
        subtype: 'success',
        result: 'Task completed successfully',
        total_cost_usd: 0.05,
        is_error: false,
      } as SDKResultMessage

      service.applyResult(iteration, event, resultPayload)

      expect(iteration.status).toBe('completed')
      expect(iteration.result).toBe('Task completed successfully')
      expect(iteration.costUsd).toBe(0.05)
      expect(iteration.endTime).toBeInstanceOf(Date)
    })

    it('should mark as failed for empty success result', () => {
      const resultPayload = {
        type: 'result',
        subtype: 'success',
        result: '',
        total_cost_usd: 0.05,
        is_error: false,
      } as SDKResultMessage

      service.applyResult(iteration, event, resultPayload)

      expect(iteration.status).toBe('failed')
      expect(iteration.result).toContain('empty result')
    })

    it('should apply error result for max turns', () => {
      const resultPayload = {
        type: 'result',
        subtype: 'error_max_turns',
        total_cost_usd: 0.1,
        is_error: true,
      } as SDKResultMessage
      event.isError = true

      service.applyResult(iteration, event, resultPayload)

      expect(iteration.status).toBe('failed')
      expect(iteration.result).toBe('Maximum number of turns exceeded')
      expect(iteration.willNotResume).toBe(true)
    })

    it('should handle unexpected result subtype', () => {
      const resultPayload = {
        type: 'result',
        subtype: 'unknown_type',
        total_cost_usd: 0.05,
        is_error: false,
      } as any

      service.applyResult(iteration, event, resultPayload)

      expect(iteration.status).toBe('failed')
      expect(iteration.result).toContain('Unexpected result subtype')
    })
  })

  describe('applyError', () => {
    it('should apply error with stack trace', () => {
      const iteration = service.createIteration(mockTask, 'Test')
      const errorPayload = {
        error: 'Something went wrong',
        stackTrace: 'Error at line 42',
      }

      service.applyError(iteration, errorPayload)

      expect(iteration.status).toBe('failed')
      expect(iteration.result).toBe('Something went wrong\nStack: Error at line 42')
      expect(iteration.endTime).toBeInstanceOf(Date)
    })

    it('should apply error without stack trace', () => {
      const iteration = service.createIteration(mockTask, 'Test')
      const errorPayload = {
        error: 'Something went wrong',
      }

      service.applyError(iteration, errorPayload)

      expect(iteration.result).toBe('Something went wrong')
    })
  })

  describe('updateContextTokens', () => {
    it('should update context tokens', () => {
      const iteration = service.createIteration(mockTask, 'Test')

      service.updateContextTokens(iteration, 1500)

      expect(iteration.contextTokens).toBe(1500)
    })
  })

  describe('captureGitMetadata', () => {
    it('should capture git status, changes, and commit hash', async () => {
      const iteration = service.createIteration(mockTask, 'Test')

      mockDriver.getGitStatus.mockResolvedValue({
        status: 'synced',
        commitsAhead: 0,
        commitsBehind: 0,
        hasUncommittedChanges: false,
      })
      mockDriver.getGitChanges.mockResolvedValue({
        currentBranch: 'main',
        status: 'clean',
        additions: 0,
        deletions: 0,
        modifications: 0,
      })
      mockDriver.getCurrentCommitHash.mockResolvedValue('abc123')

      await service.captureGitMetadata('task-123', iteration)

      expect(mockRuntimeManager.ensureWorkspace).toHaveBeenCalledWith('task-123')
      expect(iteration.gitStatus?.status).toBe('synced')
      expect(iteration.gitChanges?.currentBranch).toBe('main')
      expect(iteration.commitHash).toBe('abc123')
    })

    it('should handle git capture errors gracefully', async () => {
      const iteration = service.createIteration(mockTask, 'Test')

      mockRuntimeManager.ensureWorkspace.mockRejectedValue(new Error('Workspace not available'))

      await service.captureGitMetadata('task-123', iteration)

      // Should not throw, just log warning
      expect(iteration.gitStatus).toBeUndefined()
    })
  })

  describe('performPostProcessing', () => {
    it('should orchestrate all post-processing steps', async () => {
      const taskWithSettings = {
        ...mockTask,
        gitCheckpointEnabled: true,
        gitPushEnabled: true,
      }
      const iteration = service.createIteration(taskWithSettings, 'Test')

      mockDriver.getGitStatus.mockResolvedValue({})
      mockDriver.getGitChanges.mockResolvedValue({})
      mockDriver.getCurrentCommitHash.mockResolvedValue('abc123')
      mockRepositoryService.getRepositoryById.mockResolvedValue({
        id: 'repo-456',
        remoteUrl: null,
      })
      mockScreenshotService.captureIterationScreenshotIfEnabled.mockResolvedValue(undefined)

      await service.performPostProcessing(taskWithSettings, iteration)

      expect(mockRuntimeManager.ensureWorkspace).toHaveBeenCalled()
      expect(mockScreenshotService.captureIterationScreenshotIfEnabled).toHaveBeenCalled()
    })
  })

  describe('emitEvents', () => {
    it('should emit completed iteration event', async () => {
      const iteration = service.createIteration(mockTask, 'Test')
      iteration.status = 'completed'
      iteration.result = 'Done'

      await service.emitEvents(mockTask, iteration, 'open')

      expect(mockEventService.emitEvent).toHaveBeenCalledWith(
        expect.objectContaining({
          type: 'taskIteration',
          subType: 'completed',
          payload: expect.objectContaining({
            status: 'completed',
            result: 'Done',
          }),
        }),
      )
    })

    it('should emit task status change event', async () => {
      const iteration = service.createIteration(mockTask, 'Test')
      const updatedTask = { ...mockTask, status: 'error' as const }

      await service.emitEvents(updatedTask, iteration, 'open')

      expect(mockEventService.emitEvent).toHaveBeenCalledWith(
        expect.objectContaining({
          type: 'task',
          subType: 'error',
          payload: expect.objectContaining({
            status: 'error',
            previousStatus: 'open',
          }),
        }),
      )
    })

    it('should not emit events when no changes', async () => {
      const iteration = service.createIteration(mockTask, 'Test')

      await service.emitEvents(mockTask, iteration, 'open')

      expect(mockEventService.emitEvent).not.toHaveBeenCalled()
    })
  })

  describe('emitErrorEvent', () => {
    it('should emit iteration error event', async () => {
      const iteration = service.createIteration(mockTask, 'Test')
      iteration.status = 'failed'

      await service.emitErrorEvent(mockTask, iteration)

      expect(mockEventService.emitEvent).toHaveBeenCalledWith(
        expect.objectContaining({
          type: 'taskIteration',
          subType: 'error',
          isError: true,
        }),
      )
    })
  })
})
