import type { Task } from '@moatless/types'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { AgentService } from '../agent.service.js'
import type { CommandService } from '../command.service.js'
import type { EventService } from '../event.service.js'
import type { RepositoryService } from '../repository.service.js'
import type { StorageBackend } from '../storage.js'
import { TaskService } from '../task.service.js'
import type { WorkspaceService } from '../workspace.service.js'

describe('Task Status Management', () => {
  let taskService: TaskService
  let mockStorage: StorageBackend
  let mockWorkspaceService: WorkspaceService
  let mockAgentService: AgentService
  let mockCommandService: CommandService
  let mockRepositoryService: RepositoryService

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
    } as unknown as StorageBackend

    mockWorkspaceService = {
      createWorkspace: vi.fn(),
      getWorkspace: vi.fn(),
      setupWorkspace: vi.fn(),
      trackBackgroundTask: vi.fn(),
      stopServer: vi.fn(),
      cleanupWorkspace: vi.fn(),
    } as unknown as WorkspaceService

    mockAgentService = {
      executeAgent: vi.fn(),
      stopAgent: vi.fn(),
      getTaskRunningStatus: vi.fn(),
    } as unknown as AgentService

    mockCommandService = {
      getCommand: vi.fn(),
    } as unknown as CommandService

    mockRepositoryService = {
      getRepositoryById: vi.fn(),
      createRepository: vi.fn(),
    } as unknown as RepositoryService

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

  describe('Task Creation with start=true', () => {
    it('should set task status to running when agent is started immediately', async () => {
      // Mock workspace creation
      vi.mocked(mockWorkspaceService.createWorkspace).mockResolvedValueOnce({
        id: 'task-id',
        type: 'local',
        getWorkspacePath: () => '/test/workspace',
        getStatus: () => 'idle',
      } as any)

      // Mock workspace setup and getWorkspace
      vi.mocked(mockWorkspaceService.setupWorkspace).mockResolvedValueOnce(undefined)
      vi.mocked(mockWorkspaceService.getWorkspace).mockResolvedValue({
        getStatus: () => 'idle',
      } as any)

      // Track saved tasks to verify status changes
      const savedTasks: Task[] = []
      vi.mocked(mockStorage.saveTask).mockImplementation(async (task: Task) => {
        savedTasks.push({ ...task })
      })

      // Mock getTask to return the saved task
      vi.mocked(mockStorage.getTask).mockImplementation(async (taskId: string) => {
        const lastSaved = savedTasks.filter((t) => t.taskId === taskId).pop()
        return lastSaved || null
      })

      // Mock agent execution
      vi.mocked(mockAgentService.executeAgent).mockResolvedValueOnce(undefined)

      // Create task with start=true and message
      const task = await taskService.createTask({
        agentType: 'test-agent',
        repositoryId: 'env-123',
        message: 'Test message',
        start: true,
      })

      // Initial save should be with 'open' status and initial iteration with message
      expect(savedTasks[0]).toMatchObject({
        taskId: task.taskId,
        status: 'open',
        numTurns: 0,
        iterations: [
          expect.objectContaining({
            initialMessage: 'Test message',
            status: 'created',
            iterationNumber: 1,
          }),
        ],
      })

      // Wait for background tasks to process
      await new Promise((resolve) => setTimeout(resolve, 200))

      // After background processing:
      // 1. Workspace path should be set
      // 2. Status should be updated to 'running'
      // 3. Iteration status should be updated to 'active'
      expect(savedTasks.length).toBeGreaterThanOrEqual(3)

      // Find the save where status becomes 'running'
      const runningTask = savedTasks.find((t) => t.status === 'running')
      expect(runningTask).toBeDefined()
      expect(runningTask).toMatchObject({
        taskId: task.taskId,
        status: 'running',
        iterations: [
          expect.objectContaining({
            status: 'active',
          }),
        ],
      })

      // Verify agent was executed
      expect(mockAgentService.executeAgent).toHaveBeenCalledWith(
        task.taskId,
        'Test message',
        expect.objectContaining({
          iterationNumber: 1,
          status: 'active',
          initialMessage: 'Test message',
          messageCount: 1,
          costUsd: 0,
        }),
        'claude-code',
        undefined,
        undefined,
      )
    })

    it('should remain open when start=false', async () => {
      // Mock workspace creation
      vi.mocked(mockWorkspaceService.createWorkspace).mockResolvedValueOnce({
        id: 'task-id',
        type: 'local',
        getWorkspacePath: () => '/test/workspace',
      } as any)

      // Track saved tasks
      const savedTasks: Task[] = []
      vi.mocked(mockStorage.saveTask).mockImplementation(async (task: Task) => {
        savedTasks.push({ ...task })
      })

      // Create task with start=false
      const task = await taskService.createTask({
        agentType: 'test-agent',
        repositoryId: 'env-123',
        message: 'Test message',
        start: false,
      })

      // Wait for any background processing
      await new Promise((resolve) => setTimeout(resolve, 100))

      // Should have 2 saves: initial and after workspace setup
      expect(savedTasks.length).toBe(2)
      expect(savedTasks[0]).toMatchObject({
        taskId: task.taskId,
        status: 'open',
        numTurns: 0,
        iterations: [
          expect.objectContaining({
            initialMessage: 'Test message',
            status: 'created',
          }),
        ],
      })

      // Agent should not be executed
      expect(mockAgentService.executeAgent).not.toHaveBeenCalled()
    })

    it('should check agent running status in getTask', async () => {
      const task: Task = {
        taskId: 'task-123',
        createdAt: new Date(),
        lastMessageAt: new Date(),
        agentType: 'test-agent',
        repositoryId: 'env-123',
        status: 'running',
        numTurns: 1,
        totalCostUsd: 0,
        iterations: [],
      }

      vi.mocked(mockStorage.getTask).mockResolvedValueOnce(task)
      vi.mocked(mockAgentService.getTaskRunningStatus).mockResolvedValueOnce(false)

      const result = await taskService.getTask('task-123')

      // Should update status to open when agent is not running
      expect(result.status).toBe('open')
      expect(mockStorage.saveTask).toHaveBeenCalledWith(
        expect.objectContaining({
          taskId: 'task-123',
          status: 'open',
        }),
      )
    })

    it('should keep running status when agent is still running', async () => {
      const task: Task = {
        taskId: 'task-123',
        createdAt: new Date(),
        lastMessageAt: new Date(),
        agentType: 'test-agent',
        repositoryId: 'env-123',
        status: 'running',
        numTurns: 1,
        totalCostUsd: 0,
        iterations: [],
      }

      vi.mocked(mockStorage.getTask).mockResolvedValueOnce(task)
      vi.mocked(mockAgentService.getTaskRunningStatus).mockResolvedValueOnce(true)

      const result = await taskService.getTask('task-123')

      // Should keep running status
      expect(result.status).toBe('running')
      expect(mockStorage.saveTask).not.toHaveBeenCalled()
    })
  })

  describe('Task Status After Creation', () => {
    it('should return current task status after creation completes', async () => {
      // This test simulates getting the task right after creation
      // to verify the status reflects the actual state

      // Mock workspace creation
      vi.mocked(mockWorkspaceService.createWorkspace).mockResolvedValueOnce({
        id: 'task-id',
        type: 'local',
        getWorkspacePath: () => '/test/workspace',
        getStatus: () => 'idle',
      } as any)

      // Mock workspace being ready
      vi.mocked(mockWorkspaceService.getWorkspace).mockResolvedValue({
        getStatus: () => 'idle',
      } as any)

      // Track task states
      let currentTask: Task | null = null
      vi.mocked(mockStorage.saveTask).mockImplementation(async (task: Task) => {
        currentTask = { ...task }
      })

      vi.mocked(mockStorage.getTask).mockImplementation(async () => {
        return currentTask
      })

      // Mock agent is running
      vi.mocked(mockAgentService.getTaskRunningStatus).mockResolvedValue(true)
      vi.mocked(mockAgentService.executeAgent).mockImplementation(async () => {
        // Simulate agent execution taking some time
        await new Promise((resolve) => setTimeout(resolve, 50))
      })

      // Create task with start=true
      const task = await taskService.createTask({
        agentType: 'test-agent',
        repositoryId: 'env-123',
        message: 'Test message',
        start: true,
      })

      // Wait for background setup to start
      await new Promise((resolve) => setTimeout(resolve, 150))

      // Get task to check status
      const retrievedTask = await taskService.getTask(task.taskId)

      // Task should show as running since agent was started
      expect(retrievedTask.status).toBe('running')
      expect(retrievedTask.iterations).toHaveLength(1)
      expect(retrievedTask.iterations[0]).toMatchObject({
        status: 'active',
        initialMessage: 'Test message',
      })
    })
  })
})
