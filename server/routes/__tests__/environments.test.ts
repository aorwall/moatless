import type { FastifyInstance } from 'fastify'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { build } from '../../test/helper.js'

describe('Environment Routes', () => {
  let app: FastifyInstance

  beforeEach(async () => {
    app = await build()
  })

  afterEach(async () => {
    await app.close()
  })

  describe('POST /api/environments', () => {
    it('should create a new environment', async () => {
      const mockEnvironment = {
        id: 'test-env-id',
        name: 'Test Environment',
        description: 'A test environment',
        repository: { type: 'local' as const, path: '/test/path' },
        codeExecution: { type: 'local' as const },
        variables: {},
        createdAt: new Date(),
        updatedAt: new Date(),
      }

      vi.spyOn(app.services.environment, 'createEnvironment').mockResolvedValueOnce(mockEnvironment)

      const response = await app.inject({
        method: 'POST',
        url: '/api/v1/environments',
        payload: {
          name: 'Test Environment',
          description: 'A test environment',
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
      expect(body.name).toBe('Test Environment')
      expect(body.id).toBe('test-env-id')
      expect(body.createdAt).toBeDefined()
    })

    it('should validate required fields', async () => {
      const response = await app.inject({
        method: 'POST',
        url: '/api/v1/environments',
        payload: {
          // Missing required fields
        },
      })

      expect(response.statusCode).toBe(400)
    })
  })

  describe('GET /api/environments', () => {
    it('should list all environments', async () => {
      const mockEnvironments = [
        {
          id: 'env-1',
          name: 'Environment 1',
          repository: { type: 'local' as const, path: '/path1' },
          codeExecution: { type: 'local' as const },
          variables: {},
          createdAt: new Date(),
          updatedAt: new Date(),
        },
        {
          id: 'env-2',
          name: 'Environment 2',
          repository: { type: 'github' as const, repoId: 'owner/repo', branch: 'main' },
          codeExecution: { type: 'docker' as const, image: 'node:18' },
          variables: {},
          createdAt: new Date(),
          updatedAt: new Date(),
        },
      ]

      vi.spyOn(app.services.environment, 'listEnvironments').mockResolvedValueOnce(mockEnvironments)

      const response = await app.inject({
        method: 'GET',
        url: '/api/v1/environments',
      })

      expect(response.statusCode).toBe(200)
      const body = JSON.parse(response.body)
      expect(body.environments).toHaveLength(2)
    })
  })

  describe('GET /api/environments/:environmentId', () => {
    it('should return environment details', async () => {
      const mockEnvironment = {
        id: 'env-123',
        name: 'Test Environment',
        repository: { type: 'local' as const, path: '/test/path' },
        codeExecution: { type: 'local' as const },
        variables: {},
        createdAt: new Date(),
        updatedAt: new Date(),
      }

      vi.spyOn(app.services.environment, 'getEnvironment').mockResolvedValueOnce(mockEnvironment)

      const response = await app.inject({
        method: 'GET',
        url: '/api/v1/environments/env-123',
      })

      expect(response.statusCode).toBe(200)
      const body = JSON.parse(response.body)
      expect(body.id).toBe('env-123')
      expect(body.name).toBe('Test Environment')
    })

    it('should return 404 for non-existent environment', async () => {
      vi.spyOn(app.services.environment, 'getEnvironment').mockResolvedValueOnce(null)

      const response = await app.inject({
        method: 'GET',
        url: '/api/v1/environments/non-existent',
      })

      expect(response.statusCode).toBe(404)
      const body = JSON.parse(response.body)
      expect(body.error.code).toBe('ENVIRONMENT_NOT_FOUND')
    })
  })

  describe('PUT /api/environments/:environmentId', () => {
    it('should update environment', async () => {
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

      vi.spyOn(app.services.environment, 'updateEnvironment').mockResolvedValueOnce(
        updatedEnvironment,
      )

      // Mock getEnvironment call that happens after update
      vi.spyOn(app.services.environment, 'getEnvironment').mockResolvedValueOnce(updatedEnvironment)

      const response = await app.inject({
        method: 'PUT',
        url: '/api/v1/environments/env-123',
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

  describe('DELETE /api/environments/:environmentId', () => {
    it('should delete environment', async () => {
      vi.spyOn(app.services.environment, 'deleteEnvironment').mockResolvedValueOnce()

      const response = await app.inject({
        method: 'DELETE',
        url: '/api/v1/environments/env-123',
      })

      expect(response.statusCode).toBe(200)
      const body = JSON.parse(response.body)
      expect(body.status).toBe('deleted')
    })
  })

  describe('POST /api/environments/:environmentId/search-files', () => {
    it('should search files in environment', async () => {
      const mockEnvironment = {
        id: 'env-123',
        name: 'Test Environment',
        repository: { type: 'local' as const, path: '/test/path' },
        codeExecution: { type: 'local' as const },
        variables: {},
        createdAt: new Date(),
        updatedAt: new Date(),
      }

      vi.spyOn(app.services.environment, 'getEnvironment').mockResolvedValueOnce(mockEnvironment)

      const response = await app.inject({
        method: 'POST',
        url: '/api/v1/environments/env-123/search-files',
        payload: {
          query: 'test',
          filePattern: '*.ts',
          maxResults: 50,
        },
      })

      expect(response.statusCode).toBe(200)
      const body = JSON.parse(response.body)
      expect(body.files).toBeDefined()
      expect(body.totalCount).toBeDefined()
      expect(body.truncated).toBeDefined()
    })

    it('should validate search query', async () => {
      const response = await app.inject({
        method: 'POST',
        url: '/api/v1/environments/env-123/search-files',
        payload: {
          query: '', // Empty query
        },
      })

      expect(response.statusCode).toBe(400)
    })
  })

  describe('POST /api/environments/:environmentId/run-setup', () => {
    it('should run setup script', async () => {
      const mockEnvironment = {
        id: 'env-123',
        name: 'Test Environment',
        repository: { type: 'local' as const, path: '/test/path' },
        codeExecution: { type: 'local' as const, setupCommands: 'echo "test"' },
        variables: {},
        createdAt: new Date(),
        updatedAt: new Date(),
      }

      vi.spyOn(app.services.environment, 'getEnvironment').mockResolvedValueOnce(mockEnvironment)

      const response = await app.inject({
        method: 'POST',
        url: '/api/v1/environments/env-123/run-setup',
      })

      expect(response.statusCode).toBe(200)
      const body = JSON.parse(response.body)
      expect(body.success).toBeDefined()
      expect(body.output).toBeDefined()
    })
  })

  describe('GET /api/environments/:environmentId/branches', () => {
    it('should get branches', async () => {
      const mockEnvironment = {
        id: 'env-123',
        name: 'Test Environment',
        repository: { type: 'github' as const, repoId: 'owner/repo', branch: 'main' },
        codeExecution: { type: 'docker' as const, image: 'node:18' },
        variables: {},
        createdAt: new Date(),
        updatedAt: new Date(),
      }

      vi.spyOn(app.services.environment, 'getEnvironment').mockResolvedValueOnce(mockEnvironment)

      const response = await app.inject({
        method: 'GET',
        url: '/api/v1/environments/env-123/branches',
      })

      expect(response.statusCode).toBe(200)
      const body = JSON.parse(response.body)
      expect(body.branches).toBeDefined()
      expect(Array.isArray(body.branches)).toBe(true)
    })
  })
})
