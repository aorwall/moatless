import { promises as fs } from 'node:fs'
import type { RepositoryConfig, Task } from '@moatless/types'
import { TaskNotFoundError } from '@moatless/types'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { PreviewEnvironmentService } from '../preview-environment.service.js'
import { PreviewServer } from '../preview-server.js'
import type { StorageBackend } from '../storage.js'
import type { WorkspaceService } from '../workspace.service.js'

// Mock dependencies
vi.mock('../../utils/data-dir', () => ({
  getDataDir: vi.fn(() => '/test'),
}))

vi.mock('node:fs', () => ({
  promises: {
    mkdir: vi.fn(),
    readdir: vi.fn(),
    readFile: vi.fn(),
    writeFile: vi.fn(),
    unlink: vi.fn(),
    stat: vi.fn(),
  },
}))

vi.mock('node:child_process', () => ({
  execSync: vi.fn(),
  spawn: vi.fn(),
  exec: vi.fn((_cmd, callback) => callback?.(null, { stdout: '', stderr: '' }, null)),
}))

vi.mock('../preview-server')
vi.mock('../preview-proxy.service')
vi.mock('../../utils/logger', () => ({
  logger: {
    info: vi.fn(),
    error: vi.fn(),
    debug: vi.fn(),
    child: vi.fn(() => ({
      info: vi.fn(),
      error: vi.fn(),
      debug: vi.fn(),
      warn: vi.fn(),
    })),
  },
}))

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

// Create mock workspace service
const createMockWorkspaceService = (): WorkspaceService =>
  ({
    getOrCreateWorkspace: vi.fn(),
    getWorkspace: vi.fn(),
    cleanupWorkspace: vi.fn(),
    getAllWorkspaces: vi.fn(),
    getWorkspaceStatus: vi.fn(),
    recreateWorkspace: vi.fn(),
    canManageServer: vi.fn().mockResolvedValue(true),
  }) as any

