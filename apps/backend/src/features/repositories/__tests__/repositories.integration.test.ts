import type { RemoteRepository } from '@moatless/types'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { build } from '../../../test/helper'

describe.skipIf(!process.env['DATABASE_URL'])('Repository Routes', () => {
  let app: Awaited<ReturnType<typeof build>>

  beforeEach(async () => {
    app = await build()
  })

  afterEach(async () => {
    // Hono doesn't need cleanup
  })

  describe('POST /api/repositories', () => {
    it('should create a new repository', async () => {
      const { appContainer } = await import('../../../container/container')
      const { RepositoryService } = await import('../services/repository.service')
      const repositoryService = appContainer.get(RepositoryService)

      const mockRepository = {
        kind: 'remote' as const,
        id: 'test-env-id',
        name: 'test-repository',
        description: 'A test repository',
        remote: {
          url: 'https://github.com/test/repo',
          provider: 'github' as const,
        },
        visibility: 'private' as const,
        codeExecution: {},
        createdAt: new Date(),
        updatedAt: new Date(),
      }

      vi.spyOn(repositoryService, 'createRepository').mockResolvedValueOnce(mockRepository)

      const response = await app.fetch(
        new Request('http://localhost/api/v1/repositories', {
          method: 'POST',
          headers: {
            'Content-Type': 'application/json',
          },
          body: JSON.stringify({
            kind: 'remote',
            name: 'test-repository',
            description: 'A test repository',
            remoteUrl: 'https://github.com/test/repo',
            provider: 'github',
            codeExecution: {},
          }),
        }),
      )

      expect(response.status).toBe(200)
      const body = (await response.json()) as any
      expect(body.name).toBe('test-repository')
      expect(body.id).toBe('test-env-id')
      expect(body.createdAt).toBeDefined()
    })

    it('should validate required fields', async () => {
      const response = await app.fetch(
        new Request('http://localhost/api/v1/repositories', {
          method: 'POST',
          headers: {
            'Content-Type': 'application/json',
          },
          body: JSON.stringify({
            // Missing required fields
          }),
        }),
      )

      expect(response.status).toBe(400)
    })
  })

  describe('GET /api/repositories', () => {
    it('should list all repositories', async () => {
      const { appContainer } = await import('../../../container/container')
      const { RepositoryService } = await import('../services/repository.service')
      const repositoryService = appContainer.get(RepositoryService)

      const mockRepositories = [
        {
          kind: 'remote' as const,
          id: 'repo-1',
          name: 'Repository 1',
          remote: {
            url: 'https://github.com/test/repo1',
            provider: 'github' as const,
          },
          visibility: 'private' as const,
          codeExecution: {},
          createdAt: new Date(),
          updatedAt: new Date(),
        },
        {
          kind: 'remote' as const,
          id: 'repo-2',
          name: 'Repository 2',
          remote: {
            url: 'https://github.com/test/repo2',
            provider: 'github' as const,
          },
          visibility: 'private' as const,
          codeExecution: {},
          createdAt: new Date(),
          updatedAt: new Date(),
        },
      ]

      vi.spyOn(repositoryService, 'listRepositories').mockResolvedValueOnce(mockRepositories)

      const response = await app.fetch(
        new Request('http://localhost/api/v1/repositories', {
          method: 'GET',
        }),
      )

      expect(response.status).toBe(200)
      const body = (await response.json()) as any
      expect(body.repositories).toHaveLength(2)
    })
  })

  describe('GET /api/repositories/:repositoryId', () => {
    it('should return repository details', async () => {
      const { appContainer } = await import('../../../container/container')
      const { RepositoryService } = await import('../services/repository.service')
      const repositoryService = appContainer.get(RepositoryService)

      const mockRepository: RemoteRepository = {
        kind: 'remote',
        id: 'env-123',
        name: 'Test Repository',
        remote: {
          url: 'https://github.com/test/repo',
          provider: 'github',
        },
        visibility: 'private',
        codeExecution: {},
        createdAt: new Date(),
        updatedAt: new Date(),
      }

      vi.spyOn(repositoryService, 'getRepositoryById').mockResolvedValueOnce(mockRepository)

      const response = await app.fetch(
        new Request('http://localhost/api/v1/repositories/env-123', {
          method: 'GET',
        }),
      )

      expect(response.status).toBe(200)
      const body = (await response.json()) as any
      expect(body.id).toBe('env-123')
      expect(body.name).toBe('Test Repository')
    })

    it('should return 404 for non-existent repository', async () => {
      const { appContainer } = await import('../../../container/container')
      const { RepositoryService } = await import('../services/repository.service')
      const repositoryService = appContainer.get(RepositoryService)

      vi.spyOn(repositoryService, 'getRepositoryById').mockResolvedValueOnce(null)

      const response = await app.fetch(
        new Request('http://localhost/api/v1/repositories/non-existent', {
          method: 'GET',
        }),
      )

      expect(response.status).toBe(404)
      const body = (await response.json()) as any
      expect(body.error.code).toBe('REPOSITORY_NOT_FOUND')
    })

    it('should return 404 for soft-deleted repository', async () => {
      const { appContainer } = await import('../../../container/container')
      const { RepositoryService } = await import('../services/repository.service')
      const repositoryService = appContainer.get(RepositoryService)

      // The service now returns null for deleted repositories
      vi.spyOn(repositoryService, 'getRepositoryById').mockResolvedValueOnce(null)

      const response = await app.fetch(
        new Request('http://localhost/api/v1/repositories/deleted-repo', {
          method: 'GET',
        }),
      )

      expect(response.status).toBe(404)
      const body = (await response.json()) as any
      expect(body.error.code).toBe('REPOSITORY_NOT_FOUND')
    })
  })

  describe('PUT /api/repositories/:repositoryId', () => {
    it('should update repository', async () => {
      const { appContainer } = await import('../../../container/container')
      const { RepositoryService } = await import('../services/repository.service')
      const repositoryService = appContainer.get(RepositoryService)

      const updatedEnvironment = {
        kind: 'remote' as const,
        id: 'env-123',
        name: 'updated-name',
        description: 'Updated description',
        remote: {
          url: 'https://github.com/test/repo',
          provider: 'github' as const,
        },
        visibility: 'private' as const,
        codeExecution: {},
        createdAt: new Date(),
        updatedAt: new Date(),
      }

      vi.spyOn(repositoryService, 'updateRepository').mockResolvedValueOnce(updatedEnvironment)

      // Mock getRepositoryById call that happens after update
      vi.spyOn(repositoryService, 'getRepositoryById').mockResolvedValueOnce(updatedEnvironment)

      const response = await app.fetch(
        new Request('http://localhost/api/v1/repositories/env-123', {
          method: 'PUT',
          headers: {
            'Content-Type': 'application/json',
          },
          body: JSON.stringify({
            name: 'updated-name',
            description: 'Updated description',
          }),
        }),
      )

      expect(response.status).toBe(200)
      const body = (await response.json()) as any
      expect(body.name).toBe('updated-name')
      expect(body.description).toBe('Updated description')
    })
  })

  describe('DELETE /api/repositories/:repositoryId', () => {
    it('should delete repository', async () => {
      const { appContainer } = await import('../../../container/container')
      const { RepositoryService } = await import('../services/repository.service')
      const repositoryService = appContainer.get(RepositoryService)

      vi.spyOn(repositoryService, 'deleteRepository').mockResolvedValueOnce()

      const response = await app.fetch(
        new Request('http://localhost/api/v1/repositories/env-123', {
          method: 'DELETE',
        }),
      )

      expect(response.status).toBe(204)
    })
  })

  describe('POST /api/repositories/:repositoryId/search-files', () => {
    it('should search files in repository', async () => {
      const { appContainer } = await import('../../../container/container')
      const { RepositoryService } = await import('../services/repository.service')
      const repositoryService = appContainer.get(RepositoryService)

      const mockRepository: RemoteRepository = {
        kind: 'remote',
        id: 'env-123',
        name: 'Test Repository',
        remote: {
          url: 'https://github.com/test/repo',
          provider: 'github',
        },
        visibility: 'private',
        codeExecution: {},
        createdAt: new Date(),
        updatedAt: new Date(),
      }

      vi.spyOn(repositoryService, 'getRepositoryById').mockResolvedValueOnce(mockRepository)

      const response = await app.fetch(
        new Request('http://localhost/api/v1/repositories/env-123/search-files', {
          method: 'POST',
          headers: {
            'Content-Type': 'application/json',
          },
          body: JSON.stringify({
            query: 'test',
            limit: 50,
          }),
        }),
      )

      expect(response.status).toBe(200)
      const body = (await response.json()) as any
      expect(body.results).toBeDefined()
      expect(body.totalFound).toBeDefined()
      expect(body.query).toBeDefined()
    })

    it('should validate search query', async () => {
      const response = await app.fetch(
        new Request('http://localhost/api/v1/repositories/env-123/search-files', {
          method: 'POST',
          headers: {
            'Content-Type': 'application/json',
          },
          body: JSON.stringify({
            query: '', // Empty query
          }),
        }),
      )

      expect(response.status).toBe(400)
    })
  })

  describe('GET /api/repositories/:repositoryId/branches', () => {
    it('should get branches', async () => {
      const { appContainer } = await import('../../../container/container')
      const { RepositoryService } = await import('../services/repository.service')
      const repositoryService = appContainer.get(RepositoryService)

      const mockRepository = {
        kind: 'remote' as const,
        id: 'repo-123',
        name: 'Test Repository',
        remote: {
          url: 'https://github.com/test/repo',
          provider: 'github' as const,
        },
        visibility: 'private' as const,
        codeExecution: {},
        createdAt: new Date(),
        updatedAt: new Date(),
      }

      vi.spyOn(repositoryService, 'getRepositoryById').mockResolvedValueOnce(mockRepository)

      const response = await app.fetch(
        new Request('http://localhost/api/v1/repositories/env-123/branches', {
          method: 'GET',
        }),
      )

      expect(response.status).toBe(200)
      const body = (await response.json()) as any
      expect(body.branches).toBeDefined()
      expect(Array.isArray(body.branches)).toBe(true)
    })
  })
})
