import type { Task, TaskIteration } from '@moatless/types'
import { EventType, WorkspaceStatus } from '@moatless/types'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { BaseAgent } from '../../agents/base-agent.js'
import { ClaudeCodeAgent } from '../../agents/claude-code/claude-code-agent.js'
import type { Workspace } from '../../workspaces/workspace.js'
import { AgentService } from '../agent.service.js'
import type { EventService } from '../event.service.js'
import type { StorageBackend } from '../storage.js'
import type { WorkspaceService } from '../workspace.service.js'

// Mock dependencies
vi.mock('../../utils/logger', () => ({
  logger: {
    info: vi.fn(),
    error: vi.fn(),
    warn: vi.fn(),
    debug: vi.fn(),
    child: vi.fn(() => ({
      info: vi.fn(),
      error: vi.fn(),
      warn: vi.fn(),
      debug: vi.fn(),
    })),
  },
}))

vi.mock('../../agents/claude-code/claude-code-agent.js')

// Create mock storage
const createMockStorage = (): StorageBackend => ({
  save: vi.fn(),
  get: vi.fn(),
  delete: vi.fn(),
  list: vi.fn().mockResolvedValue([]),
  exists: vi.fn(),
  saveTask: vi.fn(),
  getTask: vi.fn(),
  saveRepository: vi.fn(),
  getRepository: vi.fn(),
  saveEvents: vi.fn(),
  getEvents: vi.fn(),
  saveWorkspace: vi.fn(),
  getWorkspace: vi.fn(),
})

// Create mock event service
const createMockEventService = (): EventService =>
  ({
    saveEvents: vi.fn(),
    getEvents: vi.fn(),
    emitEvent: vi.fn(),
    on: vi.fn(),
    off: vi.fn(),
    emit: vi.fn(),
  }) as any

// Create mock workspace service
const createMockWorkspaceService = (): WorkspaceService =>
  ({
    getOrCreateWorkspace: vi.fn(),
    getWorkspace: vi.fn(),
    cleanupWorkspace: vi.fn(),
    getAllWorkspaces: vi.fn(),
    getWorkspaceStatus: vi.fn(),
    createWorkspace: vi.fn(),
    setupWorkspace: vi.fn(),
  }) as any

// Create mock workspace
const createMockWorkspace = (status: WorkspaceStatus = WorkspaceStatus.IDLE): Workspace =>
  ({
    getStatus: vi.fn().mockReturnValue(status),
    isWorkspacePathValid: vi.fn().mockResolvedValue(true),
    getWorkspacePath: vi.fn().mockReturnValue('/workspace/test'),
    getId: vi.fn().mockReturnValue('workspace-id'),
    resetWorkspace: vi.fn(),
    getEnvironment: vi.fn().mockReturnValue({
      id: 'env-1',
      name: 'Test Env',
      path: '/test/repo',
      codeExecution: { type: 'local' },
      type: 'repository',
    }),
  }) as any

// Create mock agent
const createMockAgent = (): BaseAgent =>
  ({
    run: vi.fn().mockImplementation(async function* () {
      yield {
        eventId: 'event-1',
        type: EventType.COMPLETION,
        taskId: 'task-1',
        timestamp: new Date(),
        agentType: 'claude-code',
        isError: false,
        payload: { message: 'Test completed' },
      }
    }),
    stop: vi.fn(),
    getAgentType: vi.fn().mockReturnValue('claude-code'),
    getSessionId: vi.fn(),
  }) as any

