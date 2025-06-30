import type { FastifyInstance } from 'fastify'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { build } from '../../test/helper.js'

describe('Repository Routes', () => {
  let app: FastifyInstance

  beforeEach(async () => {
    app = await build()
  })

  afterEach(async () => {
    await app.close()
  })

  describe('POST /api/repositories', () => {
    it('should create a new repository', async () => {
      const mockRepository = {
        id: 'test-env-id',
        name: 'Test Repository',
        description: 'A test repository',
        repository: { type: 'local' as const, path: '/test/path' },
        codeExecution: { type: 'local' as const },
        variables: {},
        createdAt: new Date(),
        updatedAt: new Date(),
      }

      vi.spyOn(app.services.repository, 'createRepository').mockResolvedValueOnce(mockRepository)

      const response = await app.inject({
        method: 'POST',
        url: '/api/v1/repositories',
        payload: {
          name: 'Test Repository',
          description: 'A test repository',
          repository: {
            type: 'local',
            path: '/test/path',
          },
          codeExecution: {
            type: 'local',
          },
        },
      })

      expect(response.statusCode).toBe(200)
      const body = JSON.parse(response.body)
      expect(body.name).toBe('Test Repository')
      expect(body.id).toBe('test-env-id')
      expect(body.createdAt).toBeDefined()
    })

    it('should validate required fields', async () => {
      const response = await app.inject({
        method: 'POST',
        url: '/api/v1/repositories',
        payload: {
          // Missing required fields
        },
      })

      expect(response.statusCode).toBe(400)
    })
  })

  describe('GET /api/repositories', () => {
    it('should list all repositories', async () => {
      const mockRepositories = [
        {
          id: 'repo-1',
          name: 'Repository 1',
          repository: { type: 'local' as const, path: '/path1' },
          codeExecution: { type: 'local' as const },
          variables: {},
          createdAt: new Date(),
          updatedAt: new Date(),
        },
        {
          id: 'repo-2',
          name: 'Repository 2',
          repository: { type: 'local' as const, path: '/path2' },
          codeExecution: { type: 'local' as const },
          variables: {},
          createdAt: new Date(),
          updatedAt: new Date(),
        },
      ]

      vi.spyOn(app.services.repository, 'listRepositories').mockResolvedValueOnce(mockRepositories)

      const response = await app.inject({
        method: 'GET',
        url: '/api/v1/repositories',
      })

      expect(response.statusCode).toBe(200)
      const body = JSON.parse(response.body)
      expect(body.repositories).toHaveLength(2)
    })
  })

  describe('GET /api/repositories/:repositoryId', () => {
    it('should return repository details', async () => {
      const mockRepository = {
        id: 'env-123',
        name: 'Test Repository',
        repository: { type: 'local' as const, path: '/test/path' },
        codeExecution: { type: 'local' as const },
        variables: {},
        createdAt: new Date(),
        updatedAt: new Date(),
      }

      vi.spyOn(app.services.repository, 'getRepositoryById').mockResolvedValueOnce(mockRepository)

      const response = await app.inject({
        method: 'GET',
        url: '/api/v1/repositories/env-123',
      })

      expect(response.statusCode).toBe(200)
      const body = JSON.parse(response.body)
      expect(body.id).toBe('env-123')
      expect(body.name).toBe('Test Repository')
    })

    it('should return 404 for non-existent repository', async () => {
      vi.spyOn(app.services.repository, 'getRepositoryById').mockResolvedValueOnce(null)

      const response = await app.inject({
        method: 'GET',
        url: '/api/v1/repositories/non-existent',
      })

      expect(response.statusCode).toBe(404)
      const body = JSON.parse(response.body)
      expect(body.error.code).toBe('REPOSITORY_NOT_FOUND')
    })
  })

  describe('PUT /api/repositories/:repositoryId', () => {
    it('should update repository', async () => {
      const updatedEnvironment = {
        id: 'env-123',
        name: 'Updated Name',
        description: 'Updated description',
        repository: { type: 'local' as const, path: '/test/path' },
        codeExecution: { type: 'local' as const },
        variables: {},
        createdAt: new Date(),
        updatedAt: new Date(),
      }

      vi.spyOn(app.services.repository, 'updateRepository').mockResolvedValueOnce(
        updatedEnvironment,
      )

      // Mock getRepositoryById call that happens after update
      vi.spyOn(app.services.repository, 'getRepositoryById').mockResolvedValueOnce(
        updatedEnvironment,
      )

      const response = await app.inject({
        method: 'PUT',
        url: '/api/v1/repositories/env-123',
        payload: {
          name: 'Updated Name',
          description: 'Updated description',
        },
      })

      expect(response.statusCode).toBe(200)
      const body = JSON.parse(response.body)
      expect(body.name).toBe('Updated Name')
      expect(body.description).toBe('Updated description')
    })
  })

  describe('DELETE /api/repositories/:repositoryId', () => {
    it('should delete repository', async () => {
      vi.spyOn(app.services.repository, 'deleteRepository').mockResolvedValueOnce()

      const response = await app.inject({
        method: 'DELETE',
        url: '/api/v1/repositories/env-123',
      })

      expect(response.statusCode).toBe(200)
      const body = JSON.parse(response.body)
      expect(body.status).toBe('deleted')
    })
  })

  describe('POST /api/repositories/:repositoryId/search-files', () => {
    it('should search files in repository', async () => {
      const mockRepository = {
        id: 'env-123',
        name: 'Test Repository',
        repository: { type: 'local' as const, path: '/test/path' },
        codeExecution: { type: 'local' as const },
        variables: {},
        createdAt: new Date(),
        updatedAt: new Date(),
      }

      vi.spyOn(app.services.repository, 'getRepositoryById').mockResolvedValueOnce(mockRepository)
      vi.spyOn(app.services.workspace, 'searchFiles').mockResolvedValueOnce({
        results: [
          { path: '/test/file1.ts', absolutePath: '/test/file1.ts', score: 1 },
          { path: '/test/file2.ts', absolutePath: '/test/file2.ts', score: 2 },
        ],
        totalFound: 2,
        query: 'test',
      })

      const response = await app.inject({
        method: 'POST',
        url: '/api/v1/repositories/env-123/search-files',
        payload: {
          query: 'test',
          filePattern: '*.ts',
          maxResults: 50,
        },
      })

      expect(response.statusCode).toBe(200)
      const body = JSON.parse(response.body)
      expect(body.results).toBeDefined()
      expect(body.totalFound).toBeDefined()
      expect(body.query).toBeDefined()
    })

    it('should validate search query', async () => {
      const response = await app.inject({
        method: 'POST',
        url: '/api/v1/repositories/env-123/search-files',
        payload: {
          query: '', // Empty query
        },
      })

      expect(response.statusCode).toBe(400)
    })
  })

  describe('POST /api/repositories/:repositoryId/run-setup', () => {
    it('should run setup script', async () => {
      const mockRepository = {
        id: 'env-123',
        name: 'Test Repository',
        repository: { type: 'local' as const, path: '/test/path' },
        codeExecution: { type: 'local' as const, setupCommands: 'echo "test"' },
        variables: {},
        createdAt: new Date(),
        updatedAt: new Date(),
      }

      vi.spyOn(app.services.repository, 'getRepositoryById').mockResolvedValueOnce(mockRepository)

      const response = await app.inject({
        method: 'POST',
        url: '/api/v1/repositories/env-123/run-setup',
      })

      expect(response.statusCode).toBe(200)
      const body = JSON.parse(response.body)
      expect(body.success).toBeDefined()
      expect(body.output).toBeDefined()
    })
  })

  describe('GET /api/repositories/:repositoryId/branches', () => {
    it('should get branches', async () => {
      const mockRepository = {
        id: 'repo-123',
        name: 'Test Repository',
        repository: { type: 'local' as const, path: '/test/repo' },
        codeExecution: { type: 'local' as const },
        variables: {},
        createdAt: new Date(),
        updatedAt: new Date(),
      }

      vi.spyOn(app.services.repository, 'getRepositoryById').mockResolvedValueOnce(mockRepository)

      const response = await app.inject({
        method: 'GET',
        url: '/api/v1/repositories/env-123/branches',
      })

      expect(response.statusCode).toBe(200)
      const body = JSON.parse(response.body)
      expect(body.branches).toBeDefined()
      expect(Array.isArray(body.branches)).toBe(true)
    })
  })
})
