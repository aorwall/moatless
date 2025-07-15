import { promises as fs } from 'node:fs'
import path from 'node:path'
import type { CreateRepositoryRequest, RepositoryConfig } from '@moatless/types'
import { RepositoryNotFoundError, ValidationError } from '@moatless/types'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { RepositoryService } from '../repository.service.js'
import type { StorageBackend } from '../storage.js'

// Mock dependencies
vi.mock('node:fs', () => ({
  promises: {
    stat: vi.fn(),
    access: vi.fn(),
    readFile: vi.fn(),
  },
}))

vi.mock('node:child_process', () => ({
  spawn: vi.fn(),
  exec: vi.fn((_cmd, callback) => callback?.(null, { stdout: '', stderr: '' }, null)),
}))

vi.mock('../../utils/file-search.js', () => ({
  searchFilesInDirectoryNew: vi.fn(),
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

describe('RepositoryService', () => {
  let service: RepositoryService
  let mockStorage: StorageBackend
  const mockFsStat = vi.mocked(fs.stat)
  const mockFsAccess = vi.mocked(fs.access)
  const mockFsReadFile = vi.mocked(fs.readFile)

  beforeEach(() => {
    mockStorage = createMockStorage()
    service = new RepositoryService(mockStorage)
    vi.clearAllMocks()
  })

  afterEach(() => {
    vi.restoreAllMocks()
  })

  describe('createRepository', () => {
    const validRequest: CreateRepositoryRequest = {
      name: 'Test Repo',
      description: 'Test Description',
      path: '/test/repo',
      useGitWorktrees: true,
      originalBranch: 'main',
      codeExecution: {
        type: 'local',
        setupCommands: ['npm install'],
      },
      previewConfig: {
        framework: 'vite',
        serverStartCommand: 'npm run dev',
      },
    }

    it('should create a repository with valid configuration', async () => {
      // Mock validation
      mockFsStat.mockResolvedValue({ isDirectory: () => true } as any)
      mockFsAccess.mockResolvedValue(undefined)

      const result = await service.createRepository(validRequest)

      expect(result).toMatchObject({
        name: validRequest.name,
        description: validRequest.description,
        path: validRequest.path,
        useGitWorktrees: validRequest.useGitWorktrees,
        originalBranch: validRequest.originalBranch,
        codeExecution: validRequest.codeExecution,
        previewConfig: validRequest.previewConfig,
      })
      expect(result.id).toBeDefined()
      expect(result.createdAt).toBeInstanceOf(Date)
      expect(result.updatedAt).toBeInstanceOf(Date)
      expect(mockStorage.saveRepository).toHaveBeenCalledWith(expect.objectContaining(result))
    })

    it('should create repository with minimal code execution config', async () => {
      mockFsStat.mockResolvedValue({ isDirectory: () => true } as any)
      mockFsAccess.mockResolvedValue(undefined)

      const requestWithMinimalConfig = {
        ...validRequest,
        codeExecution: { type: 'local' as const, setupCommands: ['npm install'] },
      }

      const result = await service.createRepository(requestWithMinimalConfig)
      
      expect(result.codeExecution.type).toBe('local')
      expect(result.codeExecution.setupCommands).toEqual(['npm install'])
    })

    it('should throw ValidationError for non-existent path', async () => {
      mockFsStat.mockRejectedValue(new Error('ENOENT'))

      await expect(service.createRepository(validRequest)).rejects.toThrow(ValidationError)
    })

    it('should throw ValidationError for non-directory path', async () => {
      mockFsStat.mockResolvedValue({ isDirectory: () => false } as any)

      await expect(service.createRepository(validRequest)).rejects.toThrow(
        'Path /test/repo is not a directory',
      )
    })

    it('should throw ValidationError for non-git repository', async () => {
      mockFsStat.mockResolvedValue({ isDirectory: () => true } as any)
      mockFsAccess.mockRejectedValue(new Error('ENOENT'))

      await expect(service.createRepository(validRequest)).rejects.toThrow(
        'Path /test/repo is not a git repository',
      )
    })
  })

  describe('getRepositoryById', () => {
    it('should return repository when found', async () => {
      const mockRepo: RepositoryConfig = {
        id: 'test-id',
        name: 'Test Repo',
        path: '/test/repo',
        codeExecution: { type: 'local' },
        createdAt: new Date(),
        updatedAt: new Date(),
      }
      vi.mocked(mockStorage.getRepository).mockResolvedValue(mockRepo)

      const result = await service.getRepositoryById('test-id')
      expect(result).toBe(mockRepo)
    })

    it('should return null when not found', async () => {
      vi.mocked(mockStorage.getRepository).mockResolvedValue(null)

      const result = await service.getRepositoryById('non-existent')
      expect(result).toBeNull()
    })
  })

  describe('listRepositories', () => {
    it('should return sorted repositories', async () => {
      const repo1: RepositoryConfig = {
        id: 'repo-1',
        name: 'Repo 1',
        path: '/repo1',
        codeExecution: { type: 'local' },
        createdAt: new Date('2024-01-01'),
        updatedAt: new Date(),
      }
      const repo2: RepositoryConfig = {
        id: 'repo-2',
        name: 'Repo 2',
        path: '/repo2',
        codeExecution: { type: 'local' },
        createdAt: new Date('2024-01-02'),
        updatedAt: new Date(),
      }

      vi.mocked(mockStorage.list).mockResolvedValue(['repositories/repo-1', 'repositories/repo-2'])
      vi.mocked(mockStorage.getRepository).mockImplementation(async (id) => {
        if (id === 'repo-1') return repo1
        if (id === 'repo-2') return repo2
        return null
      })

      const result = await service.listRepositories()

      expect(result).toHaveLength(2)
      expect(result[0]).toBe(repo2) // Newer first
      expect(result[1]).toBe(repo1)
    })

    it('should handle empty repository list', async () => {
      vi.mocked(mockStorage.list).mockResolvedValue([])

      const result = await service.listRepositories()
      expect(result).toEqual([])
    })
  })

  describe('updateRepository', () => {
    const existingRepo: RepositoryConfig = {
      id: 'test-id',
      name: 'Test Repo',
      path: '/test/repo',
      codeExecution: { type: 'local' as const },
      createdAt: new Date(),
      updatedAt: new Date(),
    }

    it('should update repository successfully', async () => {
      vi.mocked(mockStorage.getRepository).mockResolvedValue(existingRepo)

      const updates = { name: 'Updated Name', description: 'New Description' }
      const result = await service.updateRepository('test-id', updates)

      expect(result).toMatchObject({
        ...existingRepo,
        ...updates,
        id: 'test-id',
      })
      expect(result.updatedAt.getTime()).toBeGreaterThan(existingRepo.updatedAt.getTime())
      expect(mockStorage.saveRepository).toHaveBeenCalledWith(result)
    })

    it('should validate when path is updated', async () => {
      vi.mocked(mockStorage.getRepository).mockResolvedValue(existingRepo)
      mockFsStat.mockResolvedValue({ isDirectory: () => true } as any)
      mockFsAccess.mockResolvedValue(undefined)

      const updates = { path: '/new/path' }
      await service.updateRepository('test-id', updates)

      expect(mockFsStat).toHaveBeenCalledWith('/new/path')
      expect(mockFsAccess).toHaveBeenCalledWith(path.join('/new/path', '.git'))
    })

    it('should throw RepositoryNotFoundError when repository not found', async () => {
      vi.mocked(mockStorage.getRepository).mockResolvedValue(null)

      await expect(service.updateRepository('non-existent', { name: 'New Name' })).rejects.toThrow(
        RepositoryNotFoundError,
      )
    })
  })

  describe('deleteRepository', () => {
    const existingRepo: RepositoryConfig = {
      id: 'test-id',
      name: 'Test Repo',
      path: '/test/repo',
      codeExecution: { type: 'local' as const },
      createdAt: new Date(),
      updatedAt: new Date(),
    }

    it('should delete repository successfully', async () => {
      vi.mocked(mockStorage.getRepository).mockResolvedValue(existingRepo)
      vi.mocked(mockStorage.list).mockResolvedValue([])

      await service.deleteRepository('test-id')
      expect(mockStorage.delete).toHaveBeenCalledWith('repositories/test-id')
    })

    it('should throw error when repository is in use by running task', async () => {
      vi.mocked(mockStorage.getRepository).mockResolvedValue(existingRepo)
      vi.mocked(mockStorage.list).mockResolvedValue(['tasks/task-1'])
      vi.mocked(mockStorage.getTask).mockResolvedValue({
        id: 'task-1',
        repositoryId: 'test-id',
        status: 'running',
        title: 'Test Task',
        createdAt: new Date(),
        updatedAt: new Date(),
      } as any)

      await expect(service.deleteRepository('test-id')).rejects.toThrow(ValidationError)
    })

    it('should allow deletion when tasks are completed', async () => {
      vi.mocked(mockStorage.getRepository).mockResolvedValue(existingRepo)
      vi.mocked(mockStorage.list).mockResolvedValue(['tasks/task-1'])
      vi.mocked(mockStorage.getTask).mockResolvedValue({
        id: 'task-1',
        repositoryId: 'test-id',
        status: 'completed',
        title: 'Test Task',
        createdAt: new Date(),
        updatedAt: new Date(),
      } as any)

      await service.deleteRepository('test-id')
      expect(mockStorage.delete).toHaveBeenCalledWith('repositories/test-id')
    })

    it('should throw RepositoryNotFoundError when repository not found', async () => {
      vi.mocked(mockStorage.getRepository).mockResolvedValue(null)

      await expect(service.deleteRepository('non-existent')).rejects.toThrow(
        RepositoryNotFoundError,
      )
    })
  })

  describe('runSetupScript', () => {
    const mockRepo: RepositoryConfig = {
      id: 'test-id',
      name: 'Test Repo',
      path: '/test/repo',
      codeExecution: {
        type: 'local',
        setupCommands: ['npm install', 'npm run build'],
      },
      createdAt: new Date(),
      updatedAt: new Date(),
    }

    it('should run setup commands successfully', async () => {
      const { spawn } = await import('node:child_process')
      const mockSpawn = vi.mocked(spawn)

      const mockProcess = {
        stdout: { on: vi.fn() },
        stderr: { on: vi.fn() },
        on: vi.fn(),
      }

      mockSpawn.mockReturnValue(mockProcess as any)

      // Simulate successful command execution
      setTimeout(() => {
        mockProcess.stdout.on.mock.calls[0]?.[1]('Output from npm install')
        mockProcess.on.mock.calls.find((call) => call[0] === 'close')?.[1](0)
      }, 10)

      const resultPromise = service.runSetupScript(mockRepo)

      // Reset mock for second command
      setTimeout(() => {
        mockProcess.stdout.on.mock.calls[0]?.[1]('Output from npm run build')
        mockProcess.on.mock.calls.find((call) => call[0] === 'close')?.[1](0)
      }, 20)

      const result = await resultPromise

      expect(result.success).toBe(true)
      expect(result.commands).toHaveLength(2)
      expect(result.commands[0]).toMatchObject({
        command: 'npm install',
        success: true,
        exit_code: 0,
      })
    })

    it('should handle empty setup commands', async () => {
      const repoWithoutSetup = { ...mockRepo, codeExecution: { type: 'local' as const } }

      const result = await service.runSetupScript(repoWithoutSetup)

      expect(result.success).toBe(true)
      expect(result.commands).toEqual([])
    })

    it('should handle command failure', async () => {
      const { spawn } = await import('node:child_process')
      const mockSpawn = vi.mocked(spawn)

      const mockProcess = {
        stdout: { on: vi.fn() },
        stderr: { on: vi.fn() },
        on: vi.fn(),
      }

      mockSpawn.mockReturnValue(mockProcess as any)

      // Simulate failed command execution
      setTimeout(() => {
        mockProcess.stderr.on.mock.calls[0]?.[1]('Error: Command failed')
        mockProcess.on.mock.calls.find((call) => call[0] === 'close')?.[1](1)
      }, 10)

      const result = await service.runSetupScript({
        ...mockRepo,
        codeExecution: { type: 'local', setupCommands: ['invalid-command'] },
      })

      expect(result.success).toBe(false)
      expect(result.commands[0]).toMatchObject({
        command: 'invalid-command',
        success: false,
        exit_code: 1,
      })
    })
  })

  describe('getBranches', () => {
    it('should return sorted branch list', async () => {
      const { spawn } = await import('node:child_process')
      const mockSpawn = vi.mocked(spawn)

      const mockProcess = {
        stdout: { on: vi.fn() },
        stderr: { on: vi.fn() },
        on: vi.fn(),
      }

      mockSpawn.mockReturnValue(mockProcess as any)

      // Simulate git branch output
      setTimeout(() => {
        const gitOutput = `* main
  feature/branch-1
  feature/branch-2
  remotes/origin/main
  remotes/origin/feature/branch-1
  remotes/origin/HEAD -> origin/main
  workspace-123`
        mockProcess.stdout.on.mock.calls[0]?.[1](gitOutput)
        mockProcess.on.mock.calls.find((call) => call[0] === 'close')?.[1](0)
      }, 10)

      const branches = await service.getBranches('/test/repo')

      expect(branches).toEqual(['main', 'feature/branch-1', 'feature/branch-2'])
    })

    it('should return empty array on error', async () => {
      const { spawn } = await import('node:child_process')
      const mockSpawn = vi.mocked(spawn)

      const mockProcess = {
        stdout: { on: vi.fn() },
        stderr: { on: vi.fn() },
        on: vi.fn(),
      }

      mockSpawn.mockReturnValue(mockProcess as any)

      // Simulate error
      setTimeout(() => {
        mockProcess.on.mock.calls.find((call) => call[0] === 'error')?.[1](
          new Error('Command failed'),
        )
      }, 10)

      const branches = await service.getBranches('/test/repo')
      expect(branches).toEqual([])
    })
  })

  describe('searchFiles', () => {
    it('should search files in repository', async () => {
      const mockRepo: RepositoryConfig = {
        id: 'test-id',
        name: 'Test Repo',
        path: '/test/repo',
        codeExecution: { type: 'local' },
        createdAt: new Date(),
        updatedAt: new Date(),
      }

      vi.mocked(mockStorage.getRepository).mockResolvedValue(mockRepo)

      const { searchFilesInDirectoryNew } = await import('../../utils/file-search.js')
      const mockSearch = vi.mocked(searchFilesInDirectoryNew)
      mockSearch.mockResolvedValue({
        results: [
          { path: 'file1.ts', absolutePath: '/test/repo/file1.ts', score: 1 },
          { path: 'file2.ts', absolutePath: '/test/repo/file2.ts', score: 1 },
        ],
        totalFound: 2,
        query: 'test',
      })

      const result = await service.searchFiles('test-id', { query: 'test', limit: 10 })

      expect(mockSearch).toHaveBeenCalledWith('/test/repo', { query: 'test', limit: 10 })
      expect(result).toEqual({
        results: [
          { path: 'file1.ts', absolutePath: '/test/repo/file1.ts', score: 1 },
          { path: 'file2.ts', absolutePath: '/test/repo/file2.ts', score: 1 },
        ],
        totalFound: 2,
        query: 'test',
      })
    })

    it('should throw RepositoryNotFoundError when repository not found', async () => {
      vi.mocked(mockStorage.getRepository).mockResolvedValue(null)

      await expect(service.searchFiles('non-existent', { query: 'test' })).rejects.toThrow(
        RepositoryNotFoundError,
      )
    })
  })

  describe('detectFramework', () => {
    const mockPackageJson = (deps: Record<string, string>, scripts?: Record<string, string>) => {
      mockFsReadFile.mockResolvedValue(
        JSON.stringify({
          dependencies: deps,
          scripts: scripts || {},
        }),
      )
    }

    beforeEach(() => {
      // Mock package manager detection
      mockFsAccess.mockRejectedValue(new Error('ENOENT')) // Default: no lock files
    })

    it('should detect Vite framework', async () => {
      mockPackageJson({ vite: '^5.0.0' }, { dev: 'vite' })

      const result = await service.detectFramework('/test/repo')

      expect(result).toMatchObject({
        framework: 'vite',
        frameworkVersion: '^5.0.0',
        serverStartCommand: 'npm run dev',
        injectionStrategy: {
          method: 'html-injection',
          config: {
            indexPath: 'index.html',
            insertLocation: 'head',
          },
        },
      })
    })

    it('should detect Next.js framework', async () => {
      mockPackageJson({ next: '^14.0.0' }, { dev: 'next dev' })

      const result = await service.detectFramework('/test/repo')

      expect(result).toMatchObject({
        framework: 'nextjs',
        frameworkVersion: '^14.0.0',
        serverStartCommand: 'npm run dev',
        injectionStrategy: {
          method: 'middleware',
        },
      })
    })

    it('should detect Create React App', async () => {
      mockPackageJson({ 'react-scripts': '^5.0.0' }, { start: 'react-scripts start' })

      const result = await service.detectFramework('/test/repo')

      expect(result).toMatchObject({
        framework: 'cra',
        frameworkVersion: '^5.0.0',
        serverStartCommand: 'npm run start',
        injectionStrategy: {
          method: 'html-injection',
          config: {
            indexPath: 'public/index.html',
            insertLocation: 'head',
          },
        },
      })
    })

    it('should handle unknown framework', async () => {
      mockPackageJson({ express: '^4.0.0' }, { start: 'node server.js' })

      const result = await service.detectFramework('/test/repo')

      expect(result).toMatchObject({
        framework: 'unknown',
        serverStartCommand: 'npm run start',
        injectionStrategy: {
          method: 'html-injection',
          config: {},
        },
      })
    })

    it('should detect pnpm package manager', async () => {
      // Mock pnpm-lock.yaml exists
      mockFsAccess.mockImplementation(async (filePath) => {
        if (filePath.toString().endsWith('pnpm-lock.yaml')) {
          return undefined
        }
        throw new Error('ENOENT')
      })

      mockPackageJson({ vite: '^5.0.0' })

      const result = await service.detectFramework('/test/repo')

      expect(result.serverStartCommand).toContain('pnpm')
    })

    it('should handle missing package.json gracefully', async () => {
      mockFsReadFile.mockRejectedValue(new Error('ENOENT'))

      const result = await service.detectFramework('/test/repo')

      expect(result).toMatchObject({
        framework: 'unknown',
        serverStartCommand: expect.stringContaining('run dev'),
        injectionStrategy: {
          method: 'html-injection',
          config: {},
        },
      })
    })
  })
})
