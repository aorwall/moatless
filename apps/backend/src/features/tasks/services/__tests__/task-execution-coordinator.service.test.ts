import type { Task, TaskIteration } from '@moatless/types'
import { TaskNotFoundError, ValidationError } from '@moatless/types'
import { Container } from 'inversify'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { CommandService } from '../../../agents/services/command.service'
import { MessageService } from '../../../agents/services/message.service'
import { AgentControllerSymbol } from '../../../agents/symbols'
import { EventService } from '../../../events/index'
import { RepositoryService } from '../../../repositories/index'
import { WorkspaceCoordinator } from '../../../workspace/services/workspace-coordinator.service'
import { TaskRepository } from '../../db/task.repository'
import { TaskExecutionCoordinator } from '../task-execution-coordinator.service'
import { TaskIterationService } from '../task-iteration.service'

describe('TaskExecutionCoordinator', () => {
  let container: Container
  let service: TaskExecutionCoordinator
  let mockTaskRepository: any
  let mockIterationService: any
  let mockMessageService: any
  let mockWorkspaceCoordinator: any
  let mockAgentController: any
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

    mockAgentController = {
      startAgent: vi.fn().mockResolvedValue(undefined),
      stopAgent: vi.fn(),
      runAgentInBackground: vi.fn().mockResolvedValue(undefined),
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
    container.bind(TaskRepository).toConstantValue(mockTaskRepository)
    container.bind(TaskIterationService).toConstantValue(mockIterationService)
    container.bind(MessageService).toConstantValue(mockMessageService)
    container.bind(WorkspaceCoordinator).toConstantValue(mockWorkspaceCoordinator)
    container.bind(AgentControllerSymbol).toConstantValue(mockAgentController)
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

    it('should create iteration and execute agent', async () => {
      await service.sendMessage('task-123', 'Test message')

      expect(mockTaskRepository.get).toHaveBeenCalledWith('task-123')
      expect(mockIterationService.createIteration).toHaveBeenCalled()
      expect(mockTaskRepository.save).toHaveBeenCalled()
      // Note: mockAgentController.startAgent is called in background via executeAgentInBackground()
      // so we can't reliably assert it was called synchronously
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

      expect(mockAgentController.runAgentInBackground).toHaveBeenCalledWith(
        'task-123',
        'claude-code',
        'Test message',
        expect.objectContaining({
          resume: 'session-123',
        }),
        expect.any(Object),
        'repo-1',
        expect.any(Object),
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

      // Check that the message was prepended with previous result
      const agentCall = mockAgentController.runAgentInBackground.mock.calls[0]
      expect(agentCall[2]).toContain('Previous result') // message is third parameter
      expect(agentCall[3]?.resume).toBeUndefined() // runOptions.resume should be undefined
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

    it('should pass fileIds and contexts to agent', async () => {
      const fileIds = ['file-1', 'file-2']
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
        fileIds,
        undefined,
        undefined,
        contexts,
      )

      expect(mockAgentController.runAgentInBackground).toHaveBeenCalledWith(
        'task-123',
        'claude-code',
        'Test message',
        expect.objectContaining({
          fileIds,
          contexts,
        }),
        expect.any(Object),
        'repo-1',
        expect.any(Object),
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

      mockTaskRepository.get.mockResolvedValue(taskWithCreatedIteration)
      mockWorkspaceCoordinator.getSnapshot.mockResolvedValue({ id: 'snapshot-1' })

      await service.startTask('task-123')

      expect(mockTaskRepository.get).toHaveBeenCalledWith('task-123')
      expect(mockWorkspaceCoordinator.getSnapshot).toHaveBeenCalled()

      const savedTask = mockTaskRepository.save.mock.calls[0][0]
      expect(savedTask.iterations[0].status).toBe('active')

      expect(mockEventService.emitEvent).toHaveBeenCalledWith(
        expect.objectContaining({
          type: 'taskIteration',
          subType: 'active',
        }),
      )

      expect(mockAgentController.runAgentInBackground).toHaveBeenCalled()
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
      mockWorkspaceCoordinator.getSnapshot
        .mockResolvedValueOnce(null)
        .mockResolvedValueOnce(null)
        .mockResolvedValueOnce({ id: 'snapshot-1' })

      await service.startTask('task-123')

      expect(mockWorkspaceCoordinator.getSnapshot).toHaveBeenCalledTimes(3)
      expect(mockAgentController.runAgentInBackground).toHaveBeenCalled()
    })
  })

  describe('setupWorkspaceAndWaitForAgentStart', () => {
    it('should setup workspace and start task', async () => {
      const createdIteration = { ...mockIteration, status: 'created' as const }
      const taskWithCreatedIteration = {
        ...mockTask,
        iterations: [createdIteration],
      }

      mockTaskRepository.get.mockResolvedValue(taskWithCreatedIteration)
      mockWorkspaceCoordinator.getSnapshot.mockResolvedValue({ id: 'snapshot-1' })

      // Pass message to trigger startTask call
      await service.setupWorkspaceAndWaitForAgentStart('task-123', true, 'Test message')

      expect(mockWorkspaceCoordinator.ensureWorkspace).toHaveBeenCalledWith('task-123')
      expect(mockWorkspaceCoordinator.getSnapshot).toHaveBeenCalled()
      expect(mockTaskRepository.get).toHaveBeenCalled()
      expect(mockAgentController.runAgentInBackground).toHaveBeenCalled()
    })

    it('should not start task when start=false', async () => {
      await service.setupWorkspaceAndWaitForAgentStart('task-123', false, 'Test message')

      expect(mockWorkspaceCoordinator.ensureWorkspace).toHaveBeenCalledWith('task-123')
      expect(mockTaskRepository.get).not.toHaveBeenCalled()
    })
  })
})
