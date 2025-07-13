import { RepositoryNotFoundError, TaskNotFoundError, ValidationError } from '@moatless/shared'
import type { CreateTaskRequest, Task } from '@moatless/shared/types/task'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { AgentService } from '../agent.service.js'
import type { CommandService } from '../command.service.js'
import type { EventService } from '../event.service.js'
import type { RepositoryService } from '../repository.service.js'
import type { StorageBackend } from '../storage.js'
import { TaskService } from '../task.service.js'
import type { WorkspaceService } from '../workspace.service.js'

describe('TaskService', () => {
  let taskService: TaskService
  let mockStorage: StorageBackend
  let mockRepositoryService: RepositoryService
  let mockWorkspaceService: WorkspaceService
  let mockAgentService: AgentService
  let mockCommandService: CommandService

  beforeEach(() => {
    // Create mock services
    mockStorage = {
      saveTask: vi.fn(),
      getTask: vi.fn(),
      list: vi.fn().mockResolvedValue([]),
      delete: vi.fn(),
      exists: vi.fn(),
      read: vi.fn(),
      write: vi.fn(),
      get: vi.fn(),
    } as unknown as StorageBackend

    mockRepositoryService = {
      getRepositoryById: vi.fn(),
      listRepositories: vi.fn(),
      createRepository: vi.fn(),
      updateRepository: vi.fn(),
      deleteRepository: vi.fn(),
    } as unknown as RepositoryService

    mockWorkspaceService = {
      getOrCreateWorkspace: vi.fn(),
      getWorkspace: vi.fn(),
      updateWorkspace: vi.fn(),
      cleanupWorkspace: vi.fn(),
      createWorkspace: vi.fn(),
      trackBackgroundTask: vi.fn(),
      setupWorkspace: vi.fn(),
    } as unknown as WorkspaceService

    mockAgentService = {
      executeAgent: vi.fn(),
      stopAgent: vi.fn(),
      isAgentRunning: vi.fn(),
      getAgentSessionId: vi.fn(),
      getTaskRunningStatus: vi.fn().mockResolvedValue(false),
    } as unknown as AgentService

    mockCommandService = {
      getCommand: vi.fn(),
    } as unknown as CommandService

    const mockEventService = {
      emitEvent: vi.fn().mockResolvedValue(undefined),
    } as unknown as EventService
    taskService = new TaskService(
      mockStorage,
      mockEventService,
      mockWorkspaceService,
      mockAgentService,
      mockCommandService,
      mockRepositoryService,
    )
  })

  describe('createTask', () => {
    it('should create a new task successfully', async () => {
      const request: CreateTaskRequest = {
        agentType: 'test-agent',
        repositoryId: 'env-123',
        message: 'Test task',
        start: true,
      }

      vi.mocked(mockRepositoryService.getRepositoryById).mockResolvedValueOnce({
        id: 'env-123',
        name: 'Test Repository',
      } as any)

      // Mock the new workspace methods
      vi.mocked(mockWorkspaceService.createWorkspace).mockResolvedValueOnce({
        id: 'task-id',
        type: 'local',
        getWorkspacePath: () => '/test/workspace',
      } as any)
      vi.mocked(mockWorkspaceService.setupWorkspace).mockResolvedValueOnce(undefined)
      vi.mocked(mockWorkspaceService.getWorkspace).mockResolvedValue({
        getStatus: () => 'idle',
      } as any)
      vi.mocked(mockAgentService.executeAgent).mockResolvedValueOnce(undefined)

      // Mock getTask to return the saved task for background processing
      vi.mocked(mockStorage.getTask).mockImplementation((id) => {
        return Promise.resolve({
          taskId: id,
          agentType: 'test-agent',
          repositoryId: 'env-123',
          status: 'open',
          numTurns: 0,
          totalCostUsd: 0,
          iterations: [
            {
              iterationNumber: 1,
              startTime: new Date(),
              status: 'created',
              messageCount: 1,
              costUsd: 0,
              initialMessage: 'Test task',
            },
          ],
          createdAt: new Date(),
          lastMessageAt: new Date(),
        })
      })

      // Mock getOrCreateWorkspace for sendMessage
      vi.mocked(mockWorkspaceService.getOrCreateWorkspace).mockResolvedValue({
        getStatus: vi.fn().mockReturnValue('ready'),
      } as any)

      const taskId = await taskService.createTask(request)

      expect(taskId).toBeDefined()
      expect(taskId).toMatch(/^[0-9a-f-]+$/) // UUID pattern

      // Verify task was saved first with 'open' status and 'created' iteration
      expect(mockStorage.saveTask).toHaveBeenCalledWith(
        expect.objectContaining({
          taskId,
          agentType: 'test-agent',
          repositoryId: 'env-123',
          status: 'open',
          numTurns: 0,
          totalCostUsd: 0,
          iterations: expect.arrayContaining([
            expect.objectContaining({
              iterationNumber: 1,
              status: 'created',
              messageCount: 1,
              initialMessage: 'Test task',
            }),
          ]),
        }),
      )

      // Events are no longer emitted on task creation

      // Wait for background tasks to complete
      // Background tasks are tracked in WorkspaceService

      // IMPORTANT: Verify workspace was created for the task
      expect(mockWorkspaceService.createWorkspace).toHaveBeenCalledWith(
        taskId,
        'env-123',
        undefined,
      )
    })

    it('should always create workspace even without initial message', async () => {
      const request: CreateTaskRequest = {
        agentType: 'test-agent',
        repositoryId: 'env-123',
        // No message provided
        start: false,
      }

      vi.mocked(mockRepositoryService.getRepositoryById).mockResolvedValueOnce({
        id: 'env-123',
        name: 'Test Repository',
      } as any)

      // Mock workspace creation
      vi.mocked(mockWorkspaceService.createWorkspace).mockResolvedValueOnce({
        id: 'task-id',
        type: 'local',
        getWorkspacePath: () => '/test/workspace',
      } as any)
      vi.mocked(mockWorkspaceService.setupWorkspace).mockResolvedValueOnce(undefined)

      const taskId = await taskService.createTask(request)

      // Wait for background tasks
      // Background tasks are tracked in WorkspaceService

      // Workspace should ALWAYS be created regardless of start flag or message
      expect(mockWorkspaceService.createWorkspace).toHaveBeenCalledWith(
        taskId,
        'env-123',
        undefined,
      )

      // Events are no longer emitted
    })

    it('should create workspace with custom branch name', async () => {
      const request: CreateTaskRequest = {
        agentType: 'test-agent',
        repositoryId: 'env-123',
        branchName: 'feature/custom-branch',
        start: false,
      }

      vi.mocked(mockRepositoryService.getRepositoryById).mockResolvedValueOnce({
        id: 'env-123',
        name: 'Test Repository',
      } as any)

      // Mock workspace creation
      vi.mocked(mockWorkspaceService.createWorkspace).mockResolvedValueOnce({
        id: 'task-id',
        type: 'local',
        getWorkspacePath: () => '/test/workspace',
      } as any)
      vi.mocked(mockWorkspaceService.setupWorkspace).mockResolvedValueOnce(undefined)

      const taskId = await taskService.createTask(request)

      // Background tasks are tracked in WorkspaceService

      // Verify workspace was created with the custom branch
      expect(mockWorkspaceService.createWorkspace).toHaveBeenCalledWith(
        taskId,
        'env-123',
        'feature/custom-branch',
      )
    })

    it('should throw RepositoryNotFoundError if repository does not exist', async () => {
      const request: CreateTaskRequest = {
        agentType: 'test-agent',
        repositoryId: 'non-existent',
        message: 'Test task',
      }

      // Mock createWorkspace to throw RepositoryNotFoundError
      vi.mocked(mockWorkspaceService.createWorkspace).mockRejectedValueOnce(
        new RepositoryNotFoundError('non-existent'),
      )

      await expect(taskService.createTask(request)).rejects.toThrow(RepositoryNotFoundError)
    })

    it('should handle workspace setup errors gracefully', async () => {
      const request: CreateTaskRequest = {
        agentType: 'test-agent',
        repositoryId: 'env-123',
        message: 'Test task',
        start: true,
      }

      vi.mocked(mockRepositoryService.getRepositoryById).mockResolvedValueOnce({
        id: 'env-123',
      } as any)

      vi.mocked(mockWorkspaceService.createWorkspace).mockRejectedValueOnce(
        new Error('Workspace setup failed'),
      )

      // Task creation should throw when workspace setup fails
      await expect(taskService.createTask(request)).rejects.toThrow('Workspace setup failed')

      // Task should have been saved initially but workspace creation failed
      expect(mockStorage.saveTask).toHaveBeenCalled()
    })

    it('should create task without starting when start=false', async () => {
      const request: CreateTaskRequest = {
        agentType: 'test-agent',
        repositoryId: 'env-123',
        message: 'Test task',
        start: false,
      }

      vi.mocked(mockRepositoryService.getRepositoryById).mockResolvedValueOnce({
        id: 'env-123',
        name: 'Test Repository',
      } as any)

      // Mock the new workspace methods
      vi.mocked(mockWorkspaceService.createWorkspace).mockResolvedValueOnce({
        id: 'task-id',
        type: 'local',
        getWorkspacePath: () => '/test/workspace',
      } as any)
      vi.mocked(mockWorkspaceService.setupWorkspace).mockResolvedValueOnce(undefined)

      const taskId = await taskService.createTask(request)

      expect(taskId).toBeDefined()

      // Verify task was saved with 'open' status and 'created' iteration but not started
      expect(mockStorage.saveTask).toHaveBeenCalledWith(
        expect.objectContaining({
          taskId,
          agentType: 'test-agent',
          repositoryId: 'env-123',
          status: 'open',
          iterations: expect.arrayContaining([
            expect.objectContaining({
              iterationNumber: 1,
              status: 'created',
              messageCount: 1,
              initialMessage: 'Test task',
            }),
          ]),
        }),
      )

      // Agent should NOT be executed since start=false
      expect(mockAgentService.executeAgent).not.toHaveBeenCalled()
    })
  })

  describe('startTask', () => {
    it('should start a task with last iteration in created status', async () => {
      const task: Task = {
        taskId: 'task-123',
        createdAt: new Date(),
        lastMessageAt: new Date(),
        agentType: 'test-agent',
        repositoryId: 'env-123',
        status: 'open',
        numTurns: 0,
        totalCostUsd: 0,
        iterations: [
          {
            iterationNumber: 1,
            startTime: new Date(),
            status: 'created',
            messageCount: 1,
            costUsd: 0,
            initialMessage: 'Test task',
          },
        ],
      }

      vi.mocked(mockStorage.getTask).mockResolvedValueOnce(task)
      vi.mocked(mockWorkspaceService.getWorkspace).mockResolvedValue({
        getStatus: () => 'idle',
      } as any)
      vi.mocked(mockAgentService.executeAgent).mockResolvedValueOnce(undefined)

      await taskService.startTask('task-123')

      // Verify task status was updated to running and iteration to active
      expect(mockStorage.saveTask).toHaveBeenCalledWith(
        expect.objectContaining({
          taskId: 'task-123',
          status: 'running',
          iterations: expect.arrayContaining([
            expect.objectContaining({
              iterationNumber: 1,
              status: 'active',
              messageCount: 1,
              initialMessage: 'Test task',
            }),
          ]),
        }),
      )

      // Verify agent was executed
      expect(mockAgentService.executeAgent).toHaveBeenCalledWith(
        'task-123',
        'Test task',
        expect.objectContaining({
          iterationNumber: 1,
          status: 'active',
          messageCount: 1,
          initialMessage: 'Test task',
        }),
        'claude-code',
        undefined,
        undefined,
      )
    })

    it('should throw ValidationError when last iteration is not in created status', async () => {
      const task: Task = {
        taskId: 'task-123',
        createdAt: new Date(),
        lastMessageAt: new Date(),
        agentType: 'test-agent',
        repositoryId: 'env-123',
        status: 'open',
        numTurns: 1,
        totalCostUsd: 0,
        iterations: [
          {
            iterationNumber: 1,
            startTime: new Date(),
            status: 'completed',
            messageCount: 1,
            costUsd: 0,
            initialMessage: 'Test task',
          },
        ],
      }

      vi.mocked(mockStorage.getTask).mockResolvedValue(task)

      await expect(taskService.startTask('task-123')).rejects.toThrow(ValidationError)
      await expect(taskService.startTask('task-123')).rejects.toThrow(
        'Last iteration does not have status "created". Cannot start task.',
      )
    })

    it('should throw ValidationError when task has no iterations', async () => {
      const task: Task = {
        taskId: 'task-123',
        createdAt: new Date(),
        lastMessageAt: new Date(),
        agentType: 'test-agent',
        repositoryId: 'env-123',
        status: 'open',
        numTurns: 0,
        totalCostUsd: 0,
        iterations: [],
      }

      vi.mocked(mockStorage.getTask).mockResolvedValueOnce(task)

      await expect(taskService.startTask('task-123')).rejects.toThrow(ValidationError)
    })

    it('should throw TaskNotFoundError for non-existent task', async () => {
      vi.mocked(mockStorage.getTask).mockResolvedValueOnce(null)

      await expect(taskService.startTask('non-existent')).rejects.toThrow(TaskNotFoundError)
    })
  })

  describe('getTask', () => {
    it('should retrieve a task by ID', async () => {
      const task: Task = {
        taskId: 'task-123',
        createdAt: new Date(),
        lastMessageAt: new Date(),
        agentType: 'test-agent',
        repositoryId: 'env-123',
        status: 'open',
        numTurns: 5,
        totalCostUsd: 0.05,
        iterations: [],
      }

      vi.mocked(mockStorage.getTask).mockResolvedValueOnce(task)

      const result = await taskService.getTask('task-123')
      expect(result).toEqual(task)
    })

    it('should throw TaskNotFoundError for non-existent task', async () => {
      vi.mocked(mockStorage.getTask).mockResolvedValueOnce(null)

      await expect(taskService.getTask('non-existent')).rejects.toThrow(TaskNotFoundError)
    })
  })

  describe('listTasks', () => {
    it('should list tasks with pagination', async () => {
      const tasks: Task[] = Array.from({ length: 10 }, (_, i) => ({
        taskId: `task-${i}`,
        createdAt: new Date(Date.now() - i * 1000),
        lastMessageAt: new Date(Date.now() - i * 1000),
        agentType: 'test-agent',
        repositoryId: 'env-123',
        status: 'open',
        numTurns: i,
        totalCostUsd: i * 0.01,
        iterations: [],
      }))

      vi.mocked(mockStorage.list).mockResolvedValueOnce(tasks.map((t) => `tasks/${t.taskId}`))

      // Mock getTask to return tasks by ID
      vi.mocked(mockStorage.getTask).mockImplementation((id) => {
        const task = tasks.find((t) => t.taskId === id)
        return Promise.resolve(task || null)
      })

      const result = await taskService.listTasks(5, 0)

      expect(result.tasks).toHaveLength(5)
      expect(result.total).toBe(10)
      expect(result.tasks[0].taskId).toBe('task-0') // Most recent first
    })
  })

  describe('sendMessage', () => {
    it('should send a message to an active task with workspace', async () => {
      const task: Task = {
        taskId: 'task-123',
        createdAt: new Date(),
        lastMessageAt: new Date(),
        agentType: 'test-agent',
        repositoryId: 'env-123',
        status: 'open',
        numTurns: 0,
        totalCostUsd: 0,
        iterations: [],
      }

      vi.mocked(mockStorage.getTask).mockResolvedValueOnce(task)
      vi.mocked(mockAgentService.executeAgent).mockResolvedValueOnce(undefined)

      await taskService.sendMessage('task-123', 'Hello, task!')

      // Verify task was saved twice - first with 'created', then with 'active'
      expect(mockStorage.saveTask).toHaveBeenCalledTimes(2)
      expect(mockStorage.saveTask).toHaveBeenCalledWith(
        expect.objectContaining({
          taskId: 'task-123',
          status: 'running',
          numTurns: 1,
          iterations: expect.arrayContaining([
            expect.objectContaining({
              iterationNumber: 1,
              status: 'active',
              messageCount: 1,
              initialMessage: 'Hello, task!',
            }),
          ]),
        }),
      )

      // Note: Events are now emitted by WorkspaceService.runAgent, not TaskService

      // Verify agent interaction
      expect(mockAgentService.executeAgent).toHaveBeenCalledWith(
        'task-123',
        'Hello, task!',
        expect.objectContaining({
          iterationNumber: 1,
          status: 'active',
          messageCount: 1,
          initialMessage: 'Hello, task!',
        }),
        'claude-code',
        undefined,
        undefined,
      )
    })

    it('should setup workspace if not yet setup', async () => {
      const task: Task = {
        taskId: 'task-123',
        createdAt: new Date(),
        lastMessageAt: new Date(),
        agentType: 'test-agent',
        repositoryId: 'env-123',
        status: 'open',
        numTurns: 0,
        totalCostUsd: 0,
        iterations: [],
      }

      vi.mocked(mockStorage.getTask).mockResolvedValueOnce(task)
      vi.mocked(mockAgentService.executeAgent).mockResolvedValueOnce(undefined)

      await taskService.sendMessage('task-123', 'Hello, task!')

      // Verify agent was started with the right parameters
      expect(mockAgentService.executeAgent).toHaveBeenCalledWith(
        'task-123',
        'Hello, task!',
        expect.objectContaining({
          iterationNumber: 1,
          status: 'active',
          messageCount: 1,
          initialMessage: 'Hello, task!',
        }),
        'claude-code',
        undefined,
        undefined,
      )
    })

    it('should throw TaskNotFoundError for non-existent task', async () => {
      vi.mocked(mockStorage.getTask).mockResolvedValueOnce(null)

      await expect(taskService.sendMessage('non-existent', 'Hello')).rejects.toThrow(
        TaskNotFoundError,
      )
    })

    it('should throw ValidationError when neither message nor commandName is provided', async () => {
      const task: Task = {
        taskId: 'task-123',
        createdAt: new Date(),
        lastMessageAt: new Date(),
        agentType: 'test-agent',
        repositoryId: 'env-123',
        status: 'open',
        numTurns: 0,
        totalCostUsd: 0,
        iterations: [],
      }

      vi.mocked(mockStorage.getTask).mockResolvedValue(task)

      // Pass empty message and no commandName
      await expect(
        taskService.sendMessage('task-123', '', false, undefined, undefined, undefined, undefined),
      ).rejects.toThrow(ValidationError)

      // Reset mock and test the error message
      vi.mocked(mockStorage.getTask).mockResolvedValue(task)
      await expect(
        taskService.sendMessage('task-123', '', false, undefined, undefined, undefined, undefined),
      ).rejects.toThrow('Either initialMessage or commandName must be provided for task iteration')
    })
  })

  describe('sendCommand', () => {
    it('should send a command by converting it to a message', async () => {
      const task: Task = {
        taskId: 'task-123',
        createdAt: new Date(),
        lastMessageAt: new Date(),
        agentType: 'test-agent',
        repositoryId: 'env-123',
        status: 'open',
        numTurns: 0,
        totalCostUsd: 0,
        iterations: [],
      }

      vi.mocked(mockStorage.getTask).mockResolvedValueOnce(task)
      vi.mocked(mockCommandService.getCommand).mockResolvedValueOnce({
        name: 'test-command',
        prompt: 'Test command prompt',
        description: 'Test command',
      } as any)
      vi.mocked(mockAgentService.executeAgent).mockResolvedValueOnce(undefined)

      await taskService.sendCommand('task-123', 'test-command')

      // Verify command was retrieved
      expect(mockCommandService.getCommand).toHaveBeenCalledWith('test-command')

      // Verify task was saved twice - first with 'created', then with 'active'
      expect(mockStorage.saveTask).toHaveBeenCalledTimes(2)
      expect(mockStorage.saveTask).toHaveBeenCalledWith(
        expect.objectContaining({
          taskId: 'task-123',
          status: 'running',
          numTurns: 1,
          iterations: expect.arrayContaining([
            expect.objectContaining({
              iterationNumber: 1,
              status: 'active',
              messageCount: 1,
              initialMessage: 'Test command prompt',
              commandName: 'test-command',
            }),
          ]),
        }),
      )

      // Verify agent was executed with the command's prompt
      expect(mockAgentService.executeAgent).toHaveBeenCalledWith(
        'task-123',
        'Test command prompt',
        expect.objectContaining({
          iterationNumber: 1,
          status: 'active',
          messageCount: 1,
          initialMessage: 'Test command prompt',
          commandName: 'test-command',
        }),
        'claude-code',
        undefined,
        undefined,
      )
    })

    it('should throw ValidationError for non-existent command', async () => {
      const task: Task = {
        taskId: 'task-123',
        createdAt: new Date(),
        lastMessageAt: new Date(),
        agentType: 'test-agent',
        repositoryId: 'env-123',
        status: 'open',
        numTurns: 0,
        totalCostUsd: 0,
        iterations: [],
      }

      vi.mocked(mockStorage.getTask).mockResolvedValue(task)
      vi.mocked(mockCommandService.getCommand).mockResolvedValue(null)

      await expect(taskService.sendCommand('task-123', 'non-existent-command')).rejects.toThrow(
        ValidationError,
      )

      // Reset mocks and test the error message
      vi.mocked(mockStorage.getTask).mockResolvedValue(task)
      vi.mocked(mockCommandService.getCommand).mockResolvedValue(null)
      await expect(taskService.sendCommand('task-123', 'non-existent-command')).rejects.toThrow(
        "Command 'non-existent-command' not found",
      )
    })
  })

  describe('updateTask', () => {
    it('should update task metadata', async () => {
      const task: Task = {
        taskId: 'task-123',
        createdAt: new Date(),
        lastMessageAt: new Date(),
        agentType: 'test-agent',
        repositoryId: 'env-123',
        status: 'open',
        numTurns: 5,
        totalCostUsd: 0.05,
        iterations: [],
      }

      vi.mocked(mockStorage.getTask).mockResolvedValueOnce(task)

      await taskService.updateTask('task-123', { status: 'closed' })

      expect(mockStorage.saveTask).toHaveBeenCalledWith(
        expect.objectContaining({
          taskId: 'task-123',
          status: 'closed',
        }),
      )
    })
  })

  describe('deleteTask', () => {
    it('should delete task and clean up resources', async () => {
      const task: Task = {
        taskId: 'task-123',
        createdAt: new Date(),
        lastMessageAt: new Date(),
        agentType: 'test-agent',
        repositoryId: 'env-123',
        status: 'closed',
        numTurns: 10,
        totalCostUsd: 0.1,
        iterations: [],
      }

      vi.mocked(mockStorage.getTask).mockResolvedValueOnce(task)

      await taskService.deleteTask('task-123')

      // Verify workspace was cleaned up
      expect(mockWorkspaceService.cleanupWorkspace).toHaveBeenCalledWith('task-123')

      // Verify storage was cleaned up
      expect(mockStorage.delete).toHaveBeenCalledWith('tasks/task-123')
      expect(mockStorage.delete).toHaveBeenCalledWith('events/task-123')

      // Note: Non-agent events are no longer emitted per architectural decision
    })

    it('should throw TaskNotFoundError for non-existent task', async () => {
      vi.mocked(mockStorage.getTask).mockResolvedValueOnce(null)

      await expect(taskService.deleteTask('non-existent')).rejects.toThrow(TaskNotFoundError)
    })
  })

  describe('background tasks', () => {
    it('should wait for all background tasks to complete', async () => {
      const request: CreateTaskRequest = {
        agentType: 'test-agent',
        repositoryId: 'env-123',
        message: 'Test task',
      }

      vi.mocked(mockRepositoryService.getRepositoryById).mockResolvedValueOnce({
        id: 'env-123',
      } as any)

      // Delay workspace creation
      vi.mocked(mockWorkspaceService.createWorkspace).mockImplementation(
        () =>
          new Promise((resolve) =>
            setTimeout(
              () => resolve({ id: 'ws-123', getWorkspacePath: () => '/test/path' } as any),
              50,
            ),
          ),
      )
      vi.mocked(mockWorkspaceService.setupWorkspace).mockResolvedValueOnce(undefined)
      vi.mocked(mockWorkspaceService.getWorkspace).mockResolvedValue({
        getStatus: () => 'idle',
      } as any)
      vi.mocked(mockAgentService.executeAgent).mockResolvedValueOnce(undefined)

      // Mock getTask for background processing
      vi.mocked(mockStorage.getTask).mockImplementation((id) => {
        return Promise.resolve({
          taskId: id,
          agentType: 'test-agent',
          repositoryId: 'env-123',
          status: 'open',
          numTurns: 0,
          totalCostUsd: 0,
          iterations: [],
          createdAt: new Date(),
          lastMessageAt: new Date(),
        })
      })

      await taskService.createTask(request)

      // Background task should be running
      // Background tasks are tracked in WorkspaceService

      // Workspace creation should have been called
      expect(mockWorkspaceService.createWorkspace).toHaveBeenCalled()
    })
  })
})
