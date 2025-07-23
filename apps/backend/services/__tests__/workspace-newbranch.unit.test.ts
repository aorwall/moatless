import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import * as gitUtils from '../../utils/git-utils.js'
import type { EventService } from '../event.service.js'
import type { RepositoryService } from '../repository.service.js'
import type { SettingsService } from '../settings.service.js'
import { WorkspaceService } from '../workspace.service.js'

describe('WorkspaceService newBranch Unit Tests', () => {
  let workspaceService: WorkspaceService
  let mockRepositoryService: RepositoryService
  let mockSettingsService: SettingsService
  let mockEventService: EventService

  const mockRepository = {
    id: 'test-repo',
    name: 'Test Repository',
    path: '/test/repo/path',
    useGitWorktrees: true,
    codeExecution: { type: 'local' as const },
  }

  beforeEach(() => {
    // Create mock instances
    mockRepositoryService = {
      getRepositoryById: vi.fn().mockResolvedValue(mockRepository),
    } as any

    mockSettingsService = {
      getSettings: vi.fn().mockResolvedValue({
        general: {
          dataDir: '/test/data',
          gitWorktreeDir: '/test/worktrees',
        },
      }),
    } as any

    mockEventService = {} as any

    // Mock git utilities
    vi.spyOn(gitUtils, 'isGitRepository').mockResolvedValue(true)
    vi.spyOn(gitUtils, 'isGitWorktree').mockResolvedValue(false)
    vi.spyOn(gitUtils, 'createGitWorktree').mockResolvedValue(undefined)
    vi.spyOn(gitUtils, 'getBranchList').mockResolvedValue(['main', 'develop'])

    // Create WorkspaceService instance
    workspaceService = new WorkspaceService(
      mockRepositoryService,
      mockSettingsService,
      mockEventService,
    )

    // Mock private methods
    vi.spyOn(workspaceService as any, 'pathExists').mockResolvedValue(true) // Repository path exists
    vi.spyOn(workspaceService as any, 'checkBranchExists').mockImplementation(
      (repoPath: string, branchName: string) => {
        // Return true for common branches to avoid "branch does not exist" errors
        return Promise.resolve(['main', 'develop', 'master'].includes(branchName))
      },
    )
    vi.spyOn(workspaceService as any, 'getBranchName').mockReturnValue('workspace-task-123')
  })

  afterEach(() => {
    vi.clearAllMocks()
  })

  describe('ensureWorktreeExists with newBranch', () => {
    it('should use newBranch when provided instead of generated branch name', async () => {
      const taskId = 'test-task-123'
      const workspacePath = '/test/worktrees/workspace-task-123'
      const sourceBranch = 'main'
      const newBranch = 'feature/custom-branch'

      // Call the private method through createWorkspace
      await workspaceService.createWorkspace(taskId, 'test-repo', sourceBranch, newBranch)

      // Verify createGitWorktree was called with newBranch as target
      expect(gitUtils.createGitWorktree).toHaveBeenCalledWith(
        '/test/repo/path',
        expect.stringContaining('workspace-task-123'),
        sourceBranch,
        newBranch, // Should use newBranch instead of auto-generated name
      )
    })

    it('should use auto-generated branch name when newBranch not provided', async () => {
      const taskId = 'test-task-123'
      const sourceBranch = 'main'

      await workspaceService.createWorkspace(taskId, 'test-repo', sourceBranch)

      // Verify createGitWorktree was called with auto-generated name
      expect(gitUtils.createGitWorktree).toHaveBeenCalledWith(
        '/test/repo/path',
        expect.stringContaining('workspace-task-123'),
        sourceBranch,
        'workspace-task-123', // Should use auto-generated name
      )
    })

    it('should handle newBranch when no source branch is provided', async () => {
      const taskId = 'test-task-123'
      const newBranch = 'feature/standalone-branch'

      await workspaceService.createWorkspace(taskId, 'test-repo', undefined, newBranch)

      // Verify createGitWorktree was called with newBranch and no source
      expect(gitUtils.createGitWorktree).toHaveBeenCalledWith(
        '/test/repo/path',
        expect.stringContaining('workspace-task-123'),
        undefined, // No source branch
        newBranch, // Use newBranch
      )
    })

    it('should handle existing newBranch gracefully', async () => {
      const taskId = 'test-task-123'
      const newBranch = 'feature/existing-branch'

      // Mock that the newBranch already exists
      vi.spyOn(workspaceService as any, 'checkBranchExists').mockImplementation(
        (repoPath: string, branchName: string) => {
          return Promise.resolve(branchName === newBranch)
        },
      )

      await workspaceService.createWorkspace(taskId, 'test-repo', undefined, newBranch)

      // Should still try to create worktree with existing branch
      expect(gitUtils.createGitWorktree).toHaveBeenCalledWith(
        '/test/repo/path',
        expect.stringContaining('workspace-task-123'),
        newBranch, // Use existing branch as source when it exists
      )
    })

    it('should validate different newBranch name formats', async () => {
      const taskId = 'test-task-123'
      const branchFormats = [
        'feature/new-feature',
        'bugfix/issue-456',
        'hotfix/critical-fix',
        'release/v2.0.0',
        'user/alice/experiment',
        'simple-branch',
        'CAPS-BRANCH',
        'branch-with-numbers-123',
      ]

      for (const branchName of branchFormats) {
        vi.clearAllMocks()

        await workspaceService.createWorkspace(taskId, 'test-repo', 'main', branchName)

        expect(gitUtils.createGitWorktree).toHaveBeenCalledWith(
          '/test/repo/path',
          expect.stringContaining('workspace-task-123'),
          'main',
          branchName,
        )
      }
    })

    it('should handle empty string newBranch by falling back to auto-generated', async () => {
      const taskId = 'test-task-123'
      const newBranch = '' // Empty string

      await workspaceService.createWorkspace(taskId, 'test-repo', 'main', newBranch)

      // Should fallback to auto-generated name since newBranch is falsy
      expect(gitUtils.createGitWorktree).toHaveBeenCalledWith(
        '/test/repo/path',
        expect.stringContaining('workspace-task-123'),
        'main',
        'workspace-task-123', // Auto-generated fallback
      )
    })
  })

  describe('createWorkspace parameter flow', () => {
    it('should pass all parameters correctly through the call chain', async () => {
      const taskId = 'flow-test-123'
      const repositoryId = 'test-repo'
      const sourceBranch = 'develop'
      const newBranch = 'feature/parameter-flow'

      const workspace = await workspaceService.createWorkspace(
        taskId,
        repositoryId,
        sourceBranch,
        newBranch,
      )

      // Verify repository was fetched
      expect(mockRepositoryService.getRepositoryById).toHaveBeenCalledWith(repositoryId)

      // Verify git operations were called with correct parameters
      expect(gitUtils.isGitRepository).toHaveBeenCalledWith('/test/repo/path')
      expect(gitUtils.createGitWorktree).toHaveBeenCalledWith(
        '/test/repo/path',
        expect.stringContaining(taskId),
        sourceBranch,
        newBranch,
      )

      // Verify workspace was created
      expect(workspace).toBeDefined()
      expect(workspace.getWorkspacePath()).toContain(taskId)
    })

    it('should maintain backward compatibility when newBranch is undefined', async () => {
      const taskId = 'backward-compat-123'
      const repositoryId = 'test-repo'
      const sourceBranch = 'main'

      await workspaceService.createWorkspace(taskId, repositoryId, sourceBranch)

      expect(gitUtils.createGitWorktree).toHaveBeenCalledWith(
        '/test/repo/path',
        expect.stringContaining(taskId),
        sourceBranch,
        'workspace-backward-compat-123', // Auto-generated
      )
    })
  })

  describe('Error scenarios with newBranch', () => {
    it('should handle git worktree creation failure with custom branch', async () => {
      const taskId = 'error-test-123'
      const newBranch = 'feature/failing-branch'

      // Mock createGitWorktree to fail
      vi.spyOn(gitUtils, 'createGitWorktree').mockRejectedValue(
        new Error('Failed to create worktree'),
      )

      await expect(
        workspaceService.createWorkspace(taskId, 'test-repo', 'main', newBranch),
      ).rejects.toThrow('Failed to create worktree')

      // Verify it attempted with the right parameters
      expect(gitUtils.createGitWorktree).toHaveBeenCalledWith(
        '/test/repo/path',
        expect.stringContaining(taskId),
        'main',
        newBranch,
      )
    })

    it('should handle repository not found gracefully', async () => {
      vi.spyOn(mockRepositoryService, 'getRepositoryById').mockResolvedValue(null)

      await expect(
        workspaceService.createWorkspace('test-123', 'nonexistent-repo', 'main', 'feature/test'),
      ).rejects.toThrow()

      // Should not attempt to create worktree
      expect(gitUtils.createGitWorktree).not.toHaveBeenCalled()
    })
  })
})
