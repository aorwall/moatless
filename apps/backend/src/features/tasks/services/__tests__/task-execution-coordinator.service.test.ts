import type { Task, TaskIteration } from '@moatless/types'
import { TaskNotFoundError, ValidationError } from '@moatless/types'
import { Container } from 'inversify'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { AgentRequestService } from '../../../agents/services/agent-request.service'
import { AgentRequestBuilder } from '../../../agents/services/agent-request-builder.service'
import { CommandService } from '../../../agents/services/command.service'
import { MessageService } from '../../../agents/services/message.service'
import { EventService } from '../../../events/index'
import { WorkspaceRuntimeManager } from '../../../workspace/services/workspace-runtime-manager.service'
import { TaskRepository } from '../../db/task.repository'
import { TaskExecutionCoordinator } from '../task-execution-coordinator.service'
import { TaskIterationService } from '../task-iteration.service'

describe('TaskExecutionCoordinator', () => {
  let container: Container
  let service: TaskExecutionCoordinator
  let mockTaskRepository: any
  let mockIterationService: any
  let mockMessageService: any
  let mockRuntimeManager: any
  let mockAgentRequestService: any
  let mockEventService: any
  let mockCommandService: any
  let mockAgentRequestBuilder: any

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
    visibility: 'private' as const,
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
    mockTaskRepository = {
      get: vi.fn(),
      getInternal: vi.fn(),
      save: vi.fn(),
    }

    mockIterationService = {
      createIteration: vi.fn(),
      emitEvents: vi.fn(),
      performPostProcessing: vi.fn(),
      emitErrorEvent: vi.fn(),
      applyError: vi.fn(),
      markIterationAsFailed: vi.fn(),
    }

    mockMessageService = {
      saveUserMessage: vi.fn(),
    }

    mockRuntimeManager = {
      getSnapshot: vi.fn(),
      ensureWorkspace: vi.fn().mockResolvedValue({
        workspacePath: '/test/workspace',
        repositoryId: 'repo-1',
        agent: { state: 'idle' },
        preview: { status: 'stopped' },
      }),
      isWorkspaceAvailable: vi.fn().mockResolvedValue(true),
      isAgentActuallyRunning: vi.fn().mockResolvedValue(false),
      updateAgentState: vi.fn().mockResolvedValue(undefined),
    }

    mockAgentRequestService = {
      publish: vi.fn().mockResolvedValue(undefined),
    }

    mockEventService = {
      emitEvent: vi.fn(),
    }

    mockCommandService = {
      getCommand: vi.fn(),
    }

    mockAgentRequestBuilder = {
      buildRequest: vi.fn().mockResolvedValue({
        content: [{ type: 'text', text: 'Test message' }],
        model: undefined,
        resume: undefined,
        agentMode: undefined,
        background: undefined,
      }),
    }

    // Bind mocks to container
    container.bind(TaskRepository).toConstantValue(mockTaskRepository)
    container.bind(TaskIterationService).toConstantValue(mockIterationService)
    container.bind(MessageService).toConstantValue(mockMessageService)
    container.bind(WorkspaceRuntimeManager).toConstantValue(mockRuntimeManager)
    container.bind(AgentRequestService).toConstantValue(mockAgentRequestService)
    container.bind(EventService).toConstantValue(mockEventService)
    container.bind(CommandService).toConstantValue(mockCommandService)
    container.bind(AgentRequestBuilder).toConstantValue(mockAgentRequestBuilder)
    container.bind(TaskExecutionCoordinator).toSelf()

    service = container.get(TaskExecutionCoordinator)
  })

  afterEach(() => {
    vi.restoreAllMocks()
  })

  describe('sendCommand', () => {
    it('should fetch command and delegate to sendMessage', async () => {
      const commandName = 'format-code'
      const mockCommand = {
        name: commandName,
        prompt: 'Format all code files',
        agentMode: 'edit' as const,
      }

      mockCommandService.getCommand.mockResolvedValue(mockCommand)
      mockTaskRepository.get.mockResolvedValue({ ...mockTask, iterations: [] })
      mockIterationService.createIteration.mockReturnValue(mockIteration)
      mockTaskRepository.save.mockResolvedValue(undefined)

      await service.sendCommand('task-123', commandName)

      expect(mockCommandService.getCommand).toHaveBeenCalledWith(commandName)
      // sendCommand delegates to sendMessage which creates with 'created' status, not 'active'
      expect(mockIterationService.createIteration).toHaveBeenCalledWith(
        expect.any(Object),
        mockCommand.prompt,
        commandName,
        undefined,
        'created',
        'edit',
        undefined,
        undefined,
      )
    })

    it('should throw ValidationError when command not found', async () => {
      mockCommandService.getCommand.mockResolvedValue(null)

      await expect(service.sendCommand('task-123', 'invalid-command')).rejects.toThrow(
        ValidationError,
      )
    })
  })

  describe('sendMessage', () => {
    beforeEach(() => {
      mockTaskRepository.get.mockResolvedValue({ ...mockTask, iterations: [] })
      mockIterationService.createIteration.mockReturnValue(mockIteration)
      mockTaskRepository.save.mockResolvedValue(undefined)
    })

    it('should create iteration and publish to queue', async () => {
      await service.sendMessage('task-123', 'Test message')

      expect(mockTaskRepository.get).toHaveBeenCalledWith('task-123')
      expect(mockIterationService.createIteration).toHaveBeenCalled()
      expect(mockTaskRepository.save).toHaveBeenCalled()
      // Agent request is published to queue
      expect(mockAgentRequestService.publish).toHaveBeenCalledWith(
        'task-123',
        expect.objectContaining({
          content: expect.any(Array),
        }),
      )
    })

    it('should mark last iteration as willNotResume when resume is false', async () => {
      const taskWithIteration = {
        ...mockTask,
        iterations: [{ ...mockIteration, willNotResume: false }],
      }
      mockTaskRepository.get.mockResolvedValue(taskWithIteration)

      await service.sendMessage('task-123', 'Test message', false)

      const savedTask = mockTaskRepository.save.mock.calls[0][0]
      expect(savedTask.iterations[0].willNotResume).toBe(true)
    })

    it('should use resumeSessionId from previous iteration', async () => {
      const previousIteration = { ...mockIteration, agentSessionId: 'session-123' }
      const taskWithIteration = {
        ...mockTask,
        iterations: [previousIteration],
      }
      mockTaskRepository.get.mockResolvedValue(taskWithIteration)

      await service.sendMessage('task-123', 'Test message', true)

      // Verify builder is called with correct resumeSessionId in options
      expect(mockAgentRequestBuilder.buildRequest).toHaveBeenCalledWith(
        'task-123',
        'Test message',
        expect.any(Object),
        'repo-1',
        expect.objectContaining({
          resumeSessionId: 'session-123',
        }),
      )
    })

    it('should prepend result when previous iteration has willNotResume', async () => {
      const previousIteration = {
        ...mockIteration,
        willNotResume: true,
        result: 'Previous result',
      }
      const taskWithIteration = {
        ...mockTask,
        iterations: [previousIteration],
      }
      mockTaskRepository.get.mockResolvedValue(taskWithIteration)

      await service.sendMessage('task-123', 'Test message', true)

      // Check that the builder was called with message prepended with previous result
      expect(mockAgentRequestBuilder.buildRequest).toHaveBeenCalledWith(
        'task-123',
        expect.stringContaining('Previous result'),
        expect.any(Object),
        'repo-1',
        expect.objectContaining({
          resumeSessionId: undefined, // resume should be undefined when willNotResume
        }),
      )
    })

    it('should handle command with willNotResume flag', async () => {
      const commandWithFlag = {
        name: 'test-command',
        prompt: 'Test',
        willNotResume: true,
      }
      mockCommandService.getCommand.mockResolvedValue(commandWithFlag)
      mockTaskRepository.get.mockResolvedValue({ ...mockTask, iterations: [] })

      await service.sendMessage(
        'task-123',
        'Test message',
        true,
        undefined,
        undefined,
        'test-command',
      )

      const savedTask = mockTaskRepository.save.mock.calls[0][0]
      const newIteration = savedTask.iterations[0]
      expect(newIteration.willNotResume).toBe(true)
    })

    it('should pass contexts to agent request', async () => {
      const contexts = [
        {
          type: 'element' as const,
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

      await service.sendMessage(
        'task-123',
        'Test message',
        true,
        undefined,
        undefined,
        undefined,
        undefined,
        contexts,
      )

      // Verify builder was called with contexts in options
      expect(mockAgentRequestBuilder.buildRequest).toHaveBeenCalledWith(
        'task-123',
        'Test message',
        expect.any(Object),
        'repo-1',
        expect.objectContaining({
          contexts,
        }),
      )
      expect(mockAgentRequestService.publish).toHaveBeenCalled()
    })
  })

  describe('startTask', () => {
    it('should activate created iteration and publish to queue', async () => {
      const createdIteration = { ...mockIteration, status: 'created' as const }
      const taskWithCreatedIteration = {
        ...mockTask,
        iterations: [createdIteration],
      }

      mockTaskRepository.get.mockResolvedValue(taskWithCreatedIteration)
      mockRuntimeManager.getSnapshot.mockResolvedValue({ id: 'snapshot-1' })

      await service.startTask('task-123')

      expect(mockTaskRepository.get).toHaveBeenCalledWith('task-123')
      expect(mockRuntimeManager.getSnapshot).toHaveBeenCalled()

      const savedTask = mockTaskRepository.save.mock.calls[0][0]
      expect(savedTask.iterations[0].status).toBe('active')

      expect(mockEventService.emitEvent).toHaveBeenCalledWith(
        expect.objectContaining({
          type: 'taskIteration',
          subType: 'active',
        }),
      )

      expect(mockAgentRequestService.publish).toHaveBeenCalled()
    })

    it('should throw TaskNotFoundError when task does not exist', async () => {
      mockTaskRepository.get.mockResolvedValue(null)

      await expect(service.startTask('task-123')).rejects.toThrow(TaskNotFoundError)
    })

    it('should throw ValidationError when last iteration is not created', async () => {
      const taskWithActiveIteration = {
        ...mockTask,
        iterations: [{ ...mockIteration, status: 'active' as const }],
      }

      mockTaskRepository.get.mockResolvedValue(taskWithActiveIteration)

      await expect(service.startTask('task-123')).rejects.toThrow(ValidationError)
    })

    it('should wait for workspace to be ready', async () => {
      const createdIteration = { ...mockIteration, status: 'created' as const }
      const taskWithCreatedIteration = {
        ...mockTask,
        iterations: [createdIteration],
      }

      mockTaskRepository.get.mockResolvedValue(taskWithCreatedIteration)

      // First two calls return null (not ready), third call returns snapshot (ready)
      // Fourth call is inside buildAgentRequest to get preview URL
      mockRuntimeManager.getSnapshot
        .mockResolvedValueOnce(null)
        .mockResolvedValueOnce(null)
        .mockResolvedValueOnce({ id: 'snapshot-1' })
        .mockResolvedValueOnce({ id: 'snapshot-1', preview: {} })

      await service.startTask('task-123')

      // 3 calls during polling + 1 call in buildAgentRequest for preview URL
      expect(mockRuntimeManager.getSnapshot).toHaveBeenCalledTimes(4)
      expect(mockAgentRequestService.publish).toHaveBeenCalled()
    })
  })

  describe('resumeIteration', () => {
    it('should resume with SDK session when agentSessionId exists', async () => {
      const activeIteration = {
        ...mockIteration,
        status: 'active' as const,
        agentSessionId: 'session-456',
      }
      mockTaskRepository.getInternal.mockResolvedValue({
        ...mockTask,
        iterations: [activeIteration],
      })

      await service.resumeIteration('task-123', activeIteration)

      // Verify builder is called with resumeSessionId in options
      expect(mockAgentRequestBuilder.buildRequest).toHaveBeenCalledWith(
        'task-123',
        '', // empty message when resuming with session
        activeIteration,
        'repo-1',
        expect.objectContaining({
          resumeSessionId: 'session-456',
        }),
      )
      expect(mockAgentRequestService.publish).toHaveBeenCalled()
    })

    it('should start fresh when no agentSessionId exists', async () => {
      const activeIteration = {
        ...mockIteration,
        status: 'active' as const,
        initialMessage: 'Initial message',
        agentSessionId: undefined,
      }
      mockTaskRepository.getInternal.mockResolvedValue({
        ...mockTask,
        iterations: [activeIteration],
      })

      await service.resumeIteration('task-123', activeIteration)

      // Verify builder is called with initialMessage and no resumeSessionId
      expect(mockAgentRequestBuilder.buildRequest).toHaveBeenCalledWith(
        'task-123',
        'Initial message',
        activeIteration,
        'repo-1',
        expect.objectContaining({
          resumeSessionId: undefined,
        }),
      )
      expect(mockAgentRequestService.publish).toHaveBeenCalled()
    })

    it('should throw ValidationError when iteration is not active', async () => {
      const completedIteration = { ...mockIteration, status: 'completed' as const }

      await expect(service.resumeIteration('task-123', completedIteration)).rejects.toThrow(
        ValidationError,
      )
    })
  })
})
