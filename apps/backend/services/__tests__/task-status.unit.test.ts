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
      removeWorktree: vi.fn(),
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
      subscribeAll: vi.fn().mockReturnValue(() => {}), // Returns unsubscribe function
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
    it('should remain open status when start=true', async () => {
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

      // Wait longer for background tasks to process
      await new Promise((resolve) => setTimeout(resolve, 300))

      // After background processing:
      // 1. Workspace path should be set
      // 2. Task should remain 'open' (no more 'running' status)
      // 3. Agent should have been started
      expect(savedTasks.length).toBeGreaterThanOrEqual(2)

      // All saved tasks should remain 'open'
      savedTasks.forEach((task) => {
        expect(task.status).toBe('open')
      })

      // The main point is that tasks remain 'open' and don't transition to 'running'
      // Agent execution happens asynchronously and may not complete in test timeframes
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

    it('should return task status without modification', async () => {
      const task: Task = {
        taskId: 'task-123',
        createdAt: new Date(),
        lastMessageAt: new Date(),
        agentType: 'test-agent',
        repositoryId: 'env-123',
        status: 'open',
        numTurns: 1,
        totalCostUsd: 0,
        iterations: [],
      }

      vi.mocked(mockStorage.getTask).mockResolvedValueOnce(task)

      const result = await taskService.getTask('task-123')

      // Should return task as-is (no more running status logic in getTask)
      expect(result.status).toBe('open')
      expect(mockStorage.saveTask).not.toHaveBeenCalled()
    })

    it('should return open task status consistently', async () => {
      const task: Task = {
        taskId: 'task-123',
        createdAt: new Date(),
        lastMessageAt: new Date(),
        agentType: 'test-agent',
        repositoryId: 'env-123',
        status: 'open',
        numTurns: 1,
        totalCostUsd: 0,
        iterations: [],
      }

      vi.mocked(mockStorage.getTask).mockResolvedValueOnce(task)

      const result = await taskService.getTask('task-123')

      // Should return open status (running state is tracked by agent, not task)
      expect(result.status).toBe('open')
      expect(mockStorage.saveTask).not.toHaveBeenCalled()
    })
  })

  describe('Task Status After Creation', () => {
    it('should return open status after creation completes', async () => {
      // This test simulates getting the task right after creation
      // to verify the status remains open (running state tracked by agent)

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

      // Task should remain open (running state is tracked by agent, not task)
      expect(retrievedTask.status).toBe('open')
      expect(retrievedTask.iterations).toHaveLength(1)
      expect(retrievedTask.iterations[0]).toMatchObject({
        initialMessage: 'Test message',
      })

      // Iteration status may be 'created' or 'active' depending on timing of background execution
      expect(['created', 'active']).toContain(retrievedTask.iterations[0].status)
    })
  })
})
