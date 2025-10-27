import type { Task, TaskIteration } from '@moatless/types'
import { TaskNotFoundError, ValidationError } from '@moatless/types'
import { Container } from 'inversify'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { CommandService } from '../../../../services/command.service'
import { EventService } from '../../../events/index'
import { RepositoryService } from '../../../repositories/index'
import { FileGateway, WorkspaceCoordinator } from '../../../workspace/index'
import { TaskMetadataStorage } from '../../storage/task-metadata.storage'
import { MessageService } from '../message.service'
import { TaskExecutionCoordinator } from '../task-execution-coordinator.service'
import { TaskIterationService } from '../task-iteration.service'

describe('TaskExecutionCoordinator', () => {
  let container: Container
  let service: TaskExecutionCoordinator
  let mockTaskStorage: any
  let mockIterationService: any
  let mockMessageService: any
  let mockWorkspaceCoordinator: any
  let mockFileGateway: any
  let mockAgentService: any
  let mockEventService: any
  let mockRepositoryService: any
  let mockCommandService: any

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
    mockTaskStorage = {
      get: vi.fn(),
      save: vi.fn(),
    }

    mockIterationService = {
      createIteration: vi.fn(),
      emitEvents: vi.fn(),
      performPostProcessing: vi.fn(),
      emitErrorEvent: vi.fn(),
      applyError: vi.fn(),
    }

    mockMessageService = {
      saveUserMessage: vi.fn(),
    }

    mockWorkspaceCoordinator = {
      getSnapshot: vi.fn(),
      ensureWorkspace: vi.fn().mockResolvedValue(undefined),
    }

    mockFileGateway = {
      getWorkspacePath: vi.fn().mockResolvedValue('/workspace/path'),
      isWorkspacePathValid: vi.fn().mockResolvedValue(true),
    }

    mockAgentService = {
      executeAgent: vi.fn(),
    }

    mockEventService = {
      emitEvent: vi.fn(),
    }

    mockRepositoryService = {
      getRepositoryById: vi.fn().mockResolvedValue({
        id: 'repo-1',
        name: 'Test Repo',
        systemPrompt: 'Test prompt',
      }),
    }

    mockCommandService = {
      getCommand: vi.fn(),
    }

    // Bind mocks to container
    container.bind(TaskMetadataStorage).toConstantValue(mockTaskStorage)
    container.bind(TaskIterationService).toConstantValue(mockIterationService)
    container.bind(MessageService).toConstantValue(mockMessageService)
    container.bind(WorkspaceCoordinator).toConstantValue(mockWorkspaceCoordinator)
    container.bind(FileGateway).toConstantValue(mockFileGateway)
    container.bind(EventService).toConstantValue(mockEventService)
    container.bind(RepositoryService).toConstantValue(mockRepositoryService)
    container.bind(CommandService).toConstantValue(mockCommandService)
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
      mockTaskStorage.get.mockResolvedValue({ ...mockTask, iterations: [] })
      mockIterationService.createIteration.mockReturnValue(mockIteration)
      mockTaskStorage.save.mockResolvedValue(undefined)

      // Mock async generator for agent events
      const mockEventGenerator = (async function* () {
        yield { type: 'start' }
      })()
      mockAgentService.executeAgent.mockReturnValue(mockEventGenerator)

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
      mockTaskStorage.get.mockResolvedValue({ ...mockTask, iterations: [] })
      mockIterationService.createIteration.mockReturnValue(mockIteration)
      mockTaskStorage.save.mockResolvedValue(undefined)

      // Mock async generator for agent events
      const mockEventGenerator = (async function* () {
        yield { type: 'start' }
      })()
      mockAgentService.executeAgent.mockReturnValue(mockEventGenerator)
    })

    it('should create iteration and execute agent', async () => {
      await service.sendMessage('task-123', 'Test message')

      expect(mockTaskStorage.get).toHaveBeenCalledWith('task-123')
      expect(mockIterationService.createIteration).toHaveBeenCalled()
      expect(mockTaskStorage.save).toHaveBeenCalled()
      expect(mockMessageService.saveUserMessage).toHaveBeenCalled()
      expect(mockAgentService.executeAgent).toHaveBeenCalled()
    })

    it('should mark last iteration as willNotResume when resume is false', async () => {
      const taskWithIteration = {
        ...mockTask,
        iterations: [{ ...mockIteration, willNotResume: false }],
      }
      mockTaskStorage.get.mockResolvedValue(taskWithIteration)

      await service.sendMessage('task-123', 'Test message', false)

      const savedTask = mockTaskStorage.save.mock.calls[0][0]
      expect(savedTask.iterations[0].willNotResume).toBe(true)
    })

    it('should use resumeSessionId from previous iteration', async () => {
      const previousIteration = { ...mockIteration, agentSessionId: 'session-123' }
      const taskWithIteration = {
        ...mockTask,
        iterations: [previousIteration],
      }
      mockTaskStorage.get.mockResolvedValue(taskWithIteration)

      await service.sendMessage('task-123', 'Test message', true)

      expect(mockAgentService.executeAgent).toHaveBeenCalledWith(
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
      mockTaskStorage.get.mockResolvedValue(taskWithIteration)

      await service.sendMessage('task-123', 'Test message', true)

      // Check that the message was prepended with previous result
      const agentCall = mockAgentService.executeAgent.mock.calls[0][0]
      expect(agentCall.message).toContain('Previous result')
      expect(agentCall.resumeSessionId).toBeUndefined()
    })

    it('should handle command with willNotResume flag', async () => {
      const commandWithFlag = {
        name: 'test-command',
        prompt: 'Test',
        willNotResume: true,
      }
      mockCommandService.getCommand.mockResolvedValue(commandWithFlag)
      mockTaskStorage.get.mockResolvedValue({ ...mockTask, iterations: [] })

      await service.sendMessage(
        'task-123',
        'Test message',
        true,
        undefined,
        undefined,
        'test-command',
      )

      const savedTask = mockTaskStorage.save.mock.calls[0][0]
      const newIteration = savedTask.iterations[0]
      expect(newIteration.willNotResume).toBe(true)
    })

    it('should pass fileIds and contexts to agent', async () => {
      const fileIds = ['file-1', 'file-2']
      const contexts = [
        {
          type: 'element' as const,
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
        fileIds,
        undefined,
        undefined,
        contexts,
      )

      expect(mockAgentService.executeAgent).toHaveBeenCalledWith(
        expect.objectContaining({
          fileIds,
          contexts,
        }),
      )
    })
  })

  describe('startTask', () => {
    it('should activate created iteration and execute agent', async () => {
      const createdIteration = { ...mockIteration, status: 'created' as const }
      const taskWithCreatedIteration = {
        ...mockTask,
        iterations: [createdIteration],
      }

      mockTaskStorage.get.mockResolvedValue(taskWithCreatedIteration)
      mockWorkspaceCoordinator.getSnapshot.mockResolvedValue({ id: 'snapshot-1' })

      // Mock async generator for agent events
      const mockEventGenerator = (async function* () {
        yield { type: 'start' }
      })()
      mockAgentService.executeAgent.mockReturnValue(mockEventGenerator)

      await service.startTask('task-123')

      expect(mockTaskStorage.get).toHaveBeenCalledWith('task-123')
      expect(mockWorkspaceCoordinator.getSnapshot).toHaveBeenCalled()

      const savedTask = mockTaskStorage.save.mock.calls[0][0]
      expect(savedTask.iterations[0].status).toBe('active')

      expect(mockEventService.emitEvent).toHaveBeenCalledWith(
        expect.objectContaining({
          type: 'taskIteration',
          subType: 'active',
        }),
      )

      expect(mockAgentService.executeAgent).toHaveBeenCalled()
    })

    it('should throw TaskNotFoundError when task does not exist', async () => {
      mockTaskStorage.get.mockResolvedValue(null)

      await expect(service.startTask('task-123')).rejects.toThrow(TaskNotFoundError)
    })

    it('should throw ValidationError when last iteration is not created', async () => {
      const taskWithActiveIteration = {
        ...mockTask,
        iterations: [{ ...mockIteration, status: 'active' as const }],
      }

      mockTaskStorage.get.mockResolvedValue(taskWithActiveIteration)

      await expect(service.startTask('task-123')).rejects.toThrow(ValidationError)
    })

    it('should wait for workspace to be ready', async () => {
      const createdIteration = { ...mockIteration, status: 'created' as const }
      const taskWithCreatedIteration = {
        ...mockTask,
        iterations: [createdIteration],
      }

      mockTaskStorage.get.mockResolvedValue(taskWithCreatedIteration)

      // First two calls return null (not ready), third call returns snapshot (ready)
      mockWorkspaceCoordinator.getSnapshot
        .mockResolvedValueOnce(null)
        .mockResolvedValueOnce(null)
        .mockResolvedValueOnce({ id: 'snapshot-1' })

      // Mock async generator
      const mockEventGenerator = (async function* () {
        yield { type: 'start' }
      })()
      mockAgentService.executeAgent.mockReturnValue(mockEventGenerator)

      await service.startTask('task-123')

      expect(mockWorkspaceCoordinator.getSnapshot).toHaveBeenCalledTimes(3)
    })
  })

  describe('setupWorkspaceAndWaitForAgentStart', () => {
    it('should setup workspace and start task', async () => {
      const createdIteration = { ...mockIteration, status: 'created' as const }
      const taskWithCreatedIteration = {
        ...mockTask,
        iterations: [createdIteration],
      }

      mockTaskStorage.get.mockResolvedValue(taskWithCreatedIteration)
      mockWorkspaceCoordinator.getSnapshot.mockResolvedValue({ id: 'snapshot-1' })

      // Mock async generator
      const mockEventGenerator = (async function* () {
        yield { type: 'start' }
      })()
      mockAgentService.executeAgent.mockReturnValue(mockEventGenerator)

      // Pass message to trigger startTask call
      await service.setupWorkspaceAndWaitForAgentStart('task-123', true, 'Test message')

      expect(mockWorkspaceCoordinator.ensureWorkspace).toHaveBeenCalledWith('task-123')
      expect(mockWorkspaceCoordinator.getSnapshot).toHaveBeenCalled()
      expect(mockTaskStorage.get).toHaveBeenCalled()
    })

    it('should not start task when start=false', async () => {
      await service.setupWorkspaceAndWaitForAgentStart('task-123', false, 'Test message')

      expect(mockWorkspaceCoordinator.ensureWorkspace).toHaveBeenCalledWith('task-123')
      expect(mockTaskStorage.get).not.toHaveBeenCalled()
    })
  })
})
