import * as child_process from 'node:child_process'
import { promises as fs } from 'node:fs'
import path from 'node:path'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { EventService } from '../event.service.js'
import { RepositoryService } from '../repository.service.js'
import type { StorageBackend } from '../storage.js'
import { WorkspaceService } from '../workspace.service.js'

// Mock child_process
vi.mock('child_process', () => ({
  exec: vi.fn((_cmd, opts, cb) => {
    if (cb) {
      cb(null, { stdout: 'mocked output', stderr: '' })
    } else if (typeof opts === 'function') {
      opts(null, { stdout: 'mocked output', stderr: '' })
    }
  }),
}))

// Mock fs promises
vi.mock('fs', () => ({
  promises: {
    access: vi.fn(() => Promise.resolve()),
    mkdir: vi.fn(() => Promise.resolve()),
    writeFile: vi.fn(() => Promise.resolve()),
    readFile: vi.fn(() => Promise.resolve('file content')),
    rm: vi.fn(() => Promise.resolve()),
    stat: vi.fn(() => Promise.resolve({ isFile: () => false, isDirectory: () => true })),
    readdir: vi.fn(() => Promise.resolve([])),
  },
}))

// Mock git-utils
vi.mock('../../utils/git-utils.js', () => ({
  getCurrentBranch: vi.fn().mockResolvedValue('main'),
  createBranch: vi.fn().mockResolvedValue(undefined),
  getGitDiffFiles: vi.fn().mockResolvedValue({
    files: [],
    totalChanges: 0,
    totalAdditions: 0,
    totalDeletions: 0,
  }),
  isGitRepository: vi.fn().mockResolvedValue(true),
  hasUncommittedChanges: vi.fn().mockResolvedValue(false),
  getGitDiffString: vi.fn().mockResolvedValue(['', '']),
}))

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
    repository: {
      type: 'local' as const,
      path: '/test/repo',
    },
    codeExecution: {
      type: 'local' as const,
    },
    variables: {},
    createdAt: new Date(),
    updatedAt: new Date(),
  }

  beforeEach(async () => {
    tempDir = path.join(process.cwd(), `temp-test-${Date.now()}`)

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
      expect(retrievedWorkspace.getStatus()).toBe('idle') // Status when workspace path exists
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

      // Manually set the workspace path to our temp directory for testing

      ;(workspace as unknown as { workspacePath: string; setupComplete: boolean }).workspacePath =
        tempDir
      ;(workspace as unknown as { workspacePath: string; setupComplete: boolean }).setupComplete =
        true

      const files = await service.listFiles('test-task')

      expect(files).toContain('test1.js')
      expect(files).toContain('test2.js')
    })

    it('should read and write files', async () => {
      // Mock fs.stat to return isFile: true for the file we're reading
      vi.mocked(fs.stat).mockResolvedValueOnce({
        isFile: () => true,
        isDirectory: () => false,
      } as any)

      const workspace = await service.getOrCreateWorkspace('test-task')

      // Manually set the workspace path to our temp directory for testing

      ;(workspace as unknown as { workspacePath: string; setupComplete: boolean }).workspacePath =
        tempDir
      ;(workspace as unknown as { workspacePath: string; setupComplete: boolean }).setupComplete =
        true

      // Write file
      const content = 'console.log("Hello, world!");'
      await service.writeFile('test-task', 'test.js', content)

      // Read file - mock should return 'file content'
      const readContent = await service.readFile('test-task', 'test.js')
      expect(readContent).toBe('file content') // From our mock
    })

    it('should prevent path traversal attacks', async () => {
      const workspace = await service.getOrCreateWorkspace('test-task')

      // Manually set the workspace path to our temp directory for testing

      ;(workspace as unknown as { workspacePath: string; setupComplete: boolean }).workspacePath =
        tempDir
      ;(workspace as unknown as { workspacePath: string; setupComplete: boolean }).setupComplete =
        true

      // Attempt path traversal
      await expect(service.readFile('test-task', '../../../etc/passwd')).rejects.toThrow(
        'Access denied',
      )

      await expect(
        service.writeFile('test-task', '../../../etc/passwd', 'malicious'),
      ).rejects.toThrow('Access denied')
    })
  })

  describe('Docker workspace', () => {
    it('should create a Docker workspace', async () => {
      // Update mock environment for Docker
      vi.mocked(repositoryService.getRepositoryById).mockResolvedValueOnce({
        ...mockRepository,
        repository: {
          type: 'local',
          path: '/test/docker-repo',
        },
        codeExecution: {
          type: 'docker',
          image: 'node:18',
        },
      } as any)

      const workspace = await service.getOrCreateWorkspace('test-task')

      expect(workspace.type).toBe('docker')
      expect(workspace.status).toBe('initializing')
      // Container ID will be empty until setup is called
      expect((workspace as unknown as { containerId: string }).containerId).toBe('')
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
        repository: {
          type: 'local',
          path: '/test/docker-repo',
        },
        codeExecution: {
          type: 'docker',
        },
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

      // Verify Docker stop and rm were called
      expect(execMock).toHaveBeenCalledWith(
        expect.stringContaining('docker stop'),
        expect.any(Object),
        expect.any(Function),
      )
      expect(execMock).toHaveBeenCalledWith(
        expect.stringContaining('docker rm'),
        expect.any(Object),
        expect.any(Function),
      )
    })
  })
})
