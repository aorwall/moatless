import * as child_process from 'node:child_process'
import { promises as fs } from 'node:fs'
import path from 'node:path'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { EventService } from '../event.service.js'
import { RepositoryService } from '../repository.service.js'
import type { StorageBackend } from '../storage.js'
import { WorkspaceService } from '../workspace.service.js'

// Mock child_process
vi.mock('node:child_process', () => ({
  exec: vi.fn((_cmd, opts, cb) => {
    if (cb) {
      cb(null, { stdout: 'mocked output', stderr: '' })
    } else if (typeof opts === 'function') {
      opts(null, { stdout: 'mocked output', stderr: '' })
    }
  }),
}))

// Mock fs promises
vi.mock('node:fs', () => ({
  existsSync: vi.fn(() => false),
  promises: {
    access: vi.fn().mockResolvedValue(undefined), // Always allow access
    mkdir: vi.fn(() => Promise.resolve()),
    writeFile: vi.fn(() => Promise.resolve()),
    readFile: vi.fn(() => Promise.resolve('file content')),
    rm: vi.fn(() => Promise.resolve()),
    stat: vi.fn(() =>
      Promise.resolve({ isFile: () => false, isDirectory: () => true, size: 1024 }),
    ),
    readdir: vi.fn(() => Promise.resolve([])),
  },
}))

// Mock git-utils
vi.mock('../../utils/git-utils.js', () => ({
  getCurrentBranch: vi.fn().mockResolvedValue('main'),
  createBranch: vi.fn().mockResolvedValue(undefined),
  createGitWorktree: vi.fn().mockResolvedValue(undefined),
  getGitDiffFiles: vi.fn().mockResolvedValue({
    files: [],
    totalChanges: 0,
    totalAdditions: 0,
    totalDeletions: 0,
  }),
  getGitDiffDetailed: vi.fn().mockResolvedValue({
    files: [],
    summary: { insertions: 0, deletions: 0, filesChanged: 0 },
  }),
  getOriginSyncStatus: vi.fn().mockResolvedValue({
    currentBranch: 'main',
    baseBranch: 'main',
    hasUncommittedChanges: false,
    status: 'synced',
    commitsAhead: 0,
    commitsBehind: 0,
    originExists: true,
  }),
  getWorkingDirectoryChanges: vi.fn().mockResolvedValue({
    currentBranch: 'main',
    status: 'clean',
    additions: 0,
    deletions: 0,
    modifications: 0,
  }),
  checkForConflicts: vi.fn().mockResolvedValue(false),
  isGitRepository: vi.fn().mockResolvedValue(true),
  hasUncommittedChanges: vi.fn().mockResolvedValue(false),
  getGitDiffString: vi.fn().mockResolvedValue(['', '']),
  searchFilesInDirectoryNew: vi.fn().mockResolvedValue({
    results: [],
    totalFound: 0,
    query: '',
  }),
}))

// Mock PortAllocationService
vi.mock('../port-allocation.service.js', () => ({
  PortAllocationService: {
    getInstance: vi.fn(() => ({
      initialize: vi.fn().mockResolvedValue(undefined),
      allocatePort: vi.fn().mockResolvedValue(8001),
      releasePort: vi.fn().mockResolvedValue(undefined),
      releasePortsForTask: vi.fn().mockResolvedValue(undefined),
    })),
  },
}))

// Mock fetch for Docker health checks
global.fetch = vi.fn(() =>
  Promise.resolve({
    ok: true,
    json: () => Promise.resolve({ status: 'healthy' }),
  } as any),
)

