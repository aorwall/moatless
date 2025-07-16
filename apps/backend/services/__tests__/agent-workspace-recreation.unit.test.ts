import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { cleanupTestDataDir, setupTestDataDir } from '../../test/helpers/data-dir.helper.js'
import {
  createMockDockerWorkspace,
  createMockWorkspace,
  createMockWorkspaceWithInvalidPath,
} from '../../test/mocks/workspace.mock.js'
import type { Workspace } from '../../workspaces/workspace.js'
import { AgentService } from '../agent.service.js'
import type { EventService } from '../event.service.js'
import type { StorageBackend } from '../storage.js'
import type { WorkspaceService } from '../workspace.service.js'

describe('AgentService - Workspace Recreation', () => {
  let agentService: AgentService
  let mockStorage: StorageBackend
  let mockEventService: EventService
  let mockWorkspaceService: WorkspaceService
  let mockWorkspace: Workspace

  const taskId = 'test-task-123'
  const testWorkspacePath = '/tmp/test-workspace-123'
  const mockRepository = {
    id: 'env-123',
    name: 'Test Repository',
    description: 'Test repository',
    path: '/tmp/test-repo',
    useGitWorktrees: true,
    codeExecution: {
      type: 'local' as const,
      setupCommands: ['pnpm install'],
    },
    variables: {},
    createdAt: new Date(),
    updatedAt: new Date(),
  }

  let tempDataDir: string

  beforeEach(async () => {
    // Setup test data directory
    tempDataDir = await setupTestDataDir('agent-workspace')

    // Setup mock storage
    mockStorage = {
      save: vi.fn(),
      get: vi.fn(),
      list: vi.fn(),
      delete: vi.fn(),
      exists: vi.fn(),
      getMetadata: vi.fn(),
      saveMetadata: vi.fn(),
      listMetadata: vi.fn(),
      deleteMetadata: vi.fn(),
      search: vi.fn(),
      bulkSave: vi.fn(),
      bulkDelete: vi.fn(),
      getTask: vi.fn().mockResolvedValue({
        taskId,
        repositoryId: 'env-123',
        status: 'running',
        iterations: [],
        agentType: 'claude-code',
      }),
      saveTask: vi.fn(),
    } as any

    // Setup mock event service
    mockEventService = {
      emitEvent: vi.fn(),
    } as unknown as EventService

    // Setup mock workspace using factory
    mockWorkspace = createMockWorkspaceWithInvalidPath({
      id: taskId,
      repositoryId: mockRepository.id,
      getWorkspacePath: vi.fn().mockReturnValue(testWorkspacePath),
    })

    // Setup mock workspace service
    mockWorkspaceService = {
      getOrCreateWorkspace: vi.fn().mockResolvedValue(mockWorkspace),
      getWorkspace: vi.fn().mockResolvedValue(mockWorkspace),
      setupWorkspace: vi.fn().mockResolvedValue(undefined),
      removeWorktree: vi.fn(),
    } as unknown as WorkspaceService

    agentService = new AgentService(mockStorage, mockEventService, mockWorkspaceService)
  })

  afterEach(async () => {
    vi.clearAllMocks()
    await cleanupTestDataDir(tempDataDir)
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
    vi.mocked(mockWorkspace.isWorkspacePathValid).mockResolvedValue(false)

    // Mock the recreated workspace with valid path
    const recreatedWorkspace = createMockWorkspace({
      id: taskId,
      repositoryId: mockRepository.id,
      getWorkspacePath: vi.fn().mockReturnValue(testWorkspacePath),
      isWorkspacePathValid: vi.fn().mockResolvedValue(true),
    })

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
    vi.mocked(mockWorkspace.isWorkspacePathValid).mockResolvedValue(false)

    // Mock setupWorkspace to succeed but workspace still not ready
    vi.mocked(mockWorkspaceService.setupWorkspace).mockResolvedValue(undefined)

    // Mock getWorkspace to return workspace that's still not ready
    const notReadyWorkspace = createMockWorkspace({
      id: taskId,
      repositoryId: mockRepository.id,
      getStatus: vi.fn().mockReturnValue('setting_up'),
      isWorkspacePathValid: vi.fn().mockResolvedValue(false),
    })
    vi.mocked(mockWorkspaceService.getWorkspace).mockResolvedValue(notReadyWorkspace)

    // Execute agent - should fail due to workspace recreation timeout
    await expect(agentService.executeAgent(taskId, 'test message', iteration)).rejects.toThrow(
      'Workspace recreation timeout',
    )

    // Verify workspace setup was attempted
    expect(mockWorkspaceService.setupWorkspace).toHaveBeenCalledWith(
      taskId,
      iteration.iterationNumber,
    )
  }, 35000) // Increase timeout for this test since it waits for recreation

  it('should handle workspace path that exists correctly', async () => {
    const iteration = {
      iterationNumber: 1,
      startTime: new Date(),
      status: 'active' as const,
      messageCount: 1,
      costUsd: 0,
      initialMessage: 'test message',
    }

    // Update mock workspace to have valid path
    const validWorkspace = createMockWorkspace({
      id: taskId,
      repositoryId: mockRepository.id,
      getWorkspacePath: vi.fn().mockReturnValue(testWorkspacePath),
      isWorkspacePathValid: vi.fn().mockResolvedValue(true),
    })

    vi.mocked(mockWorkspaceService.getOrCreateWorkspace).mockResolvedValue(validWorkspace)

    // Execute agent - should proceed without recreation
    await agentService.executeAgent(taskId, 'test message', iteration)

    // Verify workspace setup was NOT called
    expect(mockWorkspaceService.setupWorkspace).not.toHaveBeenCalled()

    // Verify getOrCreateWorkspace was called only once
    expect(mockWorkspaceService.getOrCreateWorkspace).toHaveBeenCalledTimes(1)
  })

  it('should handle workspace recreation with Docker environment', async () => {
    const iteration = {
      iterationNumber: 1,
      startTime: new Date(),
      status: 'active' as const,
      messageCount: 1,
      costUsd: 0,
      initialMessage: 'test message',
    }

    // Setup Docker workspace mock with invalid path
    const dockerWorkspace = createMockDockerWorkspace({
      id: taskId,
      repositoryId: mockRepository.id,
      isWorkspacePathValid: vi.fn().mockResolvedValue(false),
    })

    vi.mocked(mockWorkspaceService.getOrCreateWorkspace).mockResolvedValueOnce(dockerWorkspace)

    // Mock successful recreation
    const recreatedDockerWorkspace = createMockDockerWorkspace({
      id: taskId,
      repositoryId: mockRepository.id,
      isWorkspacePathValid: vi.fn().mockResolvedValue(true),
    })

    // Setup recreation flow
    let setupCalled = false
    vi.mocked(mockWorkspaceService.setupWorkspace).mockImplementation(async () => {
      setupCalled = true
    })

    vi.mocked(mockWorkspaceService.getOrCreateWorkspace).mockImplementation(async () => {
      if (setupCalled) {
        return recreatedDockerWorkspace
      }
      return dockerWorkspace
    })

    vi.mocked(mockWorkspaceService.getWorkspace).mockResolvedValue(recreatedDockerWorkspace)

    // Execute
    await agentService.executeAgent(taskId, 'test message', iteration)

    // Verify recreation happened
    expect(dockerWorkspace.isWorkspacePathValid).toHaveBeenCalled()
    expect(mockWorkspaceService.setupWorkspace).toHaveBeenCalledWith(
      taskId,
      iteration.iterationNumber,
    )
  })
})
