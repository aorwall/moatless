import { randomUUID } from 'node:crypto'
import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import type { RepositoryConfig } from '@moatless/types'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { FileStorage } from '../../storage/file-storage.js'
import { RepositoryService } from '../repository.service.js'

describe('RepositoryService - Update with null fields', () => {
  let storage: FileStorage
  let service: RepositoryService
  let testDir: string
  let testRepoId: string

  beforeEach(async () => {
    // Create temporary directory for test data
    testDir = await mkdtemp(join(tmpdir(), 'repo-update-test-'))
    storage = new FileStorage(testDir)
    service = new RepositoryService(storage)

    // Create a test repository with all optional fields populated
    const initialRepo: RepositoryConfig = {
      id: randomUUID(),
      name: 'test-repo',
      description: 'Initial description',
      path: '/tmp/test-repo',
      remoteUrl: 'https://github.com/test/repo',
      sourceType: 'github',
      useGitWorktrees: false,
      cloneOnNewTask: false,
      systemPrompt: 'Initial system prompt',
      originalBranch: 'main',
      codeExecution: { type: 'local' },
      previewConfig: { enableInspector: true },
      createdAt: new Date(),
      updatedAt: new Date(),
    }

    await storage.saveRepository(initialRepo)
    testRepoId = initialRepo.id
  })

  afterEach(async () => {
    // Clean up test directory
    await rm(testDir, { recursive: true, force: true })
  })

  it('should clear optional string fields when null is provided', async () => {
    // Update with null values for optional fields
    const updated = await service.updateRepository(testRepoId, {
      description: null,
      remoteUrl: null,
      systemPrompt: null,
      originalBranch: null,
    })

    // Verify fields are cleared (undefined)
    expect(updated.description).toBeUndefined()
    expect(updated.remoteUrl).toBeUndefined()
    expect(updated.systemPrompt).toBeUndefined()
    expect(updated.originalBranch).toBeUndefined()
  })

  it('should maintain existing values when fields are not provided', async () => {
    const original = await service.getRepositoryById(testRepoId)

    // Update only name, don't touch other fields
    const updated = await service.updateRepository(testRepoId, {
      name: 'updated-name',
    })

    // Verify original optional fields are maintained
    expect(updated.name).toBe('updated-name')
    expect(updated.description).toBe(original?.description)
    expect(updated.remoteUrl).toBe(original?.remoteUrl)
    expect(updated.systemPrompt).toBe(original?.systemPrompt)
  })

  it('should accept complete config with null for cleared fields', async () => {
    const original = await service.getRepositoryById(testRepoId)

    // This mimics what the frontend sends: complete config with null for cleared fields
    const payload = {
      name: 'updated-repo',
      description: null, // Cleared
      remoteUrl: null, // Cleared
      useGitWorktrees: false,
      cloneOnNewTask: false,
      systemPrompt: null, // Cleared
      codeExecution: { type: 'local' as const },
      previewConfig: { enableInspector: true },
    }

    const updated = await service.updateRepository(testRepoId, payload)

    expect(updated.name).toBe('updated-repo')
    expect(updated.description).toBeUndefined()
    expect(updated.remoteUrl).toBeUndefined()
    expect(updated.systemPrompt).toBeUndefined()
    expect(updated.path).toBe(original?.path) // Path unchanged
    expect(updated.useGitWorktrees).toBe(false)
  })

  it('should persist cleared fields when saved and retrieved', async () => {
    // Clear fields
    await service.updateRepository(testRepoId, {
      description: null,
      systemPrompt: null,
    })

    // Retrieve from storage
    const retrieved = await service.getRepositoryById(testRepoId)

    // Verify fields remain cleared
    expect(retrieved?.description).toBeUndefined()
    expect(retrieved?.systemPrompt).toBeUndefined()
  })

  it('should write config to file when syncWithRepoConfig is enabled', async () => {
    const original = await service.getRepositoryById(testRepoId)

    // Enable sync and update systemPrompt
    await service.updateRepository(testRepoId, {
      syncWithRepoConfig: true,
      systemPrompt: 'Updated system prompt',
    })

    // Read config file
    const configFile = await service.readRepoConfigFile(original!.path)

    // Verify config file was created with updated values
    expect(configFile).toBeDefined()
    expect(configFile?.systemPrompt).toBe('Updated system prompt')
    expect(configFile?.codeExecution).toEqual(original?.codeExecution)
  })

  it('should write cleared values to config file when sync is enabled', async () => {
    const original = await service.getRepositoryById(testRepoId)

    // Enable sync and clear systemPrompt
    await service.updateRepository(testRepoId, {
      syncWithRepoConfig: true,
      systemPrompt: null,
    })

    // Read config file
    const configFile = await service.readRepoConfigFile(original!.path)

    // Verify config file has undefined systemPrompt (field omitted in JSON)
    expect(configFile).toBeDefined()
    expect(configFile?.systemPrompt).toBeUndefined()
  })
})
