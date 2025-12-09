import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { WorkspaceProvisionPlan } from '../../types/provision-plan.types'
import { WorkspaceMaterializer } from '../workspace-materializer.service'

// Mock the filesystem module
vi.mock('../../../../utils/filesystem.js', () => ({
  removeDirectory: vi.fn().mockResolvedValue(undefined),
}))

// Mock the git provider token service
const mockTokenService = {
  getTokenForProvider: vi.fn(),
  buildAuthenticatedCloneUrl: vi.fn(),
  detectProvider: vi.fn(),
}

describe('WorkspaceMaterializer - Cleanup', () => {
  let materializer: WorkspaceMaterializer

  beforeEach(() => {
    vi.clearAllMocks()
    materializer = new WorkspaceMaterializer(mockTokenService as any)
  })

  describe('cleanupWorkspace', () => {
    it('should skip cleanup for direct mode', async () => {
      const plan: WorkspaceProvisionPlan = {
        mode: 'direct',
        reusePath: '/path/to/repo',
        setupMetadataHash: 'hash123',
      }

      // Should not throw
      await expect(materializer.cleanupWorkspace('task-123', plan)).resolves.toBeUndefined()

      // Verify removeDirectory was not called
      const { removeDirectory } = await import('../../../../utils/filesystem.js')
      expect(removeDirectory).not.toHaveBeenCalled()
    })

    it('should skip cleanup for worktree mode (not implemented yet)', async () => {
      const plan: WorkspaceProvisionPlan = {
        mode: 'worktree',
        reusePath: '/path/to/repo',
        localTargetPath: '/path/to/worktree',
        branch: 'workspace-task-123',
        setupMetadataHash: 'hash123',
      }

      // Should not throw
      await expect(materializer.cleanupWorkspace('task-123', plan)).resolves.toBeUndefined()

      // Verify removeDirectory was not called
      const { removeDirectory } = await import('../../../../utils/filesystem.js')
      expect(removeDirectory).not.toHaveBeenCalled()
    })

    it('should call removeDirectory for clone mode', async () => {
      const plan: WorkspaceProvisionPlan = {
        mode: 'clone',
        localTargetPath: '/path/to/clone',
        remoteUrl: 'https://github.com/org/repo',
        setupMetadataHash: 'hash123',
      }

      await materializer.cleanupWorkspace('task-123', plan)

      // Verify removeDirectory was called with correct path
      const { removeDirectory } = await import('../../../../utils/filesystem.js')
      expect(removeDirectory).toHaveBeenCalledWith('/path/to/clone')
    })

    it('should handle missing localTargetPath in clone mode', async () => {
      const plan: WorkspaceProvisionPlan = {
        mode: 'clone',
        setupMetadataHash: 'hash123',
      }

      // Should not throw
      await expect(materializer.cleanupWorkspace('task-123', plan)).resolves.toBeUndefined()

      // Verify removeDirectory was not called
      const { removeDirectory } = await import('../../../../utils/filesystem.js')
      expect(removeDirectory).not.toHaveBeenCalled()
    })

    it('should not throw on cleanup errors', async () => {
      const plan: WorkspaceProvisionPlan = {
        mode: 'clone',
        localTargetPath: '/path/to/clone',
        setupMetadataHash: 'hash123',
      }

      // Mock removeDirectory to throw error
      const { removeDirectory } = await import('../../../../utils/filesystem.js')
      const mockedRemoveDirectory = removeDirectory as any
      mockedRemoveDirectory.mockRejectedValueOnce(new Error('Permission denied'))

      // Should log error but not throw
      await expect(materializer.cleanupWorkspace('task-123', plan)).resolves.toBeUndefined()
    })
  })
})
