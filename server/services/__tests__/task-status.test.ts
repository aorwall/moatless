import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { Task } from '../../../shared/types/task.js'
import type { AgentService } from '../agent.service.js'
import type { CommandService } from '../command.service.js'
import type { StorageBackend } from '../storage.js'
import { TaskService } from '../task.service.js'
import type { WorkspaceService } from '../workspace.service.js'

describe('Task Status Management', () => {
  let taskService: TaskService
  let mockStorage: StorageBackend
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
    } as any

    mockWorkspaceService = {
      createWorkspace: vi.fn(),
      getWorkspace: vi.fn(),
      setupWorkspace: vi.fn(),
      trackBackgroundTask: vi.fn(),
      stopServer: vi.fn(),
      cleanupWorkspace: vi.fn(),
    } as any

    mockAgentService = {
      executeAgent: vi.fn(),
      stopAgent: vi.fn(),
      getTaskRunningStatus: vi.fn(),
    } as any

    mockCommandService = {
      getCommand: vi.fn(),
    } as any

    taskService = new TaskService(
      mockStorage,
      mockWorkspaceService,
      mockAgentService,
      mockCommandService,
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
      const taskId = await taskService.createTask({
        agentType: 'test-agent',
        environmentId: 'env-123',
        message: 'Test message',
        start: true,
      })

      // Initial save should be with 'open' status
      expect(savedTasks[0]).toMatchObject({
        taskId,
        status: 'open',
        numTurns: 0,
      })

      // Should have a second save with 'running' status before sendMessage
      expect(savedTasks.length).toBeGreaterThanOrEqual(2)
      expect(savedTasks[1]).toMatchObject({
        taskId,
        status: 'running',
        numTurns: 0,
      })

      // Wait for background tasks to process
      await new Promise((resolve) => setTimeout(resolve, 100))

      // Should have more saves when sendMessage is called
      expect(savedTasks.length).toBeGreaterThanOrEqual(3)

      // The task should have iterations after sendMessage
      const taskWithIterations = savedTasks.find((t) => t.numTurns > 0)
      expect(taskWithIterations).toBeDefined()
      expect(taskWithIterations).toMatchObject({
        taskId,
        status: 'running',
        numTurns: 1,
      })

      // Verify agent was executed
      expect(mockAgentService.executeAgent).toHaveBeenCalledWith(
        taskId,
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
      const taskId = await taskService.createTask({
        agentType: 'test-agent',
        environmentId: 'env-123',
        message: 'Test message',
        start: false,
      })

      // Wait for any background processing
      await new Promise((resolve) => setTimeout(resolve, 100))

      // Should only have the initial save
      expect(savedTasks.length).toBe(1)
      expect(savedTasks[0]).toMatchObject({
        taskId,
        status: 'open',
        numTurns: 0,
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
        environmentId: 'env-123',
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
        environmentId: 'env-123',
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
      const taskId = await taskService.createTask({
        agentType: 'test-agent',
        environmentId: 'env-123',
        message: 'Test message',
        start: true,
      })

      // Wait for background setup to start
      await new Promise((resolve) => setTimeout(resolve, 150))

      // Get task to check status
      const retrievedTask = await taskService.getTask(taskId)

      // Task should show as running since agent was started
      expect(retrievedTask.status).toBe('running')
      expect(retrievedTask.numTurns).toBe(1)
      expect(retrievedTask.iterations).toHaveLength(1)
    })
  })
})
