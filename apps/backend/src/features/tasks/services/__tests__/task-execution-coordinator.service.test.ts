import type { Task } from '@moatless/types'
import { TaskNotFoundError, ValidationError } from '@moatless/types'
import { Container } from 'inversify'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { CommandService } from '../../../agents/services/command.service'
import { MessageService } from '../../../agents/services/message.service'
import { AgentRequestBuilderSymbol } from '../../../agents/symbols'
import { EventService } from '../../../events/index'
import { WorkspaceRuntimeManager } from '../../../workspace/services/workspace-runtime-manager.service'
import { TaskRepository } from '../../db/task.repository'
import { TaskExecutionCoordinator } from '../task-execution-coordinator.service'

describe('TaskExecutionCoordinator', () => {
  let container: Container
  let service: TaskExecutionCoordinator
  let mockTaskRepository: any
  let mockMessageService: any
  let mockRuntimeManager: any
  let mockEventService: any
  let mockCommandService: any
  let mockAgentRequestBuilder: any

  const mockTask: Task = {
    taskId: 'task-123',
    name: 'Test Task',
    repositoryId: 'repo-1',
    status: 'running',
    agentType: 'claude-code',
    createdAt: new Date(),
    lastMessageAt: new Date(),
    totalCostUsd: 0,
    visibility: 'private' as const,
  }

  beforeEach(() => {
    container = new Container()

    // Create mocks with all required methods
    mockTaskRepository = {
      get: vi.fn(),
      getInternal: vi.fn(),
      save: vi.fn(),
    }

    mockMessageService = {
      saveUserMessage: vi.fn(),
      getLatestAgentSessionId: vi.fn().mockResolvedValue(undefined),
    }

    mockRuntimeManager = {
      getSandboxRecord: vi.fn().mockResolvedValue({ executionTarget: {} }),
      ensureWorkspace: vi.fn().mockResolvedValue({
        workspacePath: '/test/workspace',
        repositoryId: 'repo-1',
      }),
      isWorkspaceAvailable: vi.fn().mockResolvedValue(true),
      notifySandbox: vi.fn().mockResolvedValue(undefined),
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
        agentMode: undefined,
        model: undefined,
        resume: undefined,
        appendSystemPrompt: undefined,
      }),
    }

    // Bind mocks to container
    container.bind(TaskRepository).toConstantValue(mockTaskRepository)
    container.bind(MessageService).toConstantValue(mockMessageService)
    container.bind(WorkspaceRuntimeManager).toConstantValue(mockRuntimeManager)
    container.bind(EventService).toConstantValue(mockEventService)
    container.bind(CommandService).toConstantValue(mockCommandService)
    container.bind(AgentRequestBuilderSymbol).toConstantValue(mockAgentRequestBuilder)
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
      mockTaskRepository.get.mockResolvedValue({ ...mockTask })
      mockTaskRepository.save.mockResolvedValue(undefined)

      await service.sendCommand('task-123', commandName)

      expect(mockCommandService.getCommand).toHaveBeenCalledWith(commandName)
      // sendCommand delegates to sendMessage which saves user message
      expect(mockMessageService.saveUserMessage).toHaveBeenCalledWith(
        'task-123',
        mockCommand.prompt,
        undefined, // sessionId
        undefined, // turnNumber - no longer tracked
        undefined, // fileIds
        undefined, // contexts
        'edit', // agentMode from command
        undefined, // model
        undefined, // resumeSessionId
        expect.any(Array), // contentBlocks
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
      mockTaskRepository.get.mockResolvedValue({ ...mockTask })
      mockTaskRepository.save.mockResolvedValue(undefined)
    })

    it('should save user message and update task status', async () => {
      await service.sendMessage('task-123', 'Test message')

      expect(mockTaskRepository.get).toHaveBeenCalledWith('task-123')
      expect(mockMessageService.saveUserMessage).toHaveBeenCalledWith(
        'task-123',
        'Test message',
        undefined, // sessionId
        undefined, // turnNumber - no longer tracked
        undefined, // fileIds
        undefined, // contexts
        undefined, // agentMode
        undefined, // model
        undefined, // resumeSessionId
        expect.any(Array), // contentBlocks
      )
      expect(mockTaskRepository.save).toHaveBeenCalled()

      // Verify task status updated to running
      const savedTask = mockTaskRepository.save.mock.calls[0][0]
      expect(savedTask.status).toBe('running')
    })

    it('should not emit taskTurn event (events are handled by sandbox)', async () => {
      await service.sendMessage('task-123', 'Test message')

      expect(mockEventService.emitEvent).not.toHaveBeenCalled()
    })

    it('should notify sandbox after saving message', async () => {
      await service.sendMessage('task-123', 'Test message')

      expect(mockRuntimeManager.notifySandbox).toHaveBeenCalledWith('task-123')
    })

    it('should pass contexts to saveUserMessage', async () => {
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

      expect(mockMessageService.saveUserMessage).toHaveBeenCalledWith(
        'task-123',
        'Test message',
        undefined, // sessionId
        undefined, // turnNumber - no longer tracked
        undefined, // fileIds
        contexts,
        undefined, // agentMode
        undefined, // model
        undefined, // resumeSessionId
        expect.any(Array), // contentBlocks
      )
    })
  })

  describe('startTask', () => {
    it('should update task status and notify sandbox', async () => {
      const openTask = { ...mockTask, status: 'open' as const }
      mockTaskRepository.get.mockResolvedValue(openTask)

      await service.startTask('task-123')

      expect(mockTaskRepository.get).toHaveBeenCalledWith('task-123')
      expect(mockRuntimeManager.getSandboxRecord).toHaveBeenCalled()

      const savedTask = mockTaskRepository.save.mock.calls[0][0]
      expect(savedTask.status).toBe('running')

      expect(mockEventService.emitEvent).toHaveBeenCalledWith(
        expect.objectContaining({
          type: 'taskTurn',
          subType: 'active',
          turnNumber: 0,
        }),
      )

      expect(mockRuntimeManager.notifySandbox).toHaveBeenCalledWith('task-123')
    })

    it('should throw TaskNotFoundError when task does not exist', async () => {
      mockTaskRepository.get.mockResolvedValue(null)

      await expect(service.startTask('task-123')).rejects.toThrow(TaskNotFoundError)
    })

    it('should throw ValidationError when task is not in startable state', async () => {
      const runningTask = { ...mockTask, status: 'running' as const }
      mockTaskRepository.get.mockResolvedValue(runningTask)

      await expect(service.startTask('task-123')).rejects.toThrow(ValidationError)
    })

    it('should wait for workspace to be ready', async () => {
      const openTask = { ...mockTask, status: 'open' as const }
      mockTaskRepository.get.mockResolvedValue(openTask)

      // First two calls return null (not ready), third call returns record (ready)
      mockRuntimeManager.getSandboxRecord
        .mockResolvedValueOnce(null)
        .mockResolvedValueOnce(null)
        .mockResolvedValueOnce({ executionTarget: {} })

      await service.startTask('task-123')

      expect(mockRuntimeManager.getSandboxRecord).toHaveBeenCalledTimes(3)
      expect(mockRuntimeManager.notifySandbox).toHaveBeenCalled()
    })
  })

  describe('resumeTask', () => {
    it('should resume task and notify sandbox', async () => {
      const runningTask = { ...mockTask, status: 'running' as const }
      mockTaskRepository.getInternal.mockResolvedValue(runningTask)

      await service.resumeTask('task-123', 'session-456')

      expect(mockRuntimeManager.notifySandbox).toHaveBeenCalledWith('task-123')
    })

    it('should throw TaskNotFoundError when task not found', async () => {
      mockTaskRepository.getInternal.mockResolvedValue(null)

      await expect(service.resumeTask('task-123', 'session-456')).rejects.toThrow(TaskNotFoundError)
    })

    it('should throw ValidationError when task is not running', async () => {
      const openTask = { ...mockTask, status: 'open' as const }
      mockTaskRepository.getInternal.mockResolvedValue(openTask)

      await expect(service.resumeTask('task-123', 'session-456')).rejects.toThrow(ValidationError)
    })
  })
})