describe('AgentService', () => {
  let service: AgentService
  let mockStorage: StorageBackend
  let mockEventService: EventService
  let mockWorkspaceService: WorkspaceService

  const mockTask: Task = {
    taskId: 'task-1',
    repositoryId: 'repo-1',
    agentType: 'claude-code',
    status: 'open',
    createdAt: new Date(),
    lastMessageAt: new Date(),
    numTurns: 0,
    totalCostUsd: 0,
    iterations: [],
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
    mockStorage = createMockStorage()
    mockEventService = createMockEventService()
    mockWorkspaceService = createMockWorkspaceService()
    service = new AgentService(mockStorage, mockEventService, mockWorkspaceService)
    vi.clearAllMocks()
  })

  afterEach(() => {
    vi.restoreAllMocks()
  })

  describe('executeAgent', () => {
    beforeEach(() => {
      vi.mocked(mockStorage.getTask).mockResolvedValue(mockTask)
    })

    it('should execute agent successfully', async () => {
      const mockWorkspace = createMockWorkspace()
      vi.mocked(mockWorkspaceService.getOrCreateWorkspace).mockResolvedValue(mockWorkspace)

      const mockAgent = createMockAgent()
      // Mock agent run to return events
      vi.mocked(mockAgent.run).mockImplementation(async function* () {
        yield {
          eventId: 'event-1',
          type: EventType.COMPLETION,
          taskId: 'task-1',
          timestamp: new Date(),
          agentType: 'claude-code',
          isError: false,
          payload: { message: 'Test completed' },
        }
      })
      vi.mocked(ClaudeCodeAgent).mockImplementation(() => mockAgent as any)

      await service.executeAgent('task-1', 'Test message', mockIteration)

      expect(mockAgent.run).toHaveBeenCalledWith(
        'Test message',
        expect.objectContaining({
          iterationNumber: 1,
        }),
      )
      expect(mockStorage.saveTask).toHaveBeenCalled()
    })

    it('should throw error if agent is already running', async () => {
      const mockWorkspace = createMockWorkspace()
      vi.mocked(mockWorkspaceService.getOrCreateWorkspace).mockResolvedValue(mockWorkspace)

      const mockAgent = createMockAgent()
      // Make the agent run method take some time
      vi.mocked(mockAgent.run).mockImplementation(async function* () {
        await new Promise((resolve) => setTimeout(resolve, 100))
        yield {
          eventId: 'event-1',
          type: EventType.COMPLETION,
          taskId: 'task-1',
          timestamp: new Date(),
          agentType: 'claude-code',
          isError: false,
          payload: { message: 'Test completed' },
        }
      })
      vi.mocked(ClaudeCodeAgent).mockImplementation(() => mockAgent as any)

      // Start first agent
      const promise1 = service.executeAgent('task-1', 'Message 1', mockIteration)

      // Give it a moment to start
      await new Promise((resolve) => setTimeout(resolve, 10))

      // Try to start second agent for same task
      await expect(service.executeAgent('task-1', 'Message 2', mockIteration)).rejects.toThrow(
        'Agent is already running',
      )

      // Clean up
      await promise1
    })

    it('should throw error if workspace is setting up', async () => {
      const mockWorkspace = createMockWorkspace(WorkspaceStatus.SETTING_UP)
      vi.mocked(mockWorkspaceService.getOrCreateWorkspace).mockResolvedValue(mockWorkspace)

      await expect(service.executeAgent('task-1', 'Test message', mockIteration)).rejects.toThrow(
        'Workspace is still setting up',
      )
    })

    it('should recreate workspace if path is invalid', async () => {
      const mockWorkspace = createMockWorkspace()
      vi.mocked(mockWorkspace.isWorkspacePathValid).mockResolvedValue(false)
      vi.mocked(mockWorkspaceService.getOrCreateWorkspace).mockResolvedValue(mockWorkspace)

      // Mock the recreated workspace as ready
      const recreatedWorkspace = createMockWorkspace(WorkspaceStatus.IDLE)
      vi.mocked(recreatedWorkspace.isWorkspacePathValid).mockResolvedValue(true)
      vi.mocked(mockWorkspaceService.getWorkspace).mockResolvedValue(recreatedWorkspace)

      // Mock setupWorkspace to simulate successful recreation
      vi.mocked(mockWorkspaceService.setupWorkspace).mockResolvedValue(undefined)

      // After setup, getOrCreateWorkspace should return the recreated workspace
      let setupCalled = false
      vi.mocked(mockWorkspaceService.setupWorkspace).mockImplementation(async () => {
        setupCalled = true
      })
      vi.mocked(mockWorkspaceService.getOrCreateWorkspace).mockImplementation(async () => {
        if (setupCalled) {
          return recreatedWorkspace
        }
        return mockWorkspace
      })

      const mockAgent = createMockAgent()
      vi.mocked(ClaudeCodeAgent).mockImplementation(() => mockAgent as any)

      await service.executeAgent('task-1', 'Test message', mockIteration)

      expect(mockWorkspaceService.setupWorkspace).toHaveBeenCalledWith(
        'task-1',
        mockIteration.iterationNumber,
      )
      expect(mockWorkspaceService.getOrCreateWorkspace).toHaveBeenCalledTimes(2)
    }, 35000)

    it('should support different agent types', async () => {
      const mockWorkspace = createMockWorkspace()
      vi.mocked(mockWorkspaceService.getOrCreateWorkspace).mockResolvedValue(mockWorkspace)

      const mockAgent = createMockAgent()
      vi.mocked(ClaudeCodeAgent).mockImplementation(() => mockAgent as any)

      await service.executeAgent(
        'task-1',
        'Test message',
        mockIteration,
        'claude-code',
        undefined,
        'claude-3-opus',
      )

      expect(ClaudeCodeAgent).toHaveBeenCalledWith('task-1', { model: 'claude-3-opus' })
    })

    it('should handle agent execution errors', async () => {
      // Set up task with the iteration
      const taskWithIteration = {
        ...mockTask,
        iterations: [mockIteration],
      }
      vi.mocked(mockStorage.getTask).mockResolvedValue(taskWithIteration)

      const mockWorkspace = createMockWorkspace()
      vi.mocked(mockWorkspaceService.getOrCreateWorkspace).mockResolvedValue(mockWorkspace)

      const mockAgent = createMockAgent()
      vi.mocked(mockAgent.run).mockImplementation(async function* () {
        throw new Error('Agent execution failed')
      })
      vi.mocked(ClaudeCodeAgent).mockImplementation(() => mockAgent as any)

      // The error should be thrown by executeAgent
      await expect(service.executeAgent('task-1', 'Test message', mockIteration)).rejects.toThrow(
        'Agent execution failed',
      )

      // Verify error handling occurred
      expect(mockStorage.saveTask).toHaveBeenCalledWith(
        expect.objectContaining({
          status: 'error',
        }),
      )

      expect(mockEventService.emitEvent).toHaveBeenCalledWith(
        expect.objectContaining({
          taskId: 'task-1',
          type: EventType.TASK_STATUS,
          subType: 'error',
          isError: true,
        }),
      )
    })

    it('should resume session when sessionId provided', async () => {
      const mockWorkspace = createMockWorkspace()
      vi.mocked(mockWorkspaceService.getOrCreateWorkspace).mockResolvedValue(mockWorkspace)

      const mockAgent = createMockAgent()
      vi.mocked(ClaudeCodeAgent).mockImplementation(() => mockAgent as any)

      const resumeSessionId = 'session-123'
      await service.executeAgent(
        'task-1',
        'Test message',
        mockIteration,
        'claude-code',
        resumeSessionId,
      )

      expect(mockAgent.run).toHaveBeenCalledWith(
        'Test message',
        expect.objectContaining({
          resume: resumeSessionId,
        }),
      )
    })
  })

  describe('stopAgent', () => {
    beforeEach(() => {
      vi.mocked(mockStorage.getTask).mockResolvedValue(mockTask)
    })

    it('should stop running agent', async () => {
      // First, start an agent
      const mockWorkspace = createMockWorkspace()
      vi.mocked(mockWorkspaceService.getOrCreateWorkspace).mockResolvedValue(mockWorkspace)

      const mockAgent = createMockAgent()
      // Make the agent run method take some time
      vi.mocked(mockAgent.run).mockImplementation(async function* () {
        await new Promise((resolve) => setTimeout(resolve, 100))
        yield {
          eventId: 'event-1',
          type: EventType.COMPLETION,
          taskId: 'task-1',
          timestamp: new Date(),
          agentType: 'claude-code',
          isError: false,
          payload: { message: 'Test completed' },
        }
      })
      vi.mocked(ClaudeCodeAgent).mockImplementation(() => mockAgent as any)

      const agentPromise = service.executeAgent('task-1', 'Test message', mockIteration)

      // Give it a moment to start
      await new Promise((resolve) => setTimeout(resolve, 10))

      // Now stop it
      const result = await service.stopAgent('task-1')

      expect(result).toBe(true)
      expect(mockAgent.stop).toHaveBeenCalled()

      // Clean up
      await agentPromise
    })

    it('should return false when no agent is running', async () => {
      const result = await service.stopAgent('non-existent')
      expect(result).toBe(false)
    })
  })

  describe('agent tracking', () => {
    beforeEach(() => {
      vi.mocked(mockStorage.getTask).mockResolvedValue(mockTask)
    })

    it('should return progress for running agent', async () => {
      // Start an agent
      const mockWorkspace = createMockWorkspace()
      vi.mocked(mockWorkspaceService.getOrCreateWorkspace).mockResolvedValue(mockWorkspace)

      const mockAgent = createMockAgent()
      // Make the agent run method take some time
      vi.mocked(mockAgent.run).mockImplementation(async function* () {
        await new Promise((resolve) => setTimeout(resolve, 100))
        yield {
          eventId: 'event-1',
          type: EventType.COMPLETION,
          taskId: 'task-1',
          timestamp: new Date(),
          agentType: 'claude-code',
          isError: false,
          payload: { message: 'Test completed' },
        }
      })
      vi.mocked(ClaudeCodeAgent).mockImplementation(() => mockAgent as any)

      const agentPromise = service.executeAgent('task-1', 'Test message', mockIteration)

      // Give it a moment to start
      await new Promise((resolve) => setTimeout(resolve, 10))

      // AgentService doesn't have getAgentProgress method
      // This test should verify agent is running instead
      // @ts-ignore - accessing private members for testing
      const hasAgent = service.runningAgents.has('task-1')
      expect(hasAgent).toBe(true)

      // Clean up
      await agentPromise
    })

    it('should return false when no agent is running', () => {
      // @ts-ignore - accessing private members for testing
      const hasAgent = service.runningAgents.has('non-existent')
      expect(hasAgent).toBe(false)
    })
  })

  describe('agent status', () => {
    beforeEach(() => {
      vi.mocked(mockStorage.getTask).mockResolvedValue(mockTask)
    })

    it('should return true when agent is running', async () => {
      // Start an agent
      const mockWorkspace = createMockWorkspace()
      vi.mocked(mockWorkspaceService.getOrCreateWorkspace).mockResolvedValue(mockWorkspace)

      const mockAgent = createMockAgent()
      // Make the agent run method take some time
      vi.mocked(mockAgent.run).mockImplementation(async function* () {
        await new Promise((resolve) => setTimeout(resolve, 100))
        yield {
          eventId: 'event-1',
          type: EventType.COMPLETION,
          taskId: 'task-1',
          timestamp: new Date(),
          agentType: 'claude-code',
          isError: false,
          payload: { message: 'Test completed' },
        }
      })
      vi.mocked(ClaudeCodeAgent).mockImplementation(() => mockAgent as any)

      const agentPromise = service.executeAgent('task-1', 'Test message', mockIteration)

      // Give it a moment to start
      await new Promise((resolve) => setTimeout(resolve, 10))

      // @ts-ignore - accessing private members for testing
      const hasAgent = service.runningAgents.has('task-1')
      expect(hasAgent).toBe(true)

      // Clean up
      await agentPromise
    })

    it('should return false when no agent exists', () => {
      // @ts-ignore - accessing private members for testing
      const hasAgent = service.runningAgents.has('non-existent')
      expect(hasAgent).toBe(false)
    })

    it('should return false when agent is not running', async () => {
      // Start an agent
      const mockWorkspace = createMockWorkspace()
      vi.mocked(mockWorkspaceService.getOrCreateWorkspace).mockResolvedValue(mockWorkspace)

      const mockAgent = createMockAgent()
      // Remove agent from map to simulate not running
      // @ts-ignore - accessing private members for testing
      service.runningAgents.delete('task-1')
      vi.mocked(ClaudeCodeAgent).mockImplementation(() => mockAgent as any)

      await service.executeAgent('task-1', 'Test message', mockIteration)

      // @ts-ignore - accessing private members for testing
      const hasAgent = service.runningAgents.has('task-1')
      expect(hasAgent).toBe(false)
    })
  })

  // listAvailableAgents method doesn't exist in AgentService

  describe('processAgentExecution', () => {
    beforeEach(() => {
      vi.mocked(mockStorage.getTask).mockResolvedValue(mockTask)
    })

    it('should update task status after successful completion', async () => {
      const mockWorkspace = createMockWorkspace()
      vi.mocked(mockWorkspaceService.getOrCreateWorkspace).mockResolvedValue(mockWorkspace)
      vi.mocked(mockStorage.getTask).mockResolvedValue(mockTask)

      const mockAgent = createMockAgent()
      vi.mocked(mockAgent.run).mockImplementation(async function* () {
        yield {
          eventId: 'event-1',
          type: EventType.COMPLETION,
          taskId: 'task-1',
          timestamp: new Date(),
          agentType: 'claude-code',
          isError: false,
          payload: {
            message: 'Task completed successfully',
            iteration: mockIteration,
          },
        }
      })
      vi.mocked(ClaudeCodeAgent).mockImplementation(() => mockAgent as any)

      await service.executeAgent('task-1', 'Test message', mockIteration)

      // Wait for async processing
      await new Promise((resolve) => setTimeout(resolve, 50))

      // Task status should be set back to 'open' after agent completes
      expect(mockStorage.saveTask).toHaveBeenCalledWith(
        expect.objectContaining({
          status: 'open',
        }),
      )
    })

    it('should handle stopped agent gracefully', async () => {
      const mockWorkspace = createMockWorkspace()
      vi.mocked(mockWorkspaceService.getOrCreateWorkspace).mockResolvedValue(mockWorkspace)

      const mockAgent = createMockAgent()
      vi.mocked(mockAgent.run).mockImplementation(async function* () {
        yield {
          eventId: 'event-1',
          type: EventType.STOP,
          taskId: 'task-1',
          timestamp: new Date(),
          agentType: 'claude-code',
          isError: false,
          payload: {
            message: 'Agent was stopped by user',
          },
        }
      })
      vi.mocked(ClaudeCodeAgent).mockImplementation(() => mockAgent as any)

      await service.executeAgent('task-1', 'Test message', mockIteration)

      // Wait for async processing
      await new Promise((resolve) => setTimeout(resolve, 10))

      expect(mockEventService.emitEvent).toHaveBeenCalledWith(
        expect.objectContaining({
          taskId: 'task-1',
          type: EventType.STOP,
          isError: false,
          payload: expect.objectContaining({
            message: 'Agent was stopped by user',
          }),
        }),
      )
    })
  })
})
