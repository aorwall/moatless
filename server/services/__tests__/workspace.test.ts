import * as child_process from 'child_process'
import { promises as fs } from 'fs'
import path from 'path'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { EnvironmentService } from '../environment.service.js'
import { EventService } from '../event.service.js'
import { FileStorage } from '../storage.js'
import { TaskStorageService } from '../task-storage.service.js'
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
  let storage: FileStorage
  let eventService: EventService
  let environmentService: EnvironmentService
  let taskStorageService: TaskStorageService
  let tempDir: string

  beforeEach(async () => {
    tempDir = path.join(process.cwd(), 'temp-test-' + Date.now())
    await fs.mkdir(tempDir, { recursive: true })

    storage = new FileStorage(tempDir)
    eventService = new EventService(storage)
    environmentService = new EnvironmentService(storage)
    taskStorageService = new TaskStorageService(storage)

    // Mock the validateRepository method to skip validation
    vi.spyOn(environmentService as any, 'validateRepository').mockResolvedValue(undefined)

    service = new WorkspaceService(environmentService, eventService, taskStorageService)
  })

  afterEach(async () => {
    await fs.rm(tempDir, { recursive: true, force: true })
    vi.restoreAllMocks()
  })

  describe('getOrCreateWorkspace', () => {
    it('should create a local workspace', async () => {
      // Create test environment
      const environment = await environmentService.createEnvironment({
        name: 'Test Environment',
        repository: {
          type: 'local',
          path: '/test/repo',
        },
        codeExecution: {
          type: 'local',
        },
      })

      // Create a test task
      const task = {
        taskId: 'test-task',
        createdAt: new Date(),
        lastMessageAt: new Date(),
        agentType: 'claude-code',
        environmentId: environment.id,
        status: 'active' as const,
        numTurns: 0,
        totalCostUsd: 0,
        iterations: [],
      }
      await taskStorageService.saveTask(task)

      // Mock exec to simulate git worktree creation
      const execMock = vi.mocked(child_process.exec)
      execMock.mockImplementation(((cmd: string, opts: any, cb?: any) => {
        const callback = cb || opts
        if (cmd.includes('git worktree add')) {
          callback(null, { stdout: 'worktree created', stderr: '' })
        } else {
          callback(null, { stdout: 'success', stderr: '' })
        }
      }) as any)

      const workspace = await service.getOrCreateWorkspace('test-task')

      expect(workspace.id).toBe('test-task')
      expect(workspace.type).toBe('local')
      expect(workspace.status).toBe('initializing')
      expect(workspace.environmentId).toBe(environment.id)
    })

    it('should persist and reuse workspace path', async () => {
      // Create test environment
      const environment = await environmentService.createEnvironment({
        name: 'Test Environment',
        repository: {
          type: 'local',
          path: '/test/repo',
          useGitWorktrees: true,
        },
        codeExecution: {
          type: 'local',
        },
      })

      // Create a test task
      const task = {
        taskId: 'test-task-persist',
        createdAt: new Date(),
        lastMessageAt: new Date(),
        agentType: 'claude-code',
        environmentId: environment.id,
        status: 'active' as const,
        numTurns: 0,
        totalCostUsd: 0,
        iterations: [],
      }
      await taskStorageService.saveTask(task)

      // Mock exec to simulate git worktree creation
      const execMock = vi.mocked(child_process.exec)
      execMock.mockImplementation(((_cmd: string, opts: any, cb?: any) => {
        const callback = cb || opts
        callback(null, { stdout: 'success', stderr: '' })
      }) as any)

      // Mock fs.access to simulate workspace exists
      vi.spyOn(fs, 'access').mockResolvedValue(undefined)

      // First creation
      const workspace1 = await service.getOrCreateWorkspace('test-task-persist')
      await service.setupWorkspace('test-task-persist')
      
      const workspacePath = workspace1.getWorkspacePath()
      expect(workspacePath).toBeTruthy()

      // Verify task was updated with workspace path
      const updatedTask = await taskStorageService.getTask('test-task-persist')
      expect(updatedTask?.workspacePath).toBe(workspacePath)

      // Clear active workspaces to simulate restart
      ;(service as any).activeWorkspaces.clear()

      // Second creation should use saved path
      const workspace2 = await service.getOrCreateWorkspace('test-task-persist')
      expect(workspace2.getWorkspacePath()).toBe(workspacePath)
    })

    it('should handle workspace setup failures', async () => {
      const environment = await environmentService.createEnvironment({
        name: 'Invalid Environment',
        repository: {
          type: 'local',
          path: '/non/existent/path',
        },
        codeExecution: {
          type: 'local',
        },
      })

      // Create a test task
      const task = {
        taskId: 'test-task-fail',
        createdAt: new Date(),
        lastMessageAt: new Date(),
        agentType: 'claude-code',
        environmentId: environment.id,
        status: 'active' as const,
        numTurns: 0,
        totalCostUsd: 0,
        iterations: [],
      }
      await taskStorageService.saveTask(task)

      // Mock exec to simulate failure
      const execMock = vi.mocked(child_process.exec)
      execMock.mockImplementation(((_cmd: string, opts: any, cb?: any) => {
        const callback = cb || opts
        callback(new Error('Git worktree creation failed'), null)
      }) as any)

      // Mock fs.access to simulate workspace doesn't exist
      vi.spyOn(fs, 'access').mockRejectedValue(new Error('File not found'))
      
      // Mock fs.mkdir to fail
      vi.spyOn(fs, 'mkdir').mockRejectedValue(new Error('ENOENT: no such file or directory'))

      await service.getOrCreateWorkspace('test-task-fail')

      // Setup should fail
      await expect(service.setupWorkspace('test-task-fail')).rejects.toThrow('ENOENT')
    })

    it('should return existing workspace if already created', async () => {
      const environment = await environmentService.createEnvironment({
        name: 'Test Environment',
        repository: {
          type: 'local',
          path: '/test/repo',
        },
        codeExecution: {
          type: 'local',
        },
      })

      // Create a test task
      const task = {
        taskId: 'test-task',
        createdAt: new Date(),
        lastMessageAt: new Date(),
        agentType: 'claude-code',
        environmentId: environment.id,
        status: 'active' as const,
        numTurns: 0,
        totalCostUsd: 0,
        iterations: [],
      }
      await taskStorageService.saveTask(task)

      // Mock successful worktree creation
      const execMock = vi.mocked(child_process.exec)
      execMock.mockImplementation(((_cmd: string, opts: any, cb?: any) => {
        const callback = cb || opts
        callback(null, { stdout: 'success', stderr: '' })
      }) as any)

      const workspace1 = await service.getOrCreateWorkspace('test-task')
      const workspace2 = await service.getOrCreateWorkspace('test-task')

      expect(workspace1).toBe(workspace2)
    })
  })

  describe('file operations', () => {
    it('should list files in workspace', async () => {
      // Create a local workspace
      const environment = await environmentService.createEnvironment({
        name: 'Test Environment',
        repository: {
          type: 'local',
          path: tempDir,
        },
        codeExecution: {
          type: 'local',
        },
      })

      // Create test files
      const testFile1 = path.join(tempDir, 'test1.js')
      const testFile2 = path.join(tempDir, 'test2.js')
      await fs.writeFile(testFile1, 'test content 1')
      await fs.writeFile(testFile2, 'test content 2')

      // Mock exec for worktree creation
      const execMock = vi.mocked(child_process.exec)
      execMock.mockImplementation(((_cmd: string, opts: any, cb?: any) => {
        const callback = cb || opts
        callback(null, { stdout: tempDir, stderr: '' })
      }) as any)

      // Create a test task
      const task = {
        taskId: 'test-task',
        createdAt: new Date(),
        lastMessageAt: new Date(),
        agentType: 'claude-code',
        environmentId: environment.id,
        status: 'active' as const,
        numTurns: 0,
        totalCostUsd: 0,
        iterations: [],
      }
      await taskStorageService.saveTask(task)

      const workspace = await service.getOrCreateWorkspace('test-task')

      // Manually set the workspace path to our temp directory for testing

      ;(workspace as any).workspacePath = tempDir
      await storage.saveWorkspace(workspace.toWorkspaceInfo())

      const files = await service.listFiles('test-task')

      expect(files).toContain('test1.js')
      expect(files).toContain('test2.js')
    })

    it('should read and write files', async () => {
      const environment = await environmentService.createEnvironment({
        name: 'Test Environment',
        repository: {
          type: 'local',
          path: tempDir,
        },
        codeExecution: {
          type: 'local',
        },
      })

      // Mock exec for worktree creation
      const execMock = vi.mocked(child_process.exec)
      execMock.mockImplementation(((_cmd: string, opts: any, cb?: any) => {
        const callback = cb || opts
        callback(null, { stdout: tempDir, stderr: '' })
      }) as any)

      // Create a test task
      const task = {
        taskId: 'test-task',
        createdAt: new Date(),
        lastMessageAt: new Date(),
        agentType: 'claude-code',
        environmentId: environment.id,
        status: 'active' as const,
        numTurns: 0,
        totalCostUsd: 0,
        iterations: [],
      }
      await taskStorageService.saveTask(task)

      const workspace = await service.getOrCreateWorkspace('test-task')

      // Manually set the workspace path to our temp directory for testing

      ;(workspace as any).workspacePath = tempDir
      await storage.saveWorkspace(workspace.toWorkspaceInfo())

      // Write file
      const content = 'console.log("Hello, world!");'
      await service.writeFile('test-task', 'test.js', content)

      // Read file
      const readContent = await service.readFile('test-task', 'test.js')
      expect(readContent).toBe(content)
    })

    it('should prevent path traversal attacks', async () => {
      const environment = await environmentService.createEnvironment({
        name: 'Test Environment',
        repository: {
          type: 'local',
          path: tempDir,
        },
        codeExecution: {
          type: 'local',
        },
      })

      // Mock exec for worktree creation
      const execMock = vi.mocked(child_process.exec)
      execMock.mockImplementation(((_cmd: string, opts: any, cb?: any) => {
        const callback = cb || opts
        callback(null, { stdout: tempDir, stderr: '' })
      }) as any)

      // Create a test task
      const task = {
        taskId: 'test-task',
        createdAt: new Date(),
        lastMessageAt: new Date(),
        agentType: 'claude-code',
        environmentId: environment.id,
        status: 'active' as const,
        numTurns: 0,
        totalCostUsd: 0,
        iterations: [],
      }
      await taskStorageService.saveTask(task)

      const workspace = await service.getOrCreateWorkspace('test-task')

      // Manually set the workspace path to our temp directory for testing

      ;(workspace as any).workspacePath = tempDir
      await storage.saveWorkspace(workspace.toWorkspaceInfo())

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
      const environment = await environmentService.createEnvironment({
        name: 'Docker Environment',
        repository: {
          type: 'github',
          url: 'https://github.com/test/repo',
          repoId: 'test/repo',
          branch: 'main',
        },
        codeExecution: {
          type: 'docker',
          image: 'node:18',
        },
      })

      // Mock exec for Docker commands
      const execMock = vi.mocked(child_process.exec)
      const containerId = 'abc123'

      execMock.mockImplementation(((cmd: string, opts: any, cb?: any) => {
        const callback = cb || opts
        if (cmd.includes('docker create')) {
          callback(null, { stdout: containerId, stderr: '' })
        } else {
          callback(null, { stdout: 'success', stderr: '' })
        }
      }) as any)

      // Create a test task
      const task = {
        taskId: 'test-task',
        createdAt: new Date(),
        lastMessageAt: new Date(),
        agentType: 'claude-code',
        environmentId: environment.id,
        status: 'active' as const,
        numTurns: 0,
        totalCostUsd: 0,
        iterations: [],
      }
      await taskStorageService.saveTask(task)

      const workspace = await service.getOrCreateWorkspace('test-task')

      expect(workspace.type).toBe('docker')
      expect(workspace.status).toBe('initializing')
      // Container ID will be empty until setup is called
      expect((workspace as any).containerId).toBe('')
    })
  })

  describe('workspace cleanup', () => {
    it('should cleanup local workspace with worktree', async () => {
      const environment = await environmentService.createEnvironment({
        name: 'Test Environment',
        repository: {
          type: 'local',
          path: tempDir,
          useGitWorktrees: true,
        },
        codeExecution: {
          type: 'local',
        },
      })

      // Create a test task
      const task = {
        taskId: 'test-task-cleanup-local',
        createdAt: new Date(),
        lastMessageAt: new Date(),
        agentType: 'claude-code',
        environmentId: environment.id,
        status: 'active' as const,
        numTurns: 0,
        totalCostUsd: 0,
        iterations: [],
      }
      await taskStorageService.saveTask(task)

      // Mock exec
      const execMock = vi.mocked(child_process.exec)
      execMock.mockImplementation(((_cmd: string, opts: any, cb?: any) => {
        const callback = cb || opts
        callback(null, { stdout: 'success', stderr: '' })
      }) as any)

      // Mock fs.access to simulate workspace exists
      vi.spyOn(fs, 'access').mockResolvedValue(undefined)

      await service.getOrCreateWorkspace('test-task-cleanup-local')
      await service.setupWorkspace('test-task-cleanup-local')
      await service.cleanupWorkspace('test-task-cleanup-local')

      // Verify workspace is removed from storage
      const _deletedWorkspace = await storage.getWorkspace('test-task-cleanup-local')
      expect(_deletedWorkspace).toBeNull()

      // In a real environment with git worktrees, this would call git worktree remove
      // But in our test environment without a real git repo, it falls back to direct deletion
      // The important part is that the workspace was removed from the active workspaces map
      expect(await service.getWorkspace('test-task-cleanup-local')).toBeNull()
    })

    it('should cleanup Docker workspace', async () => {
      const environment = await environmentService.createEnvironment({
        name: 'Docker Environment',
        repository: {
          type: 'github',
          url: 'https://github.com/test/repo',
          repoId: 'test/repo',
          branch: 'main',
        },
        codeExecution: {
          type: 'docker',
        },
      })

      // Create a test task
      const task = {
        taskId: 'test-task-cleanup-docker',
        createdAt: new Date(),
        lastMessageAt: new Date(),
        agentType: 'claude-code',
        environmentId: environment.id,
        status: 'active' as const,
        numTurns: 0,
        totalCostUsd: 0,
        iterations: [],
      }
      await taskStorageService.saveTask(task)

      // Mock exec
      const execMock = vi.mocked(child_process.exec)
      const containerId = 'abc123'

      execMock.mockImplementation(((cmd: string, opts: any, cb?: any) => {
        const callback = cb || opts
        if (cmd.includes('docker create')) {
          callback(null, { stdout: containerId, stderr: '' })
        } else {
          callback(null, { stdout: 'success', stderr: '' })
        }
      }) as any)

      await service.getOrCreateWorkspace('test-task-cleanup-docker')
      await service.setupWorkspace('test-task-cleanup-docker')
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
