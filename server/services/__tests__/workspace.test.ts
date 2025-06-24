import * as child_process from 'child_process'
import { promises as fs } from 'fs'
import path from 'path'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { EnvironmentService } from '../environment.js'
import { EventService } from '../event.js'
import { FileStorage } from '../storage.js'
import { WorkspaceService } from '../workspace.js'

// Mock child_process
vi.mock('child_process', () => ({
  exec: vi.fn((cmd, opts, cb) => {
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
}))

describe('WorkspaceService', () => {
  let service: WorkspaceService
  let storage: FileStorage
  let eventService: EventService
  let environmentService: EnvironmentService
  let tempDir: string

  beforeEach(async () => {
    tempDir = path.join(process.cwd(), 'temp-test-' + Date.now())
    await fs.mkdir(tempDir, { recursive: true })

    storage = new FileStorage(tempDir)
    eventService = new EventService(storage)
    environmentService = new EnvironmentService(storage)

    // Mock the validateRepository method to skip validation
    vi.spyOn(environmentService as any, 'validateRepository').mockResolvedValue(undefined)

    service = new WorkspaceService(storage, environmentService, eventService)
  })

  afterEach(async () => {
    await fs.rm(tempDir, { recursive: true, force: true })
  })

  describe('createWorkspace', () => {
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

      const workspace = await service.createWorkspace('test-task', environment.id)

      expect(workspace.id).toBe('test-task')
      expect(workspace.type).toBe('local')
      expect(workspace.status).toBe('ready')
      expect(workspace.environmentId).toBe(environment.id)
    })

    it('should handle workspace creation failures', async () => {
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

      // Mock exec to simulate failure
      const execMock = vi.mocked(child_process.exec)
      execMock.mockImplementation(((cmd: string, opts: any, cb?: any) => {
        const callback = cb || opts
        callback(new Error('Git worktree creation failed'), null)
      }) as any)

      await expect(service.createWorkspace('test-task', environment.id)).rejects.toThrow(
        'Failed to create local workspace',
      )
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

      // Mock successful worktree creation
      const execMock = vi.mocked(child_process.exec)
      execMock.mockImplementation(((cmd: string, opts: any, cb?: any) => {
        const callback = cb || opts
        callback(null, { stdout: 'success', stderr: '' })
      }) as any)

      const workspace1 = await service.createWorkspace('test-task', environment.id)
      const workspace2 = await service.createWorkspace('test-task', environment.id)

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
      execMock.mockImplementation(((cmd: string, opts: any, cb?: any) => {
        const callback = cb || opts
        callback(null, { stdout: tempDir, stderr: '' })
      }) as any)

      const workspace = await service.createWorkspace('test-task', environment.id)

      // Manually set the workspace path to our temp directory for testing
      ;(workspace as any).path = tempDir
      await storage.saveWorkspace(workspace)

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
      execMock.mockImplementation(((cmd: string, opts: any, cb?: any) => {
        const callback = cb || opts
        callback(null, { stdout: tempDir, stderr: '' })
      }) as any)

      const workspace = await service.createWorkspace('test-task', environment.id)

      // Manually set the workspace path to our temp directory for testing
      ;(workspace as any).path = tempDir
      await storage.saveWorkspace(workspace)

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
      execMock.mockImplementation(((cmd: string, opts: any, cb?: any) => {
        const callback = cb || opts
        callback(null, { stdout: tempDir, stderr: '' })
      }) as any)

      const workspace = await service.createWorkspace('test-task', environment.id)

      // Manually set the workspace path to our temp directory for testing
      ;(workspace as any).path = tempDir
      await storage.saveWorkspace(workspace)

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
        },
        codeExecution: {
          type: 'docker',
          config: {
            image: 'node:18',
            workdir: '/app',
          },
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

      const workspace = await service.createWorkspace('test-task', environment.id)

      expect(workspace.type).toBe('docker')
      expect(workspace.status).toBe('ready')
      expect((workspace as any).containerId).toBe(containerId)
      expect((workspace as any).image).toBe('node:18')
    })
  })

  describe('workspace cleanup', () => {
    it('should cleanup local workspace with worktree', async () => {
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

      // Mock exec
      const execMock = vi.mocked(child_process.exec)
      execMock.mockImplementation(((cmd: string, opts: any, cb?: any) => {
        const callback = cb || opts
        callback(null, { stdout: 'success', stderr: '' })
      }) as any)

      const workspace = await service.createWorkspace('test-task', environment.id)
      await service.cleanupWorkspace('test-task')

      // Verify workspace is removed from storage
      const deletedWorkspace = await storage.getWorkspace('test-task')
      expect(deletedWorkspace).toBeNull()

      // Verify git worktree remove was called
      expect(execMock).toHaveBeenCalledWith(
        expect.stringContaining('git worktree remove'),
        expect.any(Object),
        expect.any(Function),
      )
    })

    it('should cleanup Docker workspace', async () => {
      const environment = await environmentService.createEnvironment({
        name: 'Docker Environment',
        repository: {
          type: 'github',
          url: 'https://github.com/test/repo',
        },
        codeExecution: {
          type: 'docker',
        },
      })

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

      await service.createWorkspace('test-task', environment.id)
      await service.cleanupWorkspace('test-task')

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
