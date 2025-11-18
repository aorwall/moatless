import { randomUUID } from 'node:crypto'
import type { RepositoryConfig } from '@moatless/types'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { DrizzleDatabase } from '../../../../infrastructure/database/drizzle'
import { RepositoryRepository } from '../repository.repository'

describe('RepositoryRepository Integration', () => {
  let database: DrizzleDatabase
  let repository: RepositoryRepository

  beforeEach(async () => {
    // Create in-memory database
    database = new DrizzleDatabase(':memory:')
    await database.migrate()

    repository = new RepositoryRepository(database)
  })

  afterEach(async () => {
    await database.close()
  })

  it('should save and retrieve a repository', async () => {
    const repo: RepositoryConfig = {
      kind: 'local',
      id: randomUUID(),
      name: 'test-repo',
      description: 'Test repository',
      path: '/path/to/repo',
      useGitWorktrees: false,
      visibility: 'private',
      originalBranch: 'main',
      deleted: false,
      systemPrompt: 'Test prompt',
      codeExecution: {
        setupCommands: ['npm install'],
      },
      previewConfig: {
        serverStartCommands: ['npm start'],
        takeScreenshots: true,
        enableInspector: true,
      },
      createdAt: new Date(),
      updatedAt: new Date(),
    }

    await repository.save(repo)
    const retrieved = await repository.get(repo.id)

    expect(retrieved).toBeDefined()
    expect(retrieved?.id).toBe(repo.id)
    expect(retrieved?.name).toBe(repo.name)
    expect(retrieved?.description).toBe(repo.description)
    expect(retrieved?.codeExecution).toEqual(repo.codeExecution)
    expect(retrieved?.previewConfig).toEqual(repo.previewConfig)
  })

  it('should update an existing repository', async () => {
    const repo: RepositoryConfig = {
      kind: 'local',
      id: randomUUID(),
      name: 'test-repo',
      path: '/path/to/repo',
      useGitWorktrees: false,
      visibility: 'private',
      codeExecution: {},
      createdAt: new Date(),
      updatedAt: new Date(),
    }

    await repository.save(repo)

    // Update the repository
    const updatedRepo = {
      ...repo,
      name: 'updated-repo',
      description: 'Updated description',
      updatedAt: new Date(),
    }

    await repository.save(updatedRepo)
    const retrieved = await repository.get(repo.id)

    expect(retrieved?.name).toBe('updated-repo')
    expect(retrieved?.description).toBe('Updated description')
  })

  it('should return null for non-existent repository', async () => {
    const retrieved = await repository.get('non-existent-id')
    expect(retrieved).toBeNull()
  })

  it('should return null for soft-deleted repository', async () => {
    const repo: RepositoryConfig = {
      kind: 'local',
      id: randomUUID(),
      name: 'test-repo',
      path: '/path/to/repo',
      useGitWorktrees: false,
      visibility: 'private',
      deleted: false,
      codeExecution: {},
      createdAt: new Date(),
      updatedAt: new Date(),
    }

    await repository.save(repo)

    // Soft delete the repository
    await repository.delete(repo.id)

    const retrieved = await repository.get(repo.id)
    expect(retrieved).toBeNull()
  })

  it('should list repository IDs excluding deleted ones', async () => {
    const repo1: RepositoryConfig = {
      kind: 'local',
      id: randomUUID(),
      name: 'repo-1',
      path: '/path/to/repo1',
      useGitWorktrees: false,
      visibility: 'private',
      codeExecution: {},
      createdAt: new Date(),
      updatedAt: new Date(),
    }

    const repo2: RepositoryConfig = {
      kind: 'local',
      id: randomUUID(),
      name: 'repo-2',
      path: '/path/to/repo2',
      useGitWorktrees: false,
      visibility: 'private',
      codeExecution: {},
      createdAt: new Date(),
      updatedAt: new Date(),
    }

    const repo3: RepositoryConfig = {
      kind: 'local',
      id: randomUUID(),
      name: 'repo-3',
      path: '/path/to/repo3',
      useGitWorktrees: false,
      visibility: 'private',
      deleted: true,
      codeExecution: {},
      createdAt: new Date(),
      updatedAt: new Date(),
    }

    await repository.save(repo1)
    await repository.save(repo2)
    await repository.save(repo3)

    const ids = await repository.list()

    expect(ids).toHaveLength(2)
    expect(ids).toContain(repo1.id)
    expect(ids).toContain(repo2.id)
    expect(ids).not.toContain(repo3.id)
  })

  it('should check if repository exists', async () => {
    const repo: RepositoryConfig = {
      kind: 'local',
      id: randomUUID(),
      name: 'test-repo',
      path: '/path/to/repo',
      useGitWorktrees: false,
      visibility: 'private',
      codeExecution: {},
      createdAt: new Date(),
      updatedAt: new Date(),
    }

    await repository.save(repo)

    const exists = await repository.exists(repo.id)
    expect(exists).toBe(true)

    const notExists = await repository.exists('non-existent-id')
    expect(notExists).toBe(false)
  })

  it('should return false for deleted repository in exists check', async () => {
    const repo: RepositoryConfig = {
      kind: 'local',
      id: randomUUID(),
      name: 'test-repo',
      path: '/path/to/repo',
      useGitWorktrees: false,
      visibility: 'private',
      codeExecution: {},
      createdAt: new Date(),
      updatedAt: new Date(),
    }

    await repository.save(repo)
    await repository.delete(repo.id)

    const exists = await repository.exists(repo.id)
    expect(exists).toBe(false)
  })

  it('should get all repositories excluding deleted by default', async () => {
    const repo1: RepositoryConfig = {
      kind: 'local',
      id: randomUUID(),
      name: 'repo-1',
      path: '/path/to/repo1',
      useGitWorktrees: false,
      visibility: 'private',
      codeExecution: {},
      createdAt: new Date(),
      updatedAt: new Date(),
    }

    const repo2: RepositoryConfig = {
      kind: 'local',
      id: randomUUID(),
      name: 'repo-2',
      path: '/path/to/repo2',
      useGitWorktrees: false,
      visibility: 'private',
      deleted: true,
      codeExecution: {},
      createdAt: new Date(),
      updatedAt: new Date(),
    }

    await repository.save(repo1)
    await repository.save(repo2)

    const all = await repository.getAll()

    expect(all).toHaveLength(1)
    expect(all[0]?.id).toBe(repo1.id)
  })

  it('should get all repositories including deleted when requested', async () => {
    const repo1: RepositoryConfig = {
      kind: 'local',
      id: randomUUID(),
      name: 'repo-1',
      path: '/path/to/repo1',
      useGitWorktrees: false,
      visibility: 'private',
      codeExecution: {},
      createdAt: new Date(),
      updatedAt: new Date(),
    }

    const repo2: RepositoryConfig = {
      kind: 'local',
      id: randomUUID(),
      name: 'repo-2',
      path: '/path/to/repo2',
      useGitWorktrees: false,
      visibility: 'private',
      deleted: true,
      codeExecution: {},
      createdAt: new Date(),
      updatedAt: new Date(),
    }

    await repository.save(repo1)
    await repository.save(repo2)

    const all = await repository.getAll(true)

    expect(all).toHaveLength(2)
  })

  it('should handle complex nested objects in JSON fields', async () => {
    const repo: RepositoryConfig = {
      kind: 'local',
      id: randomUUID(),
      name: 'test-repo',
      path: '/path/to/repo',
      useGitWorktrees: false,
      visibility: 'private',
      codeExecution: {
        dockerImage: 'node:20',
        setupCommands: ['npm install', 'npm build'],
        envVars: {
          NODE_ENV: 'production',
          DEBUG: 'true',
        },
      },
      previewConfig: {
        serverStartCommands: ['npm start'],
        takeScreenshots: true,
        buildCommand: 'npm run build',
        buildOutputDirectory: 'dist',
        enableInspector: true,
        startupTimeoutSeconds: 60,
      },
      createdAt: new Date(),
      updatedAt: new Date(),
    }

    await repository.save(repo)
    const retrieved = await repository.get(repo.id)

    expect(retrieved?.codeExecution).toEqual(repo.codeExecution)
    expect(retrieved?.previewConfig).toEqual(repo.previewConfig)
  })
})
