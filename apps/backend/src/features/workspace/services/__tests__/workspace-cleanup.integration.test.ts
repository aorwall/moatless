import { mkdir, rm, writeFile } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { pathExists } from '../../../../utils/filesystem'
import type { WorkspaceProvisionPlan } from '../../types/provision-plan.types'
import { WorkspaceMaterializer } from '../workspace-materializer.service'

// Mock the git provider token service
const mockTokenService = {
  getTokenForProvider: vi.fn(),
  buildAuthenticatedCloneUrl: vi.fn(),
  detectProvider: vi.fn(),
}

describe('Workspace Cleanup Integration Tests', () => {
  let tempDir: string
  let materializer: WorkspaceMaterializer

  beforeEach(async () => {
    // Create temp directory for tests
    tempDir = await mkdir(path.join(os.tmpdir(), `workspace-cleanup-test-${Date.now()}`), {
      recursive: true,
    }).then((dir) => dir!)

    materializer = new WorkspaceMaterializer(mockTokenService as any)
  })

  afterEach(async () => {
    // Clean up temp directory
    await rm(tempDir, { recursive: true, force: true })
  })

  describe('cleanupWorkspace - Clone Mode', () => {
    it('should clean up clone directory', async () => {
      // Create a fake clone directory with some files
      const clonePath = path.join(tempDir, 'clone-test')
      await mkdir(clonePath, { recursive: true })
      await writeFile(path.join(clonePath, 'test.txt'), 'test content')
      await writeFile(path.join(clonePath, 'another.txt'), 'more content')

      // Verify directory exists
      expect(await pathExists(clonePath)).toBe(true)

      // Create cleanup plan
      const plan: WorkspaceProvisionPlan = {
        mode: 'clone',
        localTargetPath: clonePath,
        setupMetadataHash: 'hash',
      }

      // Clean up
      await materializer.cleanupWorkspace('test-123', plan)

      // Verify clone directory was removed
      expect(await pathExists(clonePath)).toBe(false)
    })

    it('should handle non-existent clone directory gracefully', async () => {
      const clonePath = path.join(tempDir, 'non-existent-clone')

      // Verify directory doesn't exist
      expect(await pathExists(clonePath)).toBe(false)

      // Create cleanup plan
      const plan: WorkspaceProvisionPlan = {
        mode: 'clone',
        localTargetPath: clonePath,
        setupMetadataHash: 'hash',
      }

      // Should not throw
      await expect(materializer.cleanupWorkspace('test-123', plan)).resolves.toBeUndefined()
    })

    it('should clean up nested clone directory structure', async () => {
      // Create a fake clone with nested directories
      const clonePath = path.join(tempDir, 'clone-nested')
      const nestedPath = path.join(clonePath, 'nested', 'deep', 'structure')
      await mkdir(nestedPath, { recursive: true })
      await writeFile(path.join(nestedPath, 'deep-file.txt'), 'nested content')

      // Verify directory exists
      expect(await pathExists(clonePath)).toBe(true)
      expect(await pathExists(nestedPath)).toBe(true)

      // Create cleanup plan
      const plan: WorkspaceProvisionPlan = {
        mode: 'clone',
        localTargetPath: clonePath,
        setupMetadataHash: 'hash',
      }

      // Clean up
      await materializer.cleanupWorkspace('test-123', plan)

      // Verify entire clone directory was removed
      expect(await pathExists(clonePath)).toBe(false)
    })
  })

  describe('cleanupWorkspace - Direct Mode', () => {
    it('should not delete anything in direct mode', async () => {
      // Create a directory that simulates direct mode usage
      const directPath = path.join(tempDir, 'direct-repo')
      await mkdir(directPath, { recursive: true })
      await writeFile(path.join(directPath, 'important.txt'), 'important content')

      // Verify directory exists
      expect(await pathExists(directPath)).toBe(true)

      // Create cleanup plan for direct mode
      const plan: WorkspaceProvisionPlan = {
        mode: 'direct',
        reusePath: directPath,
        setupMetadataHash: 'hash',
      }

      // Clean up (should be no-op)
      await materializer.cleanupWorkspace('test-123', plan)

      // Verify directory still exists (not deleted in direct mode)
      expect(await pathExists(directPath)).toBe(true)
      expect(await pathExists(path.join(directPath, 'important.txt'))).toBe(true)
    })
  })
})