describe('PreviewEnvironmentService', () => {
  let service: PreviewEnvironmentService
  let mockStorage: StorageBackend
  let mockWorkspaceService: WorkspaceService
  const mockFsMkdir = vi.mocked(fs.mkdir)
  const mockFsReaddir = vi.mocked(fs.readdir)
  const mockFsReadFile = vi.mocked(fs.readFile)
  const mockFsWriteFile = vi.mocked(fs.writeFile)
  const mockFsUnlink = vi.mocked(fs.unlink)

  beforeEach(() => {
    mockStorage = createMockStorage()
    mockWorkspaceService = createMockWorkspaceService()
    mockFsReaddir.mockResolvedValue([])
    service = new PreviewEnvironmentService(mockStorage, mockWorkspaceService)
  })

  afterEach(() => {
    vi.restoreAllMocks()
  })

  describe('initialization', () => {
    it('should create preview environments directory on initialization', async () => {
      mockFsReaddir.mockResolvedValue([])

      // Service is already created in beforeEach which triggers initialization
      // Just need to verify the call was made
      expect(mockFsMkdir).toHaveBeenCalledWith('/test/preview-environments', { recursive: true })
    })

    it('should load and validate existing preview environments on startup', async () => {
      const mockMetadata = {
        taskId: 'task-1',
        pid: 12345,
        port: 3000,
        command: 'npm run dev',
        workspacePath: '/workspace/task-1',
        startTime: new Date(),
        processStartTime: Date.now(),
      }

      mockFsReaddir.mockResolvedValue(['task-1.json'] as any)
      mockFsReadFile.mockResolvedValue(JSON.stringify(mockMetadata))

      const { execSync } = await import('node:child_process')
      vi.mocked(execSync).mockReturnValue(Buffer.from('node'))

      // Reinitialize service to trigger loading
      service = new PreviewEnvironmentService(mockStorage, mockWorkspaceService)
      await new Promise((resolve) => setTimeout(resolve, 50))

      expect(mockFsReaddir).toHaveBeenCalled()
      expect(mockFsReadFile).toHaveBeenCalledWith(expect.stringContaining('task-1.json'), 'utf-8')
    })
  })

  describe('startServer', () => {
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

    const mockRepository: RepositoryConfig = {
      id: 'repo-1',
      name: 'Test Repo',
      path: '/test/repo',
      codeExecution: {
        type: 'local',
        envVars: { NODE_ENV: 'development' },
      },
      previewConfig: {
        framework: 'vite',
        serverStartCommand: 'npm run dev',
        injectionStrategy: {
          method: 'html-injection',
          config: {
            indexPath: 'index.html',
            insertLocation: 'head',
          },
        },
      },
      createdAt: new Date(),
      updatedAt: new Date(),
    }

    const mockWorkspace = {
      getWorkspacePath: vi.fn().mockReturnValue('/workspace/task-1'),
      exists: vi.fn().mockResolvedValue(true),
      environmentConfig: mockRepository,
    }

    beforeEach(() => {
      vi.mocked(mockStorage.getTask).mockResolvedValue(mockTask)
      vi.mocked(mockStorage.getRepository).mockResolvedValue(mockRepository)
      vi.mocked(mockWorkspaceService.getOrCreateWorkspace).mockResolvedValue(mockWorkspace as any)
    })

    it('should start a preview server successfully', async () => {
      const mockHandler = {
        start: vi.fn().mockResolvedValue({ port: 3000, pid: 12345 }),
        isRunning: vi.fn().mockReturnValue(true),
        getMetrics: vi.fn().mockReturnValue({ uptimeSeconds: 10 }),
        getLogs: vi.fn().mockResolvedValue('Server started'),
      }

      vi.mocked(PreviewServer).mockImplementation(() => mockHandler as any)

      const result = await service.startServer('task-1')

      expect(result).toMatchObject({
        running: true,
        configured: true,
        url: 'http://localhost:3000',
        pid: 12345,
        uptimeSeconds: 0,
      })
      expect(mockHandler.start).toHaveBeenCalled()
      expect(mockFsWriteFile).toHaveBeenCalledWith(
        expect.stringContaining('task-1.json'),
        expect.any(String),
        'utf-8',
      )
    })

    it('should handle server start failure gracefully', async () => {
      const mockHandler = {
        start: vi.fn().mockRejectedValue(new Error('Port already in use')),
        getLogs: vi.fn().mockResolvedValue('Error: Port 3000 is already in use'),
      }

      vi.mocked(PreviewServer).mockImplementation(() => mockHandler as any)

      const result = await service.startServer('task-1')

      expect(result).toMatchObject({
        running: false,
        configured: true,
        uptimeSeconds: 0,
        error: 'Port already in use',
        logs: 'Error: Port 3000 is already in use',
      })
    })

    it('should start server without proxy', async () => {
      const mockHandler = {
        start: vi.fn().mockResolvedValue({ port: 3000, pid: 12345 }),
        isRunning: vi.fn().mockReturnValue(true),
        getMetrics: vi.fn().mockReturnValue({ uptimeSeconds: 10 }),
      }

      vi.mocked(PreviewServer).mockImplementation(() => mockHandler as any)

      const result = await service.startServer('task-1')

      expect(result).toMatchObject({
        running: true,
        configured: true,
        url: 'http://localhost:3000',
        pid: 12345,
      })
      // Proxy is not started in current implementation
    })

    it('should throw TaskNotFoundError when task not found', async () => {
      vi.mocked(mockStorage.getTask).mockResolvedValue(null)

      await expect(service.startServer('non-existent')).rejects.toThrow(TaskNotFoundError)
    })

    it('should handle missing repository gracefully', async () => {
      vi.mocked(mockStorage.getRepository).mockResolvedValue(null)
      vi.mocked(mockWorkspaceService.getOrCreateWorkspace).mockRejectedValue(
        new Error('Repository not found'),
      )

      await expect(service.startServer('task-1')).rejects.toThrow('Repository not found')
    })
  })

  describe('stopServer', () => {
    it('should stop running server successfully', async () => {
      const mockHandler = {
        stop: vi.fn().mockResolvedValue(true),
        isRunning: vi.fn().mockReturnValue(false),
      }

      // @ts-ignore - accessing private member for testing
      service.runningServers.set('task-1', mockHandler as any)

      const result = await service.stopServer('task-1')

      expect(result).toBe(true)
      expect(mockHandler.stop).toHaveBeenCalled()
      expect(mockFsUnlink).toHaveBeenCalledWith(expect.stringContaining('task-1.json'))
    })

    it('should stop proxy when stopping server', async () => {
      const mockHandler = {
        stop: vi.fn().mockResolvedValue(true),
      }

      const mockProxy = {
        stop: vi.fn(),
      }

      // @ts-ignore - accessing private members for testing
      service.runningServers.set('task-1', mockHandler as any)
      // @ts-ignore - accessing private members for testing
      service.runningProxies.set('task-1', mockProxy as any)

      const mockWorkspace = {
        getWorkspacePath: vi.fn().mockReturnValue('/workspace/task-1'),
      }
      vi.mocked(mockWorkspaceService.getOrCreateWorkspace).mockResolvedValue(mockWorkspace as any)

      await service.stopServer('task-1')

      expect(mockProxy.stop).toHaveBeenCalled()
    })

    it('should cleanup orphaned process when no handler found', async () => {
      const mockMetadata = {
        taskId: 'task-1',
        pid: 12345,
        port: 3000,
        command: 'npm run dev',
      }

      mockFsReadFile.mockResolvedValue(JSON.stringify(mockMetadata))

      const mockProcessKill = vi.spyOn(process, 'kill').mockImplementation(() => true)

      const result = await service.stopServer('task-1', { forceKill: true })

      expect(result).toBe(true)
      expect(mockProcessKill).toHaveBeenCalledWith(12345, 'SIGKILL')
      expect(mockFsUnlink).toHaveBeenCalled()

      mockProcessKill.mockRestore()
    })

    it('should return false when no server or metadata found', async () => {
      mockFsReadFile.mockRejectedValue(new Error('File not found'))

      const result = await service.stopServer('non-existent')

      expect(result).toBe(false)
    })
  })

  describe('getServerStatus', () => {
    beforeEach(() => {
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
      vi.mocked(mockStorage.getTask).mockResolvedValue(mockTask)

      const mockRepository: RepositoryConfig = {
        id: 'repo-1',
        name: 'Test Repo',
        path: '/test/repo',
        codeExecution: {
          type: 'local',
          envVars: { NODE_ENV: 'development' },
        },
        previewConfig: {
          framework: 'vite',
          serverStartCommand: 'npm run dev',
          injectionStrategy: {
            method: 'html-injection',
            config: {
              indexPath: 'index.html',
              insertLocation: 'head',
            },
          },
        },
        createdAt: new Date(),
        updatedAt: new Date(),
      }

      const mockWorkspace = {
        getWorkspacePath: vi.fn().mockReturnValue('/workspace/task-1'),
        exists: vi.fn().mockResolvedValue(true),
        environmentConfig: mockRepository,
      }

      vi.mocked(mockWorkspaceService.getOrCreateWorkspace).mockResolvedValue(mockWorkspace as any)
    })

    it('should return running server status', async () => {
      const mockHandler = {
        getStatus: vi.fn().mockReturnValue({
          running: true,
          configured: true,
          url: 'http://localhost:3000',
          pid: 12345,
          uptimeSeconds: 60,
        }),
      }

      // @ts-ignore - accessing private member for testing
      service.runningServers.set('task-1', mockHandler as any)

      // Mock pingServer
      vi.spyOn(service as any, 'pingServer').mockResolvedValue(true)

      const status = await service.getServerStatus('task-1')

      expect(status).toMatchObject({
        running: true,
        configured: true,
        url: 'http://localhost:3000',
        pid: 12345,
        uptimeSeconds: 60,
        responsive: true,
      })
    })

    it('should return not running when metadata exists but process is gone', async () => {
      const mockMetadata = {
        taskId: 'task-1',
        pid: 12345,
        port: 3000,
        command: 'npm run dev',
        workspacePath: '/workspace/task-1',
        startTime: new Date(Date.now() - 120000), // 2 minutes ago
        processStartTime: Date.now() - 120000,
      }

      mockFsReadFile.mockImplementation(async () => {
        // Return stringified metadata, but with startTime as ISO string
        return JSON.stringify({
          ...mockMetadata,
          startTime: mockMetadata.startTime.toISOString(),
        })
      })

      const { execSync } = await import('node:child_process')
      vi.mocked(execSync).mockImplementation((cmd) => {
        if (cmd.toString().includes('ps -p')) {
          // Throw error to simulate process not found
          throw new Error('Process not found')
        }
        return Buffer.from('node')
      })

      const status = await service.getServerStatus('task-1')

      // When process is not found, it should return not running
      expect(status).toMatchObject({
        running: false,
        configured: true,
        uptimeSeconds: 0,
      })

      // Metadata should be cleaned up
      expect(mockFsUnlink).toHaveBeenCalledWith(expect.stringContaining('task-1.json'))
    })

    it('should return not configured status when no server found', async () => {
      mockFsReadFile.mockRejectedValue(new Error('File not found'))

      // Mock for the non-existent task
      vi.mocked(mockStorage.getTask).mockResolvedValue(null)

      await expect(service.getServerStatus('non-existent')).rejects.toThrow(TaskNotFoundError)
    })
  })

  describe('getServerLogs', () => {
    it('should return logs from running server', async () => {
      const mockHandler = {
        getLogs: vi.fn().mockResolvedValue('Server started on port 3000\nRequest received'),
      }

      // @ts-ignore - accessing private member for testing
      service.runningServers.set('task-1', mockHandler as any)

      const logs = await service.getServerLogs('task-1', 10)

      expect(logs).toBe('Server started on port 3000\nRequest received')
      expect(mockHandler.getLogs).toHaveBeenCalledWith(10)
    })

    it('should return logs from metadata when server not running', async () => {
      const mockMetadata = {
        taskId: 'task-1',
        logs: 'Error: Port already in use',
        failed: true,
      }

      mockFsReadFile.mockResolvedValue(JSON.stringify(mockMetadata))

      const logs = await service.getServerLogs('task-1')

      expect(logs).toBe('Error: Port already in use')
    })

    it('should return empty string when no logs available', async () => {
      mockFsReadFile.mockRejectedValue(new Error('File not found'))

      const logs = await service.getServerLogs('non-existent')

      expect(logs).toBe('')
    })
  })

  // detectFramework method removed, skipping related tests

  describe('stopAllServers', () => {
    it('should stop all running servers and proxies', async () => {
      const mockHandler1 = { stop: vi.fn().mockResolvedValue(true) }
      const mockHandler2 = { stop: vi.fn().mockResolvedValue(true) }
      const mockProxy1 = { stop: vi.fn() }

      // @ts-ignore - accessing private members for testing
      service.runningServers.set('task-1', mockHandler1 as any)
      // @ts-ignore - accessing private members for testing
      service.runningServers.set('task-2', mockHandler2 as any)
      // @ts-ignore - accessing private members for testing
      service.runningProxies.set('task-1', mockProxy1 as any)

      await service.stopAllServers()

      expect(mockHandler1.stop).toHaveBeenCalled()
      expect(mockHandler2.stop).toHaveBeenCalled()
      expect(mockProxy1.stop).toHaveBeenCalled()
    })
  })
})
