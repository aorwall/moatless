import type { CreateRepositoryRequest, RepositoryConfig } from '@moatless/types'
import { ValidationError } from '@moatless/types'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { StorageBackend } from '../../storage/index.js'
import { RepositoryService } from '../repository.service.js'
import type { SettingsService } from '../settings.service.js'

// Mock fs module
vi.mock('node:fs', () => ({
  promises: {
    mkdir: vi.fn(),
    stat: vi.fn(),
    access: vi.fn(),
    readdir: vi.fn(),
    copyFile: vi.fn(),
  },
}))

// Mock git-utils
vi.mock('../../utils/git-utils.js', () => ({
  cloneRepository: vi.fn(),
  isRegularGitRepository: vi.fn(),
}))

describe('RepositoryService', () => {
  let repositoryService: RepositoryService
  let mockStorage: StorageBackend
  let mockSettingsService: SettingsService

  beforeEach(() => {
    mockStorage = {
      // Generic storage methods
      save: vi.fn(),
      get: vi.fn(),
      list: vi.fn(),
      delete: vi.fn(),
      exists: vi.fn(),
      // JSONL methods
      appendJsonl: vi.fn(),
      readJsonl: vi.fn().mockResolvedValue([]),
      // Type-safe convenience methods
      saveTask: vi.fn(),
      getTask: vi.fn(),
      saveRepository: vi.fn(),
      getRepository: vi.fn(),
      saveWorkspace: vi.fn(),
      getWorkspace: vi.fn(),
      saveEvents: vi.fn(),
      getEvents: vi.fn(),
      saveUser: vi.fn(),
      getUser: vi.fn(),
      listUsers: vi.fn().mockResolvedValue([]),
    }

    mockSettingsService = {
      getRepositoriesDir: vi.fn().mockReturnValue('/test/repos'),
      getGitHubSettings: vi.fn(),
    } as any

    repositoryService = new RepositoryService(mockStorage)
    repositoryService.setSettingsService(mockSettingsService)
  })

  afterEach(() => {
    vi.clearAllMocks()
  })

  describe('getRepositoryById', () => {
    it('should return repository when it exists and is not deleted', async () => {
      const mockRepository: RepositoryConfig = {
        id: 'repo-123',
        name: 'test-repo',
        path: '/test/path',
        cloneOnNewTask: false,
        codeExecution: { type: 'local' },
        createdAt: new Date(),
        updatedAt: new Date(),
      }

      ;(mockStorage.getRepository as any).mockResolvedValueOnce(mockRepository)

      const result = await repositoryService.getRepositoryById('repo-123')

      expect(result).toEqual(mockRepository)
      expect(mockStorage.getRepository).toHaveBeenCalledWith('repo-123')
    })

    it('should return null when repository does not exist', async () => {
      ;(mockStorage.getRepository as any).mockResolvedValueOnce(null)

      const result = await repositoryService.getRepositoryById('non-existent')

      expect(result).toBeNull()
      expect(mockStorage.getRepository).toHaveBeenCalledWith('non-existent')
    })

    it('should return null when repository is soft-deleted', async () => {
      const deletedRepository: RepositoryConfig = {
        id: 'repo-123',
        name: 'test-repo',
        path: '/test/path',
        deleted: true,
        cloneOnNewTask: false,
        codeExecution: { type: 'local' },
        createdAt: new Date(),
        updatedAt: new Date(),
      }

      ;(mockStorage.getRepository as any).mockResolvedValueOnce(deletedRepository)

      const result = await repositoryService.getRepositoryById('repo-123')

      expect(result).toBeNull()
      expect(mockStorage.getRepository).toHaveBeenCalledWith('repo-123')
    })
  })

  describe('listRepositories', () => {
    it('should filter out soft-deleted repositories', async () => {
      const activeRepo: RepositoryConfig = {
        id: 'repo-1',
        name: 'active-repo',
        path: '/active/path',
        cloneOnNewTask: false,
        codeExecution: { type: 'local' },
        createdAt: new Date('2024-01-01'),
        updatedAt: new Date(),
      }

      const deletedRepo: RepositoryConfig = {
        id: 'repo-2',
        name: 'deleted-repo',
        path: '/deleted/path',
        deleted: true,
        cloneOnNewTask: false,
        codeExecution: { type: 'local' },
        createdAt: new Date('2024-01-02'),
        updatedAt: new Date(),
      }

      ;(mockStorage.list as any).mockResolvedValueOnce([
        'repositories/repo-1',
        'repositories/repo-2',
      ])
      ;(mockStorage.getRepository as any)
        .mockResolvedValueOnce(activeRepo)
        .mockResolvedValueOnce(deletedRepo)

      const result = await repositoryService.listRepositories()

      expect(result).toHaveLength(1)
      expect(result[0]).toEqual(activeRepo)
      expect(mockStorage.list).toHaveBeenCalledWith('repositories/')
      expect(mockStorage.getRepository).toHaveBeenCalledTimes(2)
    })

    it('should sort repositories by creation date (newest first)', async () => {
      const olderRepo: RepositoryConfig = {
        id: 'repo-1',
        name: 'older-repo',
        path: '/older/path',
        cloneOnNewTask: false,
        codeExecution: { type: 'local' },
        createdAt: new Date('2024-01-01'),
        updatedAt: new Date(),
      }

      const newerRepo: RepositoryConfig = {
        id: 'repo-2',
        name: 'newer-repo',
        path: '/newer/path',
        cloneOnNewTask: false,
        codeExecution: { type: 'local' },
        createdAt: new Date('2024-01-15'),
        updatedAt: new Date(),
      }

      ;(mockStorage.list as any).mockResolvedValueOnce([
        'repositories/repo-1',
        'repositories/repo-2',
      ])
      ;(mockStorage.getRepository as any)
        .mockResolvedValueOnce(olderRepo)
        .mockResolvedValueOnce(newerRepo)

      const result = await repositoryService.listRepositories()

      expect(result).toHaveLength(2)
      expect(result[0]?.id).toBe('repo-2') // Newer repo should be first
      expect(result[1]?.id).toBe('repo-1') // Older repo should be second
    })
  })

  describe('deleteRepository', () => {
    it('should perform soft delete on existing repository', async () => {
      const existingRepo: RepositoryConfig = {
        id: 'repo-123',
        name: 'test-repo',
        path: '/test/path',
        cloneOnNewTask: false,
        codeExecution: { type: 'local' },
        createdAt: new Date(),
        updatedAt: new Date(),
      }

      ;(mockStorage.getRepository as any).mockResolvedValueOnce(existingRepo)
      ;(mockStorage.list as any).mockResolvedValueOnce([]) // No tasks

      await repositoryService.deleteRepository('repo-123')

      expect(mockStorage.saveRepository).toHaveBeenCalledWith(
        expect.objectContaining({
          ...existingRepo,
          deleted: true,
        }),
      )
    })

    it('should do nothing if repository is already deleted', async () => {
      const deletedRepo: RepositoryConfig = {
        id: 'repo-123',
        name: 'test-repo',
        path: '/test/path',
        deleted: true,
        cloneOnNewTask: false,
        codeExecution: { type: 'local' },
        createdAt: new Date(),
        updatedAt: new Date(),
      }

      // Note: getRepositoryById in deleteRepository doesn't use the filtered version
      // It directly calls storage.getRepository to check if repo exists at all
      ;(mockStorage.getRepository as any).mockResolvedValueOnce(deletedRepo)

      await repositoryService.deleteRepository('repo-123')

      // Should not save again since it's already deleted
      expect(mockStorage.saveRepository).not.toHaveBeenCalled()
    })
  })

  describe('createRepositoryFromTemplate', () => {
    it('should successfully create repository from template', async () => {
      const { cloneRepository } = await import('../../utils/git-utils.js')
      const fs = await import('node:fs')

      await repositoryService.createRepositoryFromTemplate('my-repo', 'hello-world-ui')

      expect(fs.promises.mkdir).toHaveBeenCalledWith('/test/repos', { recursive: true })
      expect(cloneRepository).toHaveBeenCalledWith(
        'https://github.com/aorwall/hello-world-ui.git',
        '/test/repos/my-repo',
      )
      expect(mockStorage.saveRepository).toHaveBeenCalledWith(
        expect.objectContaining({
          name: 'my-repo',
          path: '/test/repos/my-repo',
          sourceType: 'template',
          templateId: 'hello-world-ui',
          vibe: true,
        }),
      )
    })

    it('should throw error when template not found', async () => {
      await expect(
        repositoryService.createRepositoryFromTemplate('my-repo', 'non-existent-template'),
      ).rejects.toThrow(ValidationError)
    })

    it('should merge template config with defaults', async () => {
      await repositoryService.createRepositoryFromTemplate('my-repo', 'hello-world-ui')

      const savedRepo = (mockStorage.saveRepository as any).mock.calls[0][0]
      expect(savedRepo.codeExecution).toEqual({
        type: 'local',
        setupCommands: ['bun install'],
      })
      expect(savedRepo.previewConfig).toMatchObject({
        serverStartCommands: ['bun run dev'],
        buildCommand: 'bun run build',
        takeScreenshots: true,
        enableInspector: true,
      })
    })
  })

  describe('createRepository', () => {
    it('should create repository from local path', async () => {
      const fs = await import('node:fs')
      const request: CreateRepositoryRequest = {
        name: 'test-repo',
        path: '/local/path',
        sourceType: 'local',
        cloneOnNewTask: false,
        codeExecution: { type: 'local' },
      }

      vi.mocked(fs.promises.stat).mockResolvedValue({ isDirectory: () => true } as any)
      vi.mocked(fs.promises.access).mockResolvedValue(undefined)

      const result = await repositoryService.createRepository(request)

      expect(result.name).toBe('test-repo')
      expect(result.path).toBe('/local/path')
      expect(result.sourceType).toBe('local')
      expect(mockStorage.saveRepository).toHaveBeenCalled()
    })

    it('should validate cloneOnNewTask requires remoteUrl', async () => {
      const request: CreateRepositoryRequest = {
        name: 'test-repo',
        path: '/local/path',
        cloneOnNewTask: true,
        codeExecution: { type: 'local' },
      }

      await expect(repositoryService.createRepository(request)).rejects.toThrow(ValidationError)
      await expect(repositoryService.createRepository(request)).rejects.toThrow(
        'cloneOnNewTask requires a remoteUrl',
      )
    })

    it('should throw error if path is missing for local repository', async () => {
      const request: CreateRepositoryRequest = {
        name: 'test-repo',
        cloneOnNewTask: false,
        codeExecution: { type: 'local' },
      }

      await expect(repositoryService.createRepository(request)).rejects.toThrow(ValidationError)
      await expect(repositoryService.createRepository(request)).rejects.toThrow(
        'Path is required for local repositories',
      )
    })
  })

  describe('saveRepository', () => {
    it('should update updatedAt and call storage.saveRepository', async () => {
      const repo: RepositoryConfig = {
        id: 'repo-123',
        name: 'test-repo',
        path: '/test/path',
        cloneOnNewTask: false,
        codeExecution: { type: 'local' },
        createdAt: new Date('2024-01-01'),
        updatedAt: new Date('2024-01-01'),
      }

      await repositoryService.saveRepository(repo)

      expect(mockStorage.saveRepository).toHaveBeenCalledWith(
        expect.objectContaining({
          ...repo,
          updatedAt: expect.any(Date),
        }),
      )
      const savedRepo = (mockStorage.saveRepository as any).mock.calls[0][0]
      expect(savedRepo.updatedAt.getTime()).toBeGreaterThanOrEqual(repo.updatedAt.getTime())
    })
  })

  describe('updateRepository', () => {
    const existingRepo: RepositoryConfig = {
      id: 'repo-123',
      name: 'test-repo',
      path: '/test/path',
      cloneOnNewTask: false,
      codeExecution: { type: 'local' },
      createdAt: new Date('2024-01-01'),
      updatedAt: new Date('2024-01-01'),
    }

    it('should update repository with partial values', async () => {
      vi.mocked(mockStorage.getRepository).mockResolvedValue(existingRepo)

      const updates = { name: 'updated-name', description: 'new description' }
      const result = await repositoryService.updateRepository('repo-123', updates)

      expect(result.name).toBe('updated-name')
      expect(result.description).toBe('new description')
      expect(result.id).toBe('repo-123')
      expect(result.updatedAt).toBeInstanceOf(Date)
      expect(mockStorage.saveRepository).toHaveBeenCalled()
    })

    it('should throw error when repository not found', async () => {
      vi.mocked(mockStorage.getRepository).mockResolvedValue(null)

      await expect(
        repositoryService.updateRepository('non-existent', { name: 'new-name' }),
      ).rejects.toThrow('Repository non-existent not found')
    })

    it('should validate path when path is updated', async () => {
      const fs = await import('node:fs')
      vi.mocked(mockStorage.getRepository).mockResolvedValue(existingRepo)
      vi.mocked(fs.promises.stat).mockResolvedValue({ isDirectory: () => true } as any)
      vi.mocked(fs.promises.access).mockResolvedValue(undefined)

      await repositoryService.updateRepository('repo-123', { path: '/new/path' })

      expect(fs.promises.stat).toHaveBeenCalledWith('/new/path')
    })

    it('should throw error if cloneOnNewTask=true but no remoteUrl', async () => {
      vi.mocked(mockStorage.getRepository).mockResolvedValue(existingRepo)

      await expect(
        repositoryService.updateRepository('repo-123', { cloneOnNewTask: true }),
      ).rejects.toThrow(ValidationError)
      await expect(
        repositoryService.updateRepository('repo-123', { cloneOnNewTask: true }),
      ).rejects.toThrow('cloneOnNewTask requires a remoteUrl')
    })
  })

  describe('getBranches', () => {
    it('should return deduplicated branches sorted correctly', async () => {
      const mockExecuteCommand = vi.spyOn(repositoryService as any, 'executeCommand')
      mockExecuteCommand.mockResolvedValue({
        success: true,
        exit_code: 0,
        command: 'git branch -a',
        output: `* main
  develop
  feature/test
  remotes/origin/main
  remotes/origin/develop
  remotes/origin/HEAD -> origin/main`,
      })

      const branches = await repositoryService.getBranches('/test/path')

      expect(branches).toEqual(['main', 'develop', 'feature/test'])
      expect(branches[0]).toBe('main') // main should be first
    })

    it('should filter out workspace branches and HEAD pointer', async () => {
      const mockExecuteCommand = vi.spyOn(repositoryService as any, 'executeCommand')
      mockExecuteCommand.mockResolvedValue({
        success: true,
        exit_code: 0,
        command: 'git branch -a',
        output: `* main
  workspace-abc123
  feature/test
  remotes/origin/HEAD -> origin/main`,
      })

      const branches = await repositoryService.getBranches('/test/path')

      expect(branches).toEqual(['main', 'feature/test'])
      expect(branches).not.toContain('workspace-abc123')
    })

    it('should return empty array on error', async () => {
      const mockExecuteCommand = vi.spyOn(repositoryService as any, 'executeCommand')
      mockExecuteCommand.mockRejectedValue(new Error('Git error'))

      const branches = await repositoryService.getBranches('/test/path')

      expect(branches).toEqual([])
    })
  })

  describe('validateRepository', () => {
    it('should skip validation for GitHub repos without path', async () => {
      const fs = await import('node:fs')
      const repo: CreateRepositoryRequest = {
        name: 'test-repo',
        sourceType: 'github',
        remoteUrl: 'https://github.com/test/repo.git',
        cloneOnNewTask: false,
        codeExecution: { type: 'local' },
      }

      await expect(repositoryService.validateRepository(repo)).resolves.not.toThrow()
      expect(fs.promises.stat).not.toHaveBeenCalled()
    })

    it('should validate local repository exists and is a git repo', async () => {
      const fs = await import('node:fs')
      vi.mocked(fs.promises.stat).mockResolvedValue({ isDirectory: () => true } as any)
      vi.mocked(fs.promises.access).mockResolvedValue(undefined)

      const repo: CreateRepositoryRequest = {
        name: 'test-repo',
        path: '/test/path',
        cloneOnNewTask: false,
        codeExecution: { type: 'local' },
      }

      await expect(repositoryService.validateRepository(repo)).resolves.not.toThrow()
      expect(fs.promises.stat).toHaveBeenCalledWith('/test/path')
      expect(fs.promises.access).toHaveBeenCalledWith('/test/path/.git')
    })

    it('should throw error if path is not a directory', async () => {
      const fs = await import('node:fs')
      vi.mocked(fs.promises.stat).mockResolvedValue({ isDirectory: () => false } as any)

      const repo: CreateRepositoryRequest = {
        name: 'test-repo',
        path: '/test/file.txt',
        cloneOnNewTask: false,
        codeExecution: { type: 'local' },
      }

      await expect(repositoryService.validateRepository(repo)).rejects.toThrow(ValidationError)
      await expect(repositoryService.validateRepository(repo)).rejects.toThrow('is not a directory')
    })

    it('should throw error if path is not a git repository', async () => {
      const fs = await import('node:fs')
      vi.mocked(fs.promises.stat).mockResolvedValue({ isDirectory: () => true } as any)
      vi.mocked(fs.promises.access).mockRejectedValue(new Error('ENOENT'))

      const repo: CreateRepositoryRequest = {
        name: 'test-repo',
        path: '/test/path',
        cloneOnNewTask: false,
        codeExecution: { type: 'local' },
      }

      await expect(repositoryService.validateRepository(repo)).rejects.toThrow(ValidationError)
      await expect(repositoryService.validateRepository(repo)).rejects.toThrow(
        'is not a git repository',
      )
    })

    it('should throw error if path is missing for local repos', async () => {
      const repo: CreateRepositoryRequest = {
        name: 'test-repo',
        cloneOnNewTask: false,
        codeExecution: { type: 'local' },
      }

      await expect(repositoryService.validateRepository(repo)).rejects.toThrow(ValidationError)
      await expect(repositoryService.validateRepository(repo)).rejects.toThrow(
        'Path is required for local repositories',
      )
    })
  })
})