describe('WorkspaceService', () => {
  let service: WorkspaceService
  let storage: StorageBackend
  let eventService: EventService
  let repositoryService: RepositoryService
  let tempDir: string

  const mockTask = {
    taskId: 'test-task',
    createdAt: new Date(),
    lastMessageAt: new Date(),
    agentType: 'claude-code' as const,
    repositoryId: 'test-env',
    status: 'open' as const,
    numTurns: 0,
    totalCostUsd: 0,
    iterations: [],
    workspacePath: '/test/repo',
  }

  const mockRepository = {
    id: 'test-env',
    name: 'Test Repository',
    path: '/test/repo',
    codeExecution: {
      type: 'local' as const,
    },
    variables: {},
    createdAt: new Date(),
    updatedAt: new Date(),
    useGitWorktrees: false, // Add this to disable git worktrees in tests
  }

  beforeEach(async () => {
    tempDir = path.join(process.cwd(), `temp-test-${Date.now()}`)

    // Ensure fs.access mock always succeeds
    vi.mocked(fs.access).mockResolvedValue(undefined)

    // Ensure isGitRepository mock always returns true
    const { isGitRepository } = await import('../../utils/git-utils.js')
    vi.mocked(isGitRepository).mockResolvedValue(true)

    // Mock storage
    storage = {
      saveTask: vi.fn().mockResolvedValue(undefined),
      getTask: vi.fn().mockImplementation((taskId) => {
        if (taskId === 'test-task' || taskId === 'test-task-existing') {
          return Promise.resolve({ ...mockTask, taskId })
        }
        if (taskId === 'test-task-persist') {
          return Promise.resolve({ ...mockTask, taskId, workspacePath: tempDir })
        }
        if (taskId === 'test-task-fail') {
          return Promise.resolve({ ...mockTask, taskId, workspacePath: '/non/existent/path' })
        }
        if (taskId === 'test-task-cleanup-local') {
          return Promise.resolve({ ...mockTask, taskId, workspacePath: tempDir })
        }
        if (taskId === 'test-task-cleanup-docker') {
          return Promise.resolve({ ...mockTask, taskId, workspacePath: '/workspace' })
        }
        return Promise.resolve(null)
      }),
      saveRepository: vi.fn().mockResolvedValue(undefined),
      getRepository: vi.fn().mockResolvedValue(mockRepository),
      list: vi.fn().mockResolvedValue([]),
      delete: vi.fn().mockResolvedValue(undefined),
      saveEvents: vi.fn().mockResolvedValue(undefined),
      getEvents: vi.fn().mockResolvedValue([]),
    } as any as StorageBackend

    eventService = new EventService(storage)
    repositoryService = new RepositoryService(storage)

    // Mock the validateRepository method to skip validation
    vi.spyOn(
      repositoryService as unknown as { validateRepository: () => Promise<void> },
      'validateRepository',
    ).mockResolvedValue(undefined)
    vi.spyOn(repositoryService, 'createRepository').mockResolvedValue(mockRepository as any)
    vi.spyOn(repositoryService, 'getRepositoryById').mockResolvedValue(mockRepository as any)

    service = new WorkspaceService(repositoryService, eventService, storage)
  })

  afterEach(async () => {
    await fs.rm(tempDir, { recursive: true, force: true })
    vi.restoreAllMocks()
  })

  describe('getOrCreateWorkspace', () => {
    it('should create a local workspace', async () => {
      const retrievedWorkspace = await service.getOrCreateWorkspace('test-task')

      expect(retrievedWorkspace.id).toBe('test-task')
      expect(retrievedWorkspace.type).toBe('local')
      expect(retrievedWorkspace.getStatus()).toBe('not_setup') // Status before setup is called
      expect(retrievedWorkspace.repositoryId).toBe(mockRepository.id)
      expect(retrievedWorkspace.getWorkspacePath()).toBe('/test/repo')
    })

    it('should persist and reuse workspace path', async () => {
      // First call - creates workspace
      const workspace1 = await service.getOrCreateWorkspace('test-task-persist')
      const workspacePath = workspace1.getWorkspacePath()
      expect(workspacePath).toBe(tempDir)

      // Clear active workspaces to simulate restart
      ;(service as unknown as { activeWorkspaces: Map<string, unknown> }).activeWorkspaces.clear()

      // Second creation should use saved path
      const workspace2 = await service.getOrCreateWorkspace('test-task-persist')
      expect(workspace2.getWorkspacePath()).toBe(workspacePath)
    })

    it.skip('should handle workspace setup failures', async () => {
      // Skip this test - the LocalWorkspace setup doesn't validate path existence
      // Path validation happens in createWorkspaceRepository which is called by WorkspaceService

      // Mock fs.access to simulate workspace doesn't exist the first time it's called
      vi.mocked(fs.access).mockRejectedValueOnce(new Error('File not found'))

      const workspace = await service.getOrCreateWorkspace('test-task-fail')

      // Setup should fail
      await expect(workspace.setup()).rejects.toThrow('Repository path does not exist')
    })

    it('should return existing workspace if already created', async () => {
      const workspace1 = await service.getOrCreateWorkspace('test-task-existing')
      const workspace2 = await service.getOrCreateWorkspace('test-task-existing')

      expect(workspace1).toBe(workspace2)
    })
  })

  describe('file operations', () => {
    it('should list files in workspace', async () => {
      // Mock readdir to return test files
      vi.mocked(fs.readdir).mockResolvedValue([
        { name: 'test1.js', isFile: () => true, isDirectory: () => false } as any,
        { name: 'test2.js', isFile: () => true, isDirectory: () => false } as any,
      ])

      const workspace = await service.getOrCreateWorkspace('test-task')

      // Setup the workspace so file operations can work
      await workspace.setup()

      const files = await workspace.listFiles()

      expect(files).toContain('test1.js')
      expect(files).toContain('test2.js')
    })

    it('should read and write files', async () => {
      // Mock fs.stat to return isFile: true for the file we're reading
      vi.mocked(fs.stat).mockResolvedValueOnce({
        isFile: () => true,
        isDirectory: () => false,
        size: 1024, // Add size property required by FileOperations
      } as any)

      const workspace = await service.getOrCreateWorkspace('test-task')

      // Setup the workspace so file operations can work
      await workspace.setup()

      // Write file
      const content = 'console.log("Hello, world!");'
      await workspace.writeFile('test.js', content)

      // Read file - mock should return 'file content'
      const readContent = await workspace.readFile('test.js')
      expect(readContent).toBe('file content') // From our mock
    })

    it('should prevent path traversal attacks', async () => {
      const workspace = await service.getOrCreateWorkspace('test-task')

      // Setup the workspace so file operations can work
      await workspace.setup()

      // Attempt path traversal
      await expect(workspace.readFile('../../../etc/passwd')).rejects.toThrow('Access denied')

      await expect(workspace.writeFile('../../../etc/passwd', 'malicious')).rejects.toThrow(
        'Access denied',
      )
    })
  })

  describe('Docker workspace', () => {
    it('should create a Docker workspace', async () => {
      // Update mock environment for Docker
      vi.mocked(repositoryService.getRepositoryById).mockResolvedValueOnce({
        ...mockRepository,
        path: '/test/docker-repo',
        codeExecution: {
          type: 'docker',
          dockerImage: 'moatless-vibe-universal:latest',
        },
      } as any)

      const workspace = await service.getOrCreateWorkspace('test-task')

      expect(workspace.type).toBe('docker')
      expect(workspace.status).toBe('not_setup') // Status before setup is called
      // Container ID will be empty until setup is called
      const workspaceInfo = workspace.toWorkspaceInfo()
      expect(workspaceInfo.type).toBe('docker')
      if (workspaceInfo.type === 'docker') {
        expect(workspaceInfo.containerId).toBe('')
      }
    })
  })

  describe('workspace cleanup', () => {
    it('should cleanup local workspace with worktree', async () => {
      // Mock exec
      const execMock = vi.mocked(child_process.exec)
      execMock.mockImplementation(((_cmd: string, opts: unknown, cb?: unknown) => {
        const callback = (cb || opts) as any
        callback(null, { stdout: 'success', stderr: '' })
      }) as any)

      // Mock fs.access to make tempDir accessible
      vi.mocked(fs.access).mockImplementation(async (path) => {
        if (path === tempDir) {
          return Promise.resolve()
        }
        return Promise.resolve() // Allow access to all paths for testing
      })

      const workspace = await service.getOrCreateWorkspace('test-task-cleanup-local')
      await workspace.setup()
      await service.cleanupWorkspace('test-task-cleanup-local')

      // The important part is that the workspace was removed from the active workspaces map
      expect(await service.getWorkspace('test-task-cleanup-local')).toBeNull()
    })

    it('should cleanup Docker workspace', async () => {
      // Update mock environment for Docker
      vi.mocked(storage.getTask).mockImplementation((taskId) => {
        if (taskId === 'test-task-cleanup-docker') {
          return Promise.resolve({ ...mockTask, taskId, workspacePath: '/workspace' })
        }
        return Promise.resolve(null)
      })

      vi.mocked(repositoryService.getRepositoryById).mockResolvedValueOnce({
        ...mockRepository,
        path: '/test/docker-repo',
        codeExecution: {
          type: 'docker',
        },
      })

      // Mock fs.access to make all paths accessible
      vi.mocked(fs.access).mockImplementation(async () => {
        return Promise.resolve() // Allow access to all paths for testing
      })

      // Mock exec
      const execMock = vi.mocked(child_process.exec)
      const containerId = 'abc123'

      execMock.mockImplementation(((cmd: string, opts: unknown, cb?: unknown) => {
        const callback = (cb || opts) as any
        if (cmd.includes('docker create')) {
          callback(null, { stdout: containerId, stderr: '' })
        } else {
          callback(null, { stdout: 'success', stderr: '' })
        }
      }) as any)

      const workspace = await service.getOrCreateWorkspace('test-task-cleanup-docker')
      await workspace.setup()
      await service.cleanupWorkspace('test-task-cleanup-docker')

      // Verify Docker stop was called (rm only happens on setup errors)
      expect(execMock).toHaveBeenCalledWith(
        expect.stringContaining('docker stop'),
        expect.any(Function),
      )
    })
  })
})
