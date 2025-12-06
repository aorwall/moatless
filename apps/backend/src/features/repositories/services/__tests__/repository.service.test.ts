import type { RemoteRepository } from '@moatless/types'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { GitHubProviderClient } from '../../../../infrastructure/clients/github-provider.client'
import type { GitLabProviderClient } from '../../../../infrastructure/clients/gitlab-provider.client'
import type { GitProviderTokenService } from '../../../auth/services/git-provider-token.service'
import type { RepositoryRepository } from '../../db/repository.repository'
import { RepositoryService } from '../repository.service'
import type { RepositoryGitService } from '../repository-git.service'

describe('RepositoryService', () => {
  let repositoryService: RepositoryService
  let mockGitService: RepositoryGitService
  let mockRepository: RepositoryRepository
  let mockGithubProvider: GitHubProviderClient
  let mockGitlabProvider: GitLabProviderClient
  let mockGitProviderTokenService: GitProviderTokenService

  beforeEach(() => {
    // Create mocks
    mockGitService = {
      getBranches: vi.fn(),
      parseGitUrl: vi.fn(),
    } as any

    mockRepository = {
      get: vi.fn(),
      save: vi.fn(),
      getAll: vi.fn(),
      delete: vi.fn(),
    } as any

    mockGithubProvider = {
      listBranches: vi.fn(),
      getDefaultBranch: vi.fn(),
    } as any

    mockGitlabProvider = {
      listBranches: vi.fn(),
      getDefaultBranch: vi.fn(),
    } as any

    mockGitProviderTokenService = {
      getTokenForProvider: vi.fn(),
      detectProvider: vi.fn(),
      buildAuthenticatedCloneUrl: vi.fn(),
    } as any

    // Create service instance
    repositoryService = new RepositoryService(
      mockGitService,
      mockRepository,
      mockGithubProvider,
      mockGitlabProvider,
      mockGitProviderTokenService,
    )
  })

  describe('getBranches', () => {
    describe('for remote repositories', () => {
      it('should include default branch when it is not in the first 100 branches from GitHub', async () => {
        const mockRemoteRepository: RemoteRepository = {
          kind: 'remote',
          id: 'repo-123',
          name: 'Test Repo',
          remote: {
            url: 'https://github.com/test/repo',
            provider: 'github',
            defaultBranch: 'main',
          },
          codeExecution: {},
          visibility: 'private',
          createdAt: new Date(),
          updatedAt: new Date(),
        }

        // Mock repository.get to return the remote repository
        vi.spyOn(mockRepository, 'get').mockResolvedValueOnce(mockRemoteRepository)

        // Mock GitHub provider to return 100 branches WITHOUT the default branch
        const branchesWithoutDefault = Array.from({ length: 100 }, (_, i) => `branch-${i}`)
        vi.spyOn(mockGithubProvider, 'listBranches').mockResolvedValueOnce(branchesWithoutDefault)

        // Mock token service
        vi.spyOn(mockGitProviderTokenService, 'getTokenForProvider').mockResolvedValueOnce(
          'mock-token',
        )

        const branches = await repositoryService.getBranches('repo-123')

        // Verify default branch is included and is first
        expect(branches).toContain('main')
        expect(branches[0]).toBe('main')
        expect(branches).toHaveLength(101) // 100 + default branch
      })

      it('should not duplicate default branch if it is already in the list from GitHub', async () => {
        const mockRemoteRepository: RemoteRepository = {
          kind: 'remote',
          id: 'repo-123',
          name: 'Test Repo',
          remote: {
            url: 'https://github.com/test/repo',
            provider: 'github',
            defaultBranch: 'main',
          },
          codeExecution: {},
          visibility: 'private',
          createdAt: new Date(),
          updatedAt: new Date(),
        }

        // Mock repository.get to return the remote repository
        vi.spyOn(mockRepository, 'get').mockResolvedValueOnce(mockRemoteRepository)

        // Mock GitHub provider to return branches WITH the default branch
        const branchesWithDefault = ['main', 'develop', 'feature-1', 'feature-2']
        vi.spyOn(mockGithubProvider, 'listBranches').mockResolvedValueOnce(branchesWithDefault)

        // Mock token service
        vi.spyOn(mockGitProviderTokenService, 'getTokenForProvider').mockResolvedValueOnce(
          'mock-token',
        )

        const branches = await repositoryService.getBranches('repo-123')

        // Verify default branch is not duplicated
        const mainCount = branches.filter((b) => b === 'main').length
        expect(mainCount).toBe(1)
        expect(branches[0]).toBe('main')
        expect(branches).toHaveLength(4) // Original 4 branches, no duplication
      })

      it('should include default branch when it is not in the first 100 branches from GitLab', async () => {
        const mockRemoteRepository: RemoteRepository = {
          kind: 'remote',
          id: 'repo-123',
          name: 'Test Repo',
          remote: {
            url: 'https://gitlab.com/test/repo',
            provider: 'gitlab',
            defaultBranch: 'master',
          },
          codeExecution: {},
          visibility: 'private',
          createdAt: new Date(),
          updatedAt: new Date(),
        }

        // Mock repository.get to return the remote repository
        vi.spyOn(mockRepository, 'get').mockResolvedValueOnce(mockRemoteRepository)

        // Mock GitLab provider to return 100 branches WITHOUT the default branch
        const branchesWithoutDefault = Array.from({ length: 100 }, (_, i) => `branch-${i}`)
        vi.spyOn(mockGitlabProvider, 'listBranches').mockResolvedValueOnce(branchesWithoutDefault)

        // Mock token service
        vi.spyOn(mockGitProviderTokenService, 'getTokenForProvider').mockResolvedValueOnce(
          'mock-token',
        )

        const branches = await repositoryService.getBranches('repo-123')

        // Verify default branch is included and is first
        expect(branches).toContain('master')
        expect(branches[0]).toBe('master')
        expect(branches).toHaveLength(101) // 100 + default branch
      })

      it('should handle undefined default branch gracefully', async () => {
        const mockRemoteRepository: RemoteRepository = {
          kind: 'remote',
          id: 'repo-123',
          name: 'Test Repo',
          remote: {
            url: 'https://github.com/test/repo',
            provider: 'github',
            defaultBranch: undefined,
          },
          codeExecution: {},
          visibility: 'private',
          createdAt: new Date(),
          updatedAt: new Date(),
        }

        // Mock repository.get to return the remote repository
        vi.spyOn(mockRepository, 'get').mockResolvedValueOnce(mockRemoteRepository)

        // Mock GitHub provider to return some branches
        const branches = ['develop', 'feature-1', 'feature-2']
        vi.spyOn(mockGithubProvider, 'listBranches').mockResolvedValueOnce(branches)

        // Mock token service
        vi.spyOn(mockGitProviderTokenService, 'getTokenForProvider').mockResolvedValueOnce(
          'mock-token',
        )

        const result = await repositoryService.getBranches('repo-123')

        // Verify branches are returned as-is without adding undefined
        expect(result).toEqual(branches)
        expect(result).toHaveLength(3)
      })
    })

    describe('for template repositories', () => {
      it('should include default branch when it is not in the first 100 branches', async () => {
        const mockTemplateRepository = {
          kind: 'template' as const,
          id: 'template-123',
          name: 'Test Template',
          remote: {
            url: 'https://github.com/test/template',
            provider: 'github' as const,
            defaultBranch: 'main',
          },
          codeExecution: {},
          visibility: 'private' as const,
          createdAt: new Date(),
          updatedAt: new Date(),
        }

        // Mock repository.get to return the template repository
        vi.spyOn(mockRepository, 'get').mockResolvedValueOnce(mockTemplateRepository)

        // Mock GitHub provider to return 100 branches WITHOUT the default branch
        const branchesWithoutDefault = Array.from({ length: 100 }, (_, i) => `branch-${i}`)
        vi.spyOn(mockGithubProvider, 'listBranches').mockResolvedValueOnce(branchesWithoutDefault)

        // Mock token service
        vi.spyOn(mockGitProviderTokenService, 'getTokenForProvider').mockResolvedValueOnce(
          'mock-token',
        )

        const branches = await repositoryService.getBranches('template-123')

        // Verify default branch is included and is first
        expect(branches).toContain('main')
        expect(branches[0]).toBe('main')
        expect(branches).toHaveLength(101) // 100 + default branch
      })
    })
  })
})
