import { randomUUID } from 'node:crypto'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { CriticalError } from '@moatless/shared'
import type { Task } from '@moatless/shared/types/task'
import type { TaskIteration } from '@moatless/shared/types/task-events'
import { WorkspaceStatus } from '@moatless/shared/types/workspace'
import { AgentService } from '../agent.service'
import type { BaseAgent } from '../agents/base-agent'
import { ClaudeCodeAgent } from '../agents/claude-code-agent'
import type { EventService } from '../event.service'
import type { StorageBackend } from '../storage'
import type { WorkspaceService } from '../workspace.service'
import type { BaseWorkspace } from '../../workspaces/base'

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
  getEventsFromTo: vi.fn(),
  saveWorkspace: vi.fn(),
  getWorkspace: vi.fn(),
})

// Create mock event service
const createMockEventService = (): EventService => ({
  saveEvents: vi.fn(),
  getEvents: vi.fn(),
  getEventsFromTo: vi.fn(),
  streamEvent: vi.fn(),
  closeStream: vi.fn(),
  closeAllStreams: vi.fn(),
} as any)

// Create mock workspace service
const createMockWorkspaceService = (): WorkspaceService => ({
  getOrCreateWorkspace: vi.fn(),
  getWorkspace: vi.fn(),
  cleanupWorkspace: vi.fn(),
  getAllWorkspaces: vi.fn(),
  getWorkspaceStatus: vi.fn(),
  recreateWorkspace: vi.fn(),
} as any)

// Create mock workspace
const createMockWorkspace = (status: WorkspaceStatus = WorkspaceStatus.IDLE): BaseWorkspace =>
  ({
    getStatus: vi.fn().mockReturnValue(status),
    setStatus: vi.fn(),
    isWorkspacePathValid: vi.fn().mockResolvedValue(true),
    getWorkspacePath: vi.fn().mockReturnValue('/workspace/test'),
    getId: vi.fn().mockReturnValue('workspace-id'),
    resetWorkspace: vi.fn(),
    getEnvironment: vi.fn().mockReturnValue({
      id: 'env-1',
      name: 'Test Env',
      path: '/test/repo',
      codeExecution: {},
    }),
  }) as any

// Create mock agent
const createMockAgent = (): BaseAgent =>
  ({
    execute: vi.fn(),
    stop: vi.fn(),
    isRunning: vi.fn().mockReturnValue(false),
    getProgress: vi.fn().mockReturnValue({ completed: 0, total: 0 }),
  }) as any

