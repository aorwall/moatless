import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { CriticalError } from '../../../shared/constants/errors.js'
import { WorkspaceStatus } from '../../../shared/types/workspace.js'
import { LocalWorkspace } from '../../workspaces/local.js'
import { AgentService } from '../agent.service.js'
import type { EventService } from '../event.service.js'
import type { StorageBackend } from '../storage.js'
import type { TaskService } from '../task.service.js'
import type { WorkspaceService } from '../workspace.service.js'

// Mock dependencies
vi.mock('../../utils/logger.js', () => ({
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
  createTaskLogger: vi.fn(() => ({
    info: vi.fn(),
    error: vi.fn(),
    warn: vi.fn(),
    debug: vi.fn(),
  })),
}))

describe('AgentService - Workspace Recreation', () => {
  let agentService: AgentService
  let mockStorage: StorageBackend
  let mockEventService: EventService
  let mockWorkspaceService: WorkspaceService
  let mockWorkspace: LocalWorkspace

  const taskId = 'test-task-123'
  const testWorkspacePath = '/tmp/test-workspace-123'
  const mockEnvironment = {
    id: 'env-123',
    name: 'Test Environment',
    description: 'Test environment',
    repository: {
      type: 'local' as const,
      path: '/tmp/test-repo',
      useGitWorktrees: true,
    },
    codeExecution: {
      type: 'local' as const,
      setupCommands: 'pnpm install',
    },
  }

  beforeEach(() => {
    // Setup mock storage
    mockStorage = {
      getTask: vi.fn().mockResolvedValue({
        taskId,
        environmentId: 'env-123',
        status: 'running',
        iterations: [],
      }),
      saveTask: vi.fn(),
    } as unknown as TaskService

    // Setup mock event service
    mockEventService = {
      emitEvent: vi.fn(),
    } as unknown as EventService

    // Setup mock workspace
    mockWorkspace = new LocalWorkspace(
      taskId,
      mockEnvironment,
      mockEventService,
      undefined,
      undefined,
      testWorkspacePath,
    )

    // Mock workspace methods
    vi.spyOn(mockWorkspace, 'getStatus').mockReturnValue(WorkspaceStatus.IDLE)
    vi.spyOn(mockWorkspace, 'isWorkspacePathValid').mockResolvedValue(false)
    vi.spyOn(mockWorkspace, 'getWorkspacePath').mockReturnValue(testWorkspacePath)
    vi.spyOn(mockWorkspace, 'setup').mockResolvedValue(undefined)

    // Setup mock workspace service
    mockWorkspaceService = {
      getOrCreateWorkspace: vi.fn().mockResolvedValue(mockWorkspace),
      getWorkspace: vi.fn().mockResolvedValue(mockWorkspace),
      setupWorkspace: vi.fn().mockResolvedValue(undefined),
    } as unknown as WorkspaceService

    agentService = new AgentService(mockStorage, mockEventService, mockWorkspaceService)
  })

  afterEach(() => {
    vi.clearAllMocks()
  })

  it('should detect missing workspace path and trigger recreation', async () => {
    const iteration = {
      iterationNumber: 1,
      startTime: new Date(),
      status: 'active' as const,
      messageCount: 1,
      costUsd: 0,
      initialMessage: 'test message',
    }

    // Mock workspace path doesn't exist
    vi.spyOn(mockWorkspace, 'isWorkspacePathValid').mockResolvedValue(false)

    // Mock the recreated workspace as a new LocalWorkspace instance
    const recreatedWorkspace = new LocalWorkspace(
      taskId,
      mockEnvironment,
      mockEventService,
      undefined,
      undefined,
      testWorkspacePath,
    )
    vi.spyOn(recreatedWorkspace, 'getStatus').mockReturnValue(WorkspaceStatus.IDLE)
    vi.spyOn(recreatedWorkspace, 'isWorkspacePathValid').mockResolvedValue(true)

    // Setup getOrCreateWorkspace to return recreated workspace after setupWorkspace is called
    let setupWorkspaceCalled = false
    vi.mocked(mockWorkspaceService.setupWorkspace).mockImplementation(async () => {
      setupWorkspaceCalled = true
    })

    vi.mocked(mockWorkspaceService.getOrCreateWorkspace).mockImplementation(async () => {
      if (setupWorkspaceCalled) {
        return recreatedWorkspace
      }
      return mockWorkspace
    })

    vi.mocked(mockWorkspaceService.getWorkspace).mockResolvedValue(recreatedWorkspace)

    // Mock agent creation
    const mockAgent = {
      run: vi.fn().mockImplementation(async function* () {
        yield { type: 'test', payload: {} }
      }),
      getSessionId: vi.fn(),
      stop: vi.fn(),
    }
    vi.spyOn(
      agentService as unknown as { createAgent: () => Promise<unknown> },
      'createAgent',
    ).mockResolvedValue(mockAgent)

    // Execute agent - should detect missing path and recreate
    await agentService.executeAgent(taskId, 'test message', iteration)

    // Verify workspace validation was checked
    expect(mockWorkspace.isWorkspacePathValid).toHaveBeenCalled()

    // Verify workspace setup was called for recreation
    expect(mockWorkspaceService.setupWorkspace).toHaveBeenCalledWith(
      taskId,
      iteration.iterationNumber,
    )

    // Verify getOrCreateWorkspace was called again after recreation
    expect(mockWorkspaceService.getOrCreateWorkspace).toHaveBeenCalledTimes(2)
  })

  it('should throw error if workspace recreation fails', async () => {
    const iteration = {
      iterationNumber: 1,
      startTime: new Date(),
      status: 'active' as const,
      messageCount: 1,
      costUsd: 0,
      initialMessage: 'test message',
    }

    // Mock workspace path doesn't exist
    vi.spyOn(mockWorkspace, 'isWorkspacePathValid').mockResolvedValue(false)

    // Mock recreateWorkspace to throw timeout error
    vi.spyOn(
      agentService as unknown as { recreateWorkspace: () => Promise<void> },
      'recreateWorkspace',
    ).mockRejectedValue(new CriticalError('Workspace recreation timeout', taskId, iteration))

    // Execute agent - should fail due to workspace recreation timeout
    await expect(agentService.executeAgent(taskId, 'test message', iteration)).rejects.toThrow(
      'Workspace recreation timeout',
    )

    // Verify workspace setup was attempted via recreateWorkspace
    expect(
      (agentService as unknown as { recreateWorkspace: () => Promise<void> }).recreateWorkspace,
    ).toHaveBeenCalledWith(taskId, iteration)
  }, 10000)

  it('should handle workspace path that exists correctly', async () => {
    const iteration = {
      iterationNumber: 1,
      startTime: new Date(),
      status: 'active' as const,
      messageCount: 1,
      costUsd: 0,
      initialMessage: 'test message',
    }

    // Mock workspace path exists
    vi.spyOn(mockWorkspace, 'isWorkspacePathValid').mockResolvedValue(true)

    // Mock agent creation
    const mockAgent = {
      run: vi.fn().mockImplementation(async function* () {
        yield { type: 'test', payload: {} }
      }),
      getSessionId: vi.fn(),
      stop: vi.fn(),
    }
    vi.spyOn(
      agentService as unknown as { createAgent: () => Promise<unknown> },
      'createAgent',
    ).mockResolvedValue(mockAgent)

    // Execute agent - should proceed without recreation
    await agentService.executeAgent(taskId, 'test message', iteration)

    // Verify workspace setup was NOT called
    expect(mockWorkspaceService.setupWorkspace).not.toHaveBeenCalled()

    // Verify getOrCreateWorkspace was called only once
    expect(mockWorkspaceService.getOrCreateWorkspace).toHaveBeenCalledTimes(1)
  })
})
