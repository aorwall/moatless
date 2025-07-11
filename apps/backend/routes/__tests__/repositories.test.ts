import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { build } from '../../test/helper.js'

describe('Repository Routes', () => {
  let app: Awaited<ReturnType<typeof build>>

  beforeEach(async () => {
    app = await build()
  })

  afterEach(async () => {
    // Hono doesn't need cleanup
  })

  describe('POST /api/repositories', () => {
    it('should create a new repository', async () => {
      const mockRepository = {
        id: 'test-env-id',
        name: 'Test Repository',
        description: 'A test repository',
        path: '/test/path',
        useGitWorktrees: false,
        codeExecution: { type: 'local' as const },
        variables: {},
        createdAt: new Date(),
        updatedAt: new Date(),
      }

      vi.spyOn(app.services.repository, 'createRepository').mockResolvedValueOnce(mockRepository)

      const response = await app.fetch(
        new Request('http://localhost/api/v1/repositories', {
          method: 'POST',
          headers: {
            'Content-Type': 'application/json',
          },
          body: JSON.stringify({
            name: 'Test Repository',
            description: 'A test repository',
            path: '/test/path',
            useGitWorktrees: false,
            codeExecution: {
              type: 'local',
            },
          }),
        }),
      )

      expect(response.status).toBe(200)
      const body = (await response.json()) as any
      expect(body.name).toBe('Test Repository')
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
      const mockRepositories = [
        {
          id: 'repo-1',
          name: 'Repository 1',
          path: '/path1',
          useGitWorktrees: false,
          codeExecution: { type: 'local' as const },
          variables: {},
          createdAt: new Date(),
          updatedAt: new Date(),
        },
        {
          id: 'repo-2',
          name: 'Repository 2',
          path: '/path2',
          useGitWorktrees: false,
          codeExecution: { type: 'local' as const },
          variables: {},
          createdAt: new Date(),
          updatedAt: new Date(),
        },
      ]

      vi.spyOn(app.services.repository, 'listRepositories').mockResolvedValueOnce(mockRepositories)

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
      const mockRepository = {
        id: 'env-123',
        name: 'Test Repository',
        path: '/test/path',
        useGitWorktrees: false,
        codeExecution: { type: 'local' as const },
        variables: {},
        createdAt: new Date(),
        updatedAt: new Date(),
      }

      vi.spyOn(app.services.repository, 'getRepositoryById').mockResolvedValueOnce(mockRepository)

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
      vi.spyOn(app.services.repository, 'getRepositoryById').mockResolvedValueOnce(null)

      const response = await app.fetch(
        new Request('http://localhost/api/v1/repositories/non-existent', {
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
      const updatedEnvironment = {
        id: 'env-123',
        name: 'Updated Name',
        description: 'Updated description',
        path: '/test/path',
        useGitWorktrees: false,
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

      const response = await app.fetch(
        new Request('http://localhost/api/v1/repositories/env-123', {
          method: 'PUT',
          headers: {
            'Content-Type': 'application/json',
          },
          body: JSON.stringify({
            name: 'Updated Name',
            description: 'Updated description',
          }),
        }),
      )

      expect(response.status).toBe(200)
      const body = (await response.json()) as any
      expect(body.name).toBe('Updated Name')
      expect(body.description).toBe('Updated description')
    })
  })

  describe('DELETE /api/repositories/:repositoryId', () => {
    it('should delete repository', async () => {
      vi.spyOn(app.services.repository, 'deleteRepository').mockResolvedValueOnce()

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
      const mockRepository = {
        id: 'env-123',
        name: 'Test Repository',
        path: '/test/path',
        useGitWorktrees: false,
        codeExecution: { type: 'local' as const },
        variables: {},
        createdAt: new Date(),
        updatedAt: new Date(),
      }

      vi.spyOn(app.services.repository, 'getRepositoryById').mockResolvedValueOnce(mockRepository)
      vi.spyOn(app.services.repository, 'searchFiles').mockResolvedValueOnce({
        results: [
          { path: '/test/file1.ts', absolutePath: '/test/file1.ts', score: 1 },
          { path: '/test/file2.ts', absolutePath: '/test/file2.ts', score: 2 },
        ],
        totalFound: 2,
        query: 'test',
      })

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

  describe('POST /api/repositories/:repositoryId/run-setup', () => {
    it('should run setup script', async () => {
      const mockRepository = {
        id: 'env-123',
        name: 'Test Repository',
        path: '/test/path',
        useGitWorktrees: false,
        codeExecution: { type: 'local' as const, setupCommands: 'echo "test"' },
        variables: {},
        createdAt: new Date(),
        updatedAt: new Date(),
      }

      vi.spyOn(app.services.repository, 'getRepositoryById').mockResolvedValueOnce(mockRepository)
      vi.spyOn(app.services.workspace, 'createWorkspace').mockResolvedValueOnce({} as any)
      vi.spyOn(app.services.workspace, 'setupWorkspace').mockResolvedValueOnce(undefined)
      vi.spyOn(app.services.workspace, 'runCommand').mockResolvedValueOnce({
        command: 'echo "test"',
        output: 'test',
        exitCode: 0,
        durationMs: 100,
      })

      const response = await app.fetch(
        new Request('http://localhost/api/v1/repositories/env-123/run-setup', {
          method: 'POST',
        }),
      )

      expect(response.status).toBe(200)
      const body = (await response.json()) as any
      expect(body.success).toBeDefined()
      expect(body.output).toBeDefined()
    })
  })

  describe('GET /api/repositories/:repositoryId/branches', () => {
    it('should get branches', async () => {
      const mockRepository = {
        id: 'repo-123',
        name: 'Test Repository',
        path: '/test/repo',
        useGitWorktrees: false,
        codeExecution: { type: 'local' as const },
        variables: {},
        createdAt: new Date(),
        updatedAt: new Date(),
      }

      vi.spyOn(app.services.repository, 'getRepositoryById').mockResolvedValueOnce(mockRepository)

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
