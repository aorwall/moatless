import type { Event, Task, TaskIteration } from '@moatless/types'
import { Container } from 'inversify'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { EventService } from '../../../events/index'
import { TaskMetadataStorage } from '../../storage/task-metadata.storage'
import { TaskEventProjector } from '../task-event-projector.service'
import { TaskIterationService } from '../task-iteration.service'

describe('TaskEventProjector', () => {
  let container: Container
  let service: TaskEventProjector
  let mockEventService: any
  let mockTaskStorage: any
  let mockIterationService: any
  let eventHandler: ((event: Event) => void) | null = null

  const mockTask: Task = {
    taskId: 'task-123',
    name: 'Test Task',
    repositoryId: 'repo-1',
    status: 'running',
    agentType: 'claude-code',
    iterations: [],
    createdAt: new Date(),
    lastMessageAt: new Date(),
    numTurns: 0,
    totalCostUsd: 0,
  }

  const mockIteration: TaskIteration = {
    iterationNumber: 1,
    startTime: new Date(),
    status: 'active',
    messageCount: 1,
    costUsd: 0,
    initialMessage: 'Test message',
  }

  beforeEach(() => {
    container = new Container()

    // Create mocks with all required methods
    mockEventService = {
      subscribeAll: vi.fn((handler) => {
        eventHandler = handler
      }),
    }

    mockTaskStorage = {
      get: vi.fn(),
      save: vi.fn(),
    }

    mockIterationService = {
      applyResult: vi.fn(),
      applyError: vi.fn(),
      updateContextTokens: vi.fn(),
      performPostProcessing: vi.fn().mockResolvedValue(undefined),
      emitEvents: vi.fn().mockResolvedValue(undefined),
      emitErrorEvent: vi.fn().mockResolvedValue(undefined),
      captureGitMetadata: vi.fn().mockResolvedValue(undefined),
    }

    // Bind mocks to container
    container.bind(EventService).toConstantValue(mockEventService)
    container.bind(TaskMetadataStorage).toConstantValue(mockTaskStorage)
    container.bind(TaskIterationService).toConstantValue(mockIterationService)
    container.bind(TaskEventProjector).toSelf()

    service = container.get(TaskEventProjector)
  })

  afterEach(() => {
    vi.restoreAllMocks()
    eventHandler = null
  })

  describe('subscribe', () => {
    it('should subscribe to event service', () => {
      service.subscribe()

      expect(mockEventService.subscribeAll).toHaveBeenCalled()
    })

    it('should capture event handler', () => {
      service.subscribe()

      expect(eventHandler).not.toBeNull()
      expect(typeof eventHandler).toBe('function')
    })
  })

  describe('event filtering', () => {
    beforeEach(() => {
      service.subscribe()
    })

    it('should handle agent.completed events', async () => {
      const taskWithIteration = {
        ...mockTask,
        iterations: [mockIteration],
      }
      mockTaskStorage.get.mockResolvedValue(taskWithIteration)

      const event: Event = {
        eventId: 'event-1',
        taskId: 'task-123',
        type: 'agent',
        subType: 'completed',
        agentType: 'claude-code',
        timestamp: new Date(),
        isError: false,
        payload: {
          type: 'result',
          subtype: 'success',
          result: 'Done',
          total_cost_usd: 0.05,
          is_error: false,
        },
      }

      await eventHandler?.(event)

      expect(mockIterationService.applyResult).toHaveBeenCalled()
      expect(mockIterationService.performPostProcessing).toHaveBeenCalled()
      expect(mockTaskStorage.save).toHaveBeenCalled()
    })

    it('should handle agent.error events', async () => {
      const taskWithIteration = {
        ...mockTask,
        iterations: [mockIteration],
      }
      mockTaskStorage.get.mockResolvedValue(taskWithIteration)

      const event: Event = {
        eventId: 'event-1',
        taskId: 'task-123',
        type: 'agent',
        subType: 'error',
        agentType: 'claude-code',
        timestamp: new Date(),
        isError: true,
        payload: {
          error: 'Something went wrong',
          stackTrace: 'Error stack',
        },
      }

      await eventHandler?.(event)

      expect(mockIterationService.applyError).toHaveBeenCalled()
      expect(mockIterationService.captureGitMetadata).toHaveBeenCalled()
      expect(mockTaskStorage.save).toHaveBeenCalled()
    })

    it('should handle agent.message events with context tokens', async () => {
      const taskWithIteration = {
        ...mockTask,
        iterations: [mockIteration],
      }
      mockTaskStorage.get.mockResolvedValue(taskWithIteration)

      const event: Event = {
        eventId: 'event-1',
        taskId: 'task-123',
        type: 'agent',
        subType: 'message',
        agentType: 'claude-code',
        timestamp: new Date(),
        isError: false,
        payload: {
          contextTokens: 5000,
        },
      }

      await eventHandler?.(event)

      expect(mockIterationService.updateContextTokens).toHaveBeenCalledWith(mockIteration, 5000)
      expect(mockTaskStorage.save).toHaveBeenCalled()
    })

    it('should ignore events without context tokens', async () => {
      const taskWithIteration = {
        ...mockTask,
        iterations: [mockIteration],
      }
      mockTaskStorage.get.mockResolvedValue(taskWithIteration)

      const event: Event = {
        eventId: 'event-1',
        taskId: 'task-123',
        type: 'agent',
        subType: 'message',
        agentType: 'claude-code',
        timestamp: new Date(),
        isError: false,
        payload: {
          someOtherData: 'value',
        },
      }

      await eventHandler?.(event)

      // Should not handle event if no contextTokens or agentSessionId
      expect(mockIterationService.updateContextTokens).not.toHaveBeenCalled()
      expect(mockTaskStorage.save).not.toHaveBeenCalled()
    })

    it('should extract and store agent session ID', async () => {
      const taskWithIteration = {
        ...mockTask,
        iterations: [{ ...mockIteration, agentSessionId: undefined }],
      }
      mockTaskStorage.get.mockResolvedValue(taskWithIteration)

      const event: Event = {
        eventId: 'event-1',
        taskId: 'task-123',
        type: 'agent',
        subType: 'message',
        agentType: 'claude-code',
        timestamp: new Date(),
        isError: false,
        payload: {
          agentSessionId: 'session-abc-123',
        },
      }

      await eventHandler?.(event)

      expect(taskWithIteration.iterations[0]?.agentSessionId).toBe('session-abc-123')
      expect(mockTaskStorage.save).toHaveBeenCalled()
    })

    it('should not overwrite existing agent session ID', async () => {
      const taskWithIteration = {
        ...mockTask,
        iterations: [{ ...mockIteration, agentSessionId: 'existing-session' }],
      }
      mockTaskStorage.get.mockResolvedValue(taskWithIteration)

      const event: Event = {
        eventId: 'event-1',
        taskId: 'task-123',
        type: 'agent',
        subType: 'message',
        agentType: 'claude-code',
        timestamp: new Date(),
        isError: false,
        payload: {
          agentSessionId: 'new-session',
        },
      }

      await eventHandler?.(event)

      // Should not change existing session ID
      expect(taskWithIteration.iterations[0]?.agentSessionId).toBe('existing-session')
    })

    it('should ignore non-agent events', async () => {
      const event: Event = {
        eventId: 'event-1',
        taskId: 'task-123',
        type: 'task',
        subType: 'open',
        agentType: 'claude-code',
        timestamp: new Date(),
        isError: false,
        payload: {
          status: 'open',
          previousStatus: 'closed',
        },
      }

      await eventHandler?.(event)

      expect(mockTaskStorage.get).not.toHaveBeenCalled()
    })
  })

  describe('iteration matching', () => {
    beforeEach(() => {
      service.subscribe()
    })

    it('should find iteration by iterationId', async () => {
      const iteration1 = { ...mockIteration, iterationNumber: 1 }
      const iteration2 = { ...mockIteration, iterationNumber: 2 }
      const taskWithIterations = {
        ...mockTask,
        iterations: [iteration1, iteration2],
      }
      mockTaskStorage.get.mockResolvedValue(taskWithIterations)

      const event: Event = {
        eventId: 'event-1',
        taskId: 'task-123',
        type: 'agent',
        subType: 'completed',
        agentType: 'claude-code',
        iterationId: 1,
        timestamp: new Date(),
        isError: false,
        payload: {
          type: 'result',
          subtype: 'success',
          result: 'Done',
          total_cost_usd: 0.05,
          is_error: false,
        },
      }

      await eventHandler?.(event)

      // Should apply result to iteration 1, not iteration 2
      expect(mockIterationService.applyResult).toHaveBeenCalledWith(
        iteration1,
        expect.any(Object),
        expect.any(Object),
      )
    })

    it('should use last iteration when no iterationId provided', async () => {
      const iteration1 = { ...mockIteration, iterationNumber: 1 }
      const iteration2 = { ...mockIteration, iterationNumber: 2 }
      const taskWithIterations = {
        ...mockTask,
        iterations: [iteration1, iteration2],
      }
      mockTaskStorage.get.mockResolvedValue(taskWithIterations)

      const event: Event = {
        eventId: 'event-1',
        taskId: 'task-123',
        type: 'agent',
        subType: 'completed',
        agentType: 'claude-code',
        timestamp: new Date(),
        isError: false,
        payload: {
          type: 'result',
          subtype: 'success',
          result: 'Done',
          total_cost_usd: 0.05,
          is_error: false,
        },
      }

      await eventHandler?.(event)

      // Should apply result to last iteration (iteration 2)
      expect(mockIterationService.applyResult).toHaveBeenCalledWith(
        iteration2,
        expect.any(Object),
        expect.any(Object),
      )
    })
  })

  describe('error handling', () => {
    beforeEach(() => {
      service.subscribe()
    })

    it('should skip events when task not found', async () => {
      mockTaskStorage.get.mockResolvedValue(null)

      const event: Event = {
        eventId: 'event-1',
        taskId: 'task-999',
        type: 'agent',
        subType: 'completed',
        agentType: 'claude-code',
        timestamp: new Date(),
        isError: false,
        payload: {},
      }

      await eventHandler?.(event)

      expect(mockIterationService.applyResult).not.toHaveBeenCalled()
      expect(mockTaskStorage.save).not.toHaveBeenCalled()
    })

    it('should skip events when iteration not found', async () => {
      const taskWithoutIterations = {
        ...mockTask,
        iterations: [],
      }
      mockTaskStorage.get.mockResolvedValue(taskWithoutIterations)

      const event: Event = {
        eventId: 'event-1',
        taskId: 'task-123',
        type: 'agent',
        subType: 'completed',
        agentType: 'claude-code',
        timestamp: new Date(),
        isError: false,
        payload: {},
      }

      await eventHandler?.(event)

      expect(mockIterationService.applyResult).not.toHaveBeenCalled()
      expect(mockTaskStorage.save).not.toHaveBeenCalled()
    })

    it('should catch and log errors during event handling', async () => {
      mockTaskStorage.get.mockRejectedValue(new Error('Storage error'))

      const event: Event = {
        eventId: 'event-1',
        taskId: 'task-123',
        type: 'agent',
        subType: 'completed',
        agentType: 'claude-code',
        timestamp: new Date(),
        isError: false,
        payload: {},
      }

      // Should not throw - errors are caught and logged
      await eventHandler?.(event)

      // Error should be logged but not propagate
      expect(mockTaskStorage.get).toHaveBeenCalled()
    })
  })

  describe('result event handling', () => {
    beforeEach(() => {
      service.subscribe()
    })

    it('should update task status to error when iteration fails', async () => {
      const taskWithIteration = {
        ...mockTask,
        status: 'running',
        iterations: [mockIteration],
      }
      mockTaskStorage.get.mockResolvedValue(taskWithIteration)

      // Mock iteration service to mark iteration as failed
      mockIterationService.applyResult.mockImplementation((iteration: TaskIteration) => {
        iteration.status = 'failed'
      })

      const event: Event = {
        eventId: 'event-1',
        taskId: 'task-123',
        type: 'agent',
        subType: 'completed',
        agentType: 'claude-code',
        timestamp: new Date(),
        isError: true,
        payload: {
          type: 'result',
          subtype: 'error_max_turns',
          total_cost_usd: 0.1,
          is_error: true,
        },
      }

      await eventHandler?.(event)

      const savedTask = mockTaskStorage.save.mock.calls[0][0]
      expect(savedTask.status).toBe('error')
    })

    it('should update task status to open when iteration succeeds', async () => {
      const taskWithIteration = {
        ...mockTask,
        status: 'running',
        iterations: [mockIteration],
      }
      mockTaskStorage.get.mockResolvedValue(taskWithIteration)

      // Mock iteration service to mark iteration as completed
      mockIterationService.applyResult.mockImplementation((iteration: TaskIteration) => {
        iteration.status = 'completed'
      })

      const event: Event = {
        eventId: 'event-1',
        taskId: 'task-123',
        type: 'agent',
        subType: 'completed',
        agentType: 'claude-code',
        timestamp: new Date(),
        isError: false,
        payload: {
          type: 'result',
          subtype: 'success',
          result: 'Task completed',
          total_cost_usd: 0.05,
          is_error: false,
        },
      }

      await eventHandler?.(event)

      const savedTask = mockTaskStorage.save.mock.calls[0][0]
      expect(savedTask.status).toBe('open')
    })

    it('should emit events after handling result', async () => {
      const taskWithIteration = {
        ...mockTask,
        status: 'running',
        iterations: [mockIteration],
      }
      mockTaskStorage.get.mockResolvedValue(taskWithIteration)

      const event: Event = {
        eventId: 'event-1',
        taskId: 'task-123',
        type: 'agent',
        subType: 'completed',
        agentType: 'claude-code',
        timestamp: new Date(),
        isError: false,
        payload: {
          type: 'result',
          subtype: 'success',
          result: 'Done',
          total_cost_usd: 0.05,
          is_error: false,
        },
      }

      await eventHandler?.(event)

      expect(mockIterationService.emitEvents).toHaveBeenCalled()
    })
  })
})
