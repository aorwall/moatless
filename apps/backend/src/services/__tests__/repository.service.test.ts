import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createRepositoryService } from '../repository.service.js'
import type { StorageBackend } from '../storage.js'

describe('RepositoryService', () => {
  let mockStorage: StorageBackend
  let repositoryService: ReturnType<typeof createRepositoryService>

  beforeEach(() => {
    // Create mock storage
    mockStorage = {
      saveRepository: vi.fn(),
      getRepository: vi.fn(),
      list: vi.fn(),
      delete: vi.fn(),
      saveTask: vi.fn(),
      getTask: vi.fn(),
      deleteTask: vi.fn(),
      saveEvents: vi.fn(),
      getEvents: vi.fn(),
      deleteEvents: vi.fn(),
      saveTaskStatus: vi.fn(),
      getTaskStatus: vi.fn(),
      deleteTaskStatus: vi.fn(),
      saveWorkspace: vi.fn(),
      getWorkspace: vi.fn(),
      deleteWorkspace: vi.fn(),
      saveMessages: vi.fn(),
      getMessages: vi.fn(),
      deleteMessages: vi.fn(),
      saveUsage: vi.fn(),
      getUsage: vi.fn(),
      getMonthlyCosts: vi.fn(),
      getDailyCosts: vi.fn(),
      getTotalCostsForTask: vi.fn(),
      exists: vi.fn(),
    } as unknown as StorageBackend

    repositoryService = createRepositoryService(mockStorage)
  })

  describe('getBranches', () => {
    it('should parse branch names correctly removing * and + prefixes', async () => {
      const mockRepo = {
        id: 'test-repo',
        name: 'Test Repo',
        path: '/test/repo',
        useGitWorktrees: false,
        codeExecution: { type: 'local' as const },
        createdAt: new Date(),
        updatedAt: new Date(),
      }

      // Mock executeCommand to return git branch output with both * and + prefixes
      const executeCommandSpy = vi.spyOn(repositoryService as any, 'executeCommand')
      executeCommandSpy.mockResolvedValueOnce({
        command: 'git branch -a',
        success: true,
        exit_code: 0,
        output: `* main
+ feature/create-plugin-service-with-id-plu-34eedcf6
+ feature/create-plugin-service-with-id-plu-5ec74e4c
  feature/some-local-branch
  remotes/origin/feature/remote-branch
  remotes/origin/HEAD -> origin/main
  workspace-123`,
      })

      const branches = await repositoryService.getBranches(mockRepo.path)

      // Verify that * and + prefixes are removed
      expect(branches).toEqual([
        'main',
        'feature/create-plugin-service-with-id-plu-34eedcf6',
        'feature/create-plugin-service-with-id-plu-5ec74e4c',
        'feature/remote-branch',
        'feature/some-local-branch',
      ])

      // Verify workspace branches are filtered out
      expect(branches).not.toContain('workspace-123')
      // Verify HEAD pointer is filtered out
      expect(branches).not.toContain('HEAD -> origin/main')
    })

    it('should handle empty branch list', async () => {
      const executeCommandSpy = vi.spyOn(repositoryService as any, 'executeCommand')
      executeCommandSpy.mockResolvedValueOnce({
        command: 'git branch -a',
        success: true,
        exit_code: 0,
        output: '',
      })

      const branches = await repositoryService.getBranches('/test/repo')
      expect(branches).toEqual([])
    })

    it('should handle git command failure', async () => {
      const executeCommandSpy = vi.spyOn(repositoryService as any, 'executeCommand')
      executeCommandSpy.mockResolvedValueOnce({
        command: 'git branch -a',
        success: false,
        exit_code: 1,
        error: 'Not a git repository',
      })

      const branches = await repositoryService.getBranches('/test/repo')
      expect(branches).toEqual([])
    })

    it('should deduplicate and sort branches correctly', async () => {
      const executeCommandSpy = vi.spyOn(repositoryService as any, 'executeCommand')
      executeCommandSpy.mockResolvedValueOnce({
        command: 'git branch -a',
        success: true,
        exit_code: 0,
        output: `* develop
  feature/a
  feature/b
  main
  master
  remotes/origin/develop
  remotes/origin/feature/a
  remotes/origin/main`,
      })

      const branches = await repositoryService.getBranches('/test/repo')

      // Should have main/master first, then others alphabetically
      expect(branches).toEqual(['main', 'master', 'develop', 'feature/a', 'feature/b'])
    })
  })
})
