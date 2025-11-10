import { describe, expect, it } from 'vitest'
import type { RepositorySelect } from '../repository.schema'
import { selectToRepository } from '../repository.schema'

describe('repository.schema', () => {
  describe('selectToRepository', () => {
    it('should handle TEXT timestamps (ISO strings) from database', () => {
      const row: RepositorySelect = {
        id: 'test-id',
        kind: 'template',
        name: 'test-repo',
        description: 'Test repository',
        path: null,
        remoteUrl: 'https://github.com/test/repo',
        provider: 'github',
        defaultBranch: 'main',
        templateId: null,
        useGitWorktrees: null,
        originalBranch: null,
        sourceType: null,
        cloneOnNewTask: null,
        deleted: false,
        vibe: null,
        syncWithRepoConfig: false,
        systemPrompt: null,
        setupCommands: null,
        dockerImage: null,
        envVars: null,
        serverStartCommands: null,
        buildCommand: null,
        buildOutputDirectory: 'dist',
        takeScreenshots: null,
        enableInspector: true,
        startupTimeoutSeconds: 30,
        autoReload: null,
        // Simulate TEXT timestamps from database (as returned by better-sqlite3)
        createdAt: '2025-11-09 17:11:04' as unknown as Date,
        updatedAt: '2025-11-09 17:11:04' as unknown as Date,
      }

      const result = selectToRepository(row)

      expect(result).toBeDefined()
      expect(result.id).toBe('test-id')
      expect(result.name).toBe('test-repo')
      expect(result.kind).toBe('template')

      // Verify dates are valid Date objects
      expect(result.createdAt).toBeInstanceOf(Date)
      expect(result.updatedAt).toBeInstanceOf(Date)
      expect(result.createdAt.getTime()).not.toBeNaN()
      expect(result.updatedAt.getTime()).not.toBeNaN()
    })

    it('should handle INTEGER timestamps (epoch ms) from database', () => {
      const now = Date.now()
      const row: RepositorySelect = {
        id: 'test-id',
        kind: 'local',
        name: 'test-repo',
        description: 'Test repository',
        path: '/path/to/repo',
        remoteUrl: null,
        provider: null,
        defaultBranch: null,
        templateId: null,
        useGitWorktrees: false,
        originalBranch: null,
        sourceType: null,
        cloneOnNewTask: null,
        deleted: false,
        vibe: null,
        syncWithRepoConfig: false,
        systemPrompt: null,
        setupCommands: null,
        dockerImage: null,
        envVars: null,
        serverStartCommands: null,
        buildCommand: null,
        buildOutputDirectory: 'dist',
        takeScreenshots: null,
        enableInspector: true,
        startupTimeoutSeconds: 30,
        autoReload: null,
        // Simulate INTEGER timestamps from database (as Date objects with timestamp mode)
        createdAt: new Date(now),
        updatedAt: new Date(now),
      }

      const result = selectToRepository(row)

      expect(result).toBeDefined()
      expect(result.id).toBe('test-id')
      expect(result.name).toBe('test-repo')
      expect(result.kind).toBe('local')

      // Verify dates are valid Date objects
      expect(result.createdAt).toBeInstanceOf(Date)
      expect(result.updatedAt).toBeInstanceOf(Date)
      expect(result.createdAt.getTime()).toBe(now)
      expect(result.updatedAt.getTime()).toBe(now)
    })

    it('should correctly reconstruct a template repository', () => {
      const row: RepositorySelect = {
        id: 'hello-world-ui',
        kind: 'template',
        name: 'Hello World UI',
        description: null,
        path: null,
        remoteUrl: 'https://github.com/test/hello-world',
        provider: 'github',
        defaultBranch: 'main',
        templateId: null,
        useGitWorktrees: null,
        originalBranch: null,
        sourceType: null,
        cloneOnNewTask: null,
        deleted: false,
        vibe: null,
        syncWithRepoConfig: false,
        systemPrompt: null,
        setupCommands: null,
        dockerImage: null,
        envVars: null,
        serverStartCommands: null,
        buildCommand: null,
        buildOutputDirectory: 'dist',
        takeScreenshots: null,
        enableInspector: true,
        startupTimeoutSeconds: 30,
        autoReload: null,
        createdAt: '2025-11-09T17:11:04.000Z' as unknown as Date,
        updatedAt: '2025-11-09T17:11:04.000Z' as unknown as Date,
      }

      const result = selectToRepository(row)

      expect(result.kind).toBe('template')
      expect(result.id).toBe('hello-world-ui')
      expect(result.name).toBe('Hello World UI')

      if (result.kind === 'template') {
        expect(result.remote.url).toBe('https://github.com/test/hello-world')
        expect(result.remote.provider).toBe('github')
        expect(result.remote.defaultBranch).toBe('main')
      }

      // Most importantly: dates should be valid
      expect(result.createdAt).toBeInstanceOf(Date)
      expect(result.updatedAt).toBeInstanceOf(Date)
      expect(Number.isNaN(result.createdAt.getTime())).toBe(false)
      expect(Number.isNaN(result.updatedAt.getTime())).toBe(false)
    })
  })
})
