import type { RepositoryConfig } from '@moatless/types'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { RepositoryService } from '../repository.service.js'
import type { StorageBackend } from '../storage.js'

describe('RepositoryService', () => {
  let repositoryService: RepositoryService
  let mockStorage: StorageBackend

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
    }

    repositoryService = new RepositoryService(mockStorage)
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
})