describe('AgentService', () => {
  let service: AgentService
  let mockStorage: StorageBackend
  let mockEventService: EventService
  let mockWorkspaceService: WorkspaceService

  const mockTask: Task = {
    id: 'task-1',
    repositoryId: 'repo-1',
    title: 'Test Task',
    status: 'open',
    createdAt: new Date(),
    updatedAt: new Date(),
  }

  const mockIteration: TaskIteration = {
    iterationNumber: 1,
    message: 'Test message',
    timestamp: new Date(),
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
      vi.mocked(mockAgent.execute).mockResolvedValue({
        success: true,
        message: 'Test completed',
        iterations: [mockIteration],
      } as any)
      vi.mocked(ClaudeCodeAgent).mockImplementation(() => mockAgent as any)

      await service.executeAgent('task-1', 'Test message', mockIteration)

      expect(mockAgent.execute).toHaveBeenCalledWith(
        'Test message',
        expect.objectContaining({
          taskId: 'task-1',
          iterationNumber: 1,
        }),
        undefined,
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
      await expect(
        service.executeAgent('task-1', 'Message 2', mockIteration),
      ).rejects.toThrow(CriticalError)
      await expect(
        service.executeAgent('task-1', 'Message 2', mockIteration),
      ).rejects.toThrow('Agent is already running')

      // Clean up
      await promise1
    })

    it('should throw error if workspace is setting up', async () => {
      const mockWorkspace = createMockWorkspace(WorkspaceStatus.SETTING_UP)
      vi.mocked(mockWorkspaceService.getOrCreateWorkspace).mockResolvedValue(mockWorkspace)

      await expect(
        service.executeAgent('task-1', 'Test message', mockIteration),
      ).rejects.toThrow(CriticalError)
      await expect(
        service.executeAgent('task-1', 'Test message', mockIteration),
      ).rejects.toThrow('Workspace is still setting up')
    })

    it('should recreate workspace if path is invalid', async () => {
      const mockWorkspace = createMockWorkspace()
      vi.mocked(mockWorkspace.isWorkspacePathValid).mockResolvedValue(false)
      vi.mocked(mockWorkspaceService.getOrCreateWorkspace).mockResolvedValue(mockWorkspace)
      vi.mocked(mockWorkspaceService.recreateWorkspace).mockResolvedValue(undefined)

      const mockAgent = createMockAgent()
      vi.mocked(ClaudeCodeAgent).mockImplementation(() => mockAgent as any)

      await service.executeAgent('task-1', 'Test message', mockIteration)

      expect(mockWorkspaceService.recreateWorkspace).toHaveBeenCalledWith('task-1')
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
      vi.mocked(mockAgent.execute).mockRejectedValue(new Error('Agent execution failed'))
      vi.mocked(ClaudeCodeAgent).mockImplementation(() => mockAgent as any)

      await service.executeAgent('task-1', 'Test message', mockIteration)

      // Wait for async processing
      await new Promise((resolve) => setTimeout(resolve, 10))

      expect(mockEventService.streamEvent).toHaveBeenCalledWith(
        'task-1',
        expect.objectContaining({
          type: 'agent_error',
          error: 'Agent execution failed',
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

      expect(mockAgent.execute).toHaveBeenCalledWith(
        'Test message',
        expect.any(Object),
        resumeSessionId,
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
      expect(mockWorkspace.setStatus).toHaveBeenCalledWith(WorkspaceStatus.IDLE)
    })

    it('should return false when no agent is running', async () => {
      const result = await service.stopAgent('non-existent')
      expect(result).toBe(false)
    })
  })

  describe('getAgentProgress', () => {
    it('should return progress for running agent', async () => {
      // Start an agent
      const mockWorkspace = createMockWorkspace()
      vi.mocked(mockWorkspaceService.getOrCreateWorkspace).mockResolvedValue(mockWorkspace)

      const mockAgent = createMockAgent()
      vi.mocked(mockAgent.getProgress).mockReturnValue({
        completed: 5,
        total: 10,
        percentage: 50,
      })
      vi.mocked(ClaudeCodeAgent).mockImplementation(() => mockAgent as any)

      await service.executeAgent('task-1', 'Test message', mockIteration)

      // Get progress
      const progress = service.getAgentProgress('task-1')

      expect(progress).toEqual({
        completed: 5,
        total: 10,
        percentage: 50,
      })
    })

    it('should return null when no agent is running', () => {
      const progress = service.getAgentProgress('non-existent')
      expect(progress).toBeNull()
    })
  })

  describe('isAgentRunning', () => {
    it('should return true when agent is running', async () => {
      // Start an agent
      const mockWorkspace = createMockWorkspace()
      vi.mocked(mockWorkspaceService.getOrCreateWorkspace).mockResolvedValue(mockWorkspace)

      const mockAgent = createMockAgent()
      vi.mocked(mockAgent.isRunning).mockReturnValue(true)
      vi.mocked(ClaudeCodeAgent).mockImplementation(() => mockAgent as any)

      await service.executeAgent('task-1', 'Test message', mockIteration)

      const isRunning = service.isAgentRunning('task-1')
      expect(isRunning).toBe(true)
    })

    it('should return false when no agent exists', () => {
      const isRunning = service.isAgentRunning('non-existent')
      expect(isRunning).toBe(false)
    })

    it('should return false when agent is not running', async () => {
      // Start an agent
      const mockWorkspace = createMockWorkspace()
      vi.mocked(mockWorkspaceService.getOrCreateWorkspace).mockResolvedValue(mockWorkspace)

      const mockAgent = createMockAgent()
      vi.mocked(mockAgent.isRunning).mockReturnValue(false)
      vi.mocked(ClaudeCodeAgent).mockImplementation(() => mockAgent as any)

      await service.executeAgent('task-1', 'Test message', mockIteration)

      const isRunning = service.isAgentRunning('task-1')
      expect(isRunning).toBe(false)
    })
  })

  describe('listAvailableAgents', () => {
    it('should return list of available agents', () => {
      const agents = service.listAvailableAgents()

      expect(agents).toEqual([
        {
          id: 'claude-code',
          name: 'Claude Code Agent',
          description: 'Code generation and task execution with Claude',
          supported: true,
        },
      ])
    })
  })

  describe('processAgentExecution', () => {
    it('should update task status after successful completion', async () => {
      const mockWorkspace = createMockWorkspace()
      vi.mocked(mockWorkspaceService.getOrCreateWorkspace).mockResolvedValue(mockWorkspace)
      vi.mocked(mockStorage.getTask).mockResolvedValue(mockTask)

      const mockAgent = createMockAgent()
      vi.mocked(mockAgent.execute).mockResolvedValue({
        success: true,
        message: 'Completed',
        iterations: [mockIteration],
        finalResult: {
          type: 'completion',
          iteration: mockIteration,
          message: 'Task completed successfully',
        },
      } as any)
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
      vi.mocked(mockAgent.execute).mockResolvedValue({
        success: false,
        message: 'Agent stopped',
        iterations: [],
        wasStopped: true,
      } as any)
      vi.mocked(ClaudeCodeAgent).mockImplementation(() => mockAgent as any)

      await service.executeAgent('task-1', 'Test message', mockIteration)

      // Wait for async processing
      await new Promise((resolve) => setTimeout(resolve, 10))

      expect(mockEventService.streamEvent).toHaveBeenCalledWith(
        'task-1',
        expect.objectContaining({
          type: 'agent_stopped',
          message: 'Agent was stopped by user',
        }),
      )
    })
  })
})