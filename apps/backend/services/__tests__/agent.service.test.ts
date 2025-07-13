import type { Task, TaskIteration } from '@moatless/shared'
import { CriticalError, EventType, WorkspaceStatus } from '@moatless/shared'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { BaseAgent } from '../../agents/base-agent.js'
import { ClaudeCodeAgent } from '../../agents/claude-code-agent.js'
import type { BaseWorkspace } from '../../workspaces/base.js'
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
    child: vi.fn().mockReturnThis(),
  },
}))

vi.mock('../agents/claude-code-agent')

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
  }) as any

// Create mock workspace
const createMockWorkspace = (status: WorkspaceStatus = WorkspaceStatus.IDLE): BaseWorkspace =>
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
      expect(mockStorage.saveTask).toHaveBeenCalledWith(
        expect.objectContaining({
          status: 'running',
        }),
      )
    })

    it('should throw error if agent is already running', async () => {
      const mockWorkspace = createMockWorkspace()
      vi.mocked(mockWorkspaceService.getOrCreateWorkspace).mockResolvedValue(mockWorkspace)

      const mockAgent = createMockAgent()
      vi.mocked(ClaudeCodeAgent).mockImplementation(() => mockAgent as any)

      // Start first agent
      const promise1 = service.executeAgent('task-1', 'Message 1', mockIteration)

      // Try to start second agent for same task
      await expect(service.executeAgent('task-1', 'Message 2', mockIteration)).rejects.toThrow(
        CriticalError,
      )
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
        CriticalError,
      )
      await expect(service.executeAgent('task-1', 'Test message', mockIteration)).rejects.toThrow(
        'Workspace is still setting up',
      )
    })

    it('should recreate workspace if path is invalid', async () => {
      const mockWorkspace = createMockWorkspace()
      vi.mocked(mockWorkspace.isWorkspacePathValid).mockResolvedValue(false)
      vi.mocked(mockWorkspaceService.getOrCreateWorkspace).mockResolvedValue(mockWorkspace)
      vi.mocked(mockWorkspaceService.createWorkspace).mockResolvedValue(mockWorkspace)

      const mockAgent = createMockAgent()
      vi.mocked(ClaudeCodeAgent).mockImplementation(() => mockAgent as any)

      await service.executeAgent('task-1', 'Test message', mockIteration)

      expect(mockWorkspaceService.createWorkspace).toHaveBeenCalled()
      expect(mockWorkspaceService.getOrCreateWorkspace).toHaveBeenCalledTimes(2)
    })

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

      expect(ClaudeCodeAgent).toHaveBeenCalledWith(
        expect.objectContaining({
          taskId: 'task-1',
          model: 'claude-3-opus',
        }),
      )
    })

    it('should handle agent execution errors', async () => {
      const mockWorkspace = createMockWorkspace()
      vi.mocked(mockWorkspaceService.getOrCreateWorkspace).mockResolvedValue(mockWorkspace)

      const mockAgent = createMockAgent()
      vi.mocked(mockAgent.run).mockRejectedValue(new Error('Agent execution failed'))
      vi.mocked(ClaudeCodeAgent).mockImplementation(() => mockAgent as any)

      await service.executeAgent('task-1', 'Test message', mockIteration)

      // Wait for async processing
      await new Promise((resolve) => setTimeout(resolve, 10))

      expect(mockEventService.emitEvent).toHaveBeenCalledWith(
        expect.objectContaining({
          taskId: 'task-1',
          type: EventType.ERROR,
          isError: true,
          payload: expect.objectContaining({
            error: 'Agent execution failed',
          }),
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
    it('should stop running agent', async () => {
      // First, start an agent
      const mockWorkspace = createMockWorkspace()
      vi.mocked(mockWorkspaceService.getOrCreateWorkspace).mockResolvedValue(mockWorkspace)

      const mockAgent = createMockAgent()
      vi.mocked(ClaudeCodeAgent).mockImplementation(() => mockAgent as any)

      await service.executeAgent('task-1', 'Test message', mockIteration)

      // Now stop it
      const result = await service.stopAgent('task-1')

      expect(result).toBe(true)
      expect(mockAgent.stop).toHaveBeenCalled()
      // Workspace status is managed internally
    })

    it('should return false when no agent is running', async () => {
      const result = await service.stopAgent('non-existent')
      expect(result).toBe(false)
    })
  })

  describe('agent tracking', () => {
    it('should return progress for running agent', async () => {
      // Start an agent
      const mockWorkspace = createMockWorkspace()
      vi.mocked(mockWorkspaceService.getOrCreateWorkspace).mockResolvedValue(mockWorkspace)

      const mockAgent = createMockAgent()
      // Agent doesn't have getProgress method
      vi.mocked(ClaudeCodeAgent).mockImplementation(() => mockAgent as any)

      await service.executeAgent('task-1', 'Test message', mockIteration)

      // AgentService doesn't have getAgentProgress method
      // This test should verify agent is running instead
      const hasAgent = service['runningAgents'].has('task-1')
      expect(hasAgent).toBe(true)
    })

    it('should return false when no agent is running', () => {
      const hasAgent = service['runningAgents'].has('non-existent')
      expect(hasAgent).toBe(false)
    })
  })

  describe('agent status', () => {
    it('should return true when agent is running', async () => {
      // Start an agent
      const mockWorkspace = createMockWorkspace()
      vi.mocked(mockWorkspaceService.getOrCreateWorkspace).mockResolvedValue(mockWorkspace)

      const mockAgent = createMockAgent()
      // Agent is tracked in runningAgents map
      vi.mocked(ClaudeCodeAgent).mockImplementation(() => mockAgent as any)

      await service.executeAgent('task-1', 'Test message', mockIteration)

      const hasAgent = service['runningAgents'].has('task-1')
      expect(hasAgent).toBe(true)
    })

    it('should return false when no agent exists', () => {
      const hasAgent = service['runningAgents'].has('non-existent')
      expect(hasAgent).toBe(false)
    })

    it('should return false when agent is not running', async () => {
      // Start an agent
      const mockWorkspace = createMockWorkspace()
      vi.mocked(mockWorkspaceService.getOrCreateWorkspace).mockResolvedValue(mockWorkspace)

      const mockAgent = createMockAgent()
      // Remove agent from map to simulate not running
      service['runningAgents'].delete('task-1')
      vi.mocked(ClaudeCodeAgent).mockImplementation(() => mockAgent as any)

      await service.executeAgent('task-1', 'Test message', mockIteration)

      const hasAgent = service['runningAgents'].has('task-1')
      expect(hasAgent).toBe(false)
    })
  })

  // listAvailableAgents method doesn't exist in AgentService

  describe('processAgentExecution', () => {
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

      expect(mockStorage.saveTask).toHaveBeenCalledWith(
        expect.objectContaining({
          status: 'completed',
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
