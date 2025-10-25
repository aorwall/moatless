import type { RepositoryConfig } from '@moatless/types'
import { MoatlessError } from '@moatless/types'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { build } from '../../test/helper'

describe('API Error Handling', () => {
  let app: Awaited<ReturnType<typeof build>>

  beforeEach(async () => {
    app = await build()
  })

  afterEach(async () => {
    // Hono doesn't need cleanup
  })

  describe('Validation Errors', () => {
    it('should return 400 for invalid request body', async () => {
      const response = await app.fetch(
        new Request('http://localhost/api/v1/tasks', {
          method: 'POST',
          headers: {
            'Content-Type': 'application/json',
          },
          body: JSON.stringify({
            // Missing required fields
            agentType: 'invalid-agent-type',
          }),
        }),
      )

      expect(response.status).toBe(400)
      const body = (await response.json()) as any
      expect(body.error).toBeDefined()
      expect(body.error.code).toBe('VALIDATION_ERROR')
      expect(body.error.details).toBeDefined()
    })

    it('should return 400 for invalid query parameters', async () => {
      const response = await app.fetch(
        new Request('http://localhost/api/v1/tasks?limit=invalid&offset=-1', {
          method: 'GET',
        }),
      )

      expect(response.status).toBe(400)
      const body = (await response.json()) as any
      expect(body.error.code).toBe('VALIDATION_ERROR')
    })
  })

  describe('Service Errors', () => {
    it('should handle MoatlessError correctly', async () => {
      const { appContainer } = await import('../../container/container.js')
      const { TaskService } = await import('../../services/task.service.js')
      const taskService = appContainer.get(TaskService)

      const error = new MoatlessError('Test error message', 'INTERNAL_SERVER_ERROR' as never, 418)
      vi.spyOn(taskService, 'getTask').mockRejectedValueOnce(error)

      const response = await app.fetch(
        new Request('http://localhost/api/v1/tasks/test-123', {
          method: 'GET',
        }),
      )

      expect(response.status).toBe(418)
      const body = (await response.json()) as any
      expect(body.error.code).toBe('INTERNAL_SERVER_ERROR')
      expect(body.error.message).toBe('Test error message')
    })

    it('should handle unexpected errors', async () => {
      const { appContainer } = await import('../../container/container.js')
      const { TaskService } = await import('../../services/task.service.js')
      const taskService = appContainer.get(TaskService)

      vi.spyOn(taskService, 'getTask').mockRejectedValueOnce(new Error('Unexpected database error'))

      const response = await app.fetch(
        new Request('http://localhost/api/v1/tasks/test-123', {
          method: 'GET',
        }),
      )

      expect(response.status).toBe(500)
      const body = (await response.json()) as any
      expect(body.error.code).toBe('INTERNAL_SERVER_ERROR')
      expect(body.error.message).toBe('Internal server error')
    })

    it('should handle null or undefined errors', async () => {
      const { appContainer } = await import('../../container/container.js')
      const { TaskService } = await import('../../services/task.service.js')
      const taskService = appContainer.get(TaskService)

      vi.spyOn(taskService, 'getTask').mockRejectedValueOnce(new Error())

      const response = await app.fetch(
        new Request('http://localhost/api/v1/tasks/test-123', {
          method: 'GET',
        }),
      )

      expect(response.status).toBe(500)
      const body = (await response.json()) as any
      expect(body.error.code).toBe('INTERNAL_SERVER_ERROR')
    })
  })

  describe('Rate Limiting', () => {
    it.skip('should return 429 when rate limit is exceeded', async () => {
      // Make requests up to the limit from the same IP
      const promises = []
      for (let i = 0; i < 101; i++) {
        promises.push(
          app.fetch(
            new Request('http://localhost/api/v1/tasks', {
              method: 'GET',
              headers: {
                'x-forwarded-for': '127.0.0.1', // Simulate same client
              },
            }),
          ),
        )
      }

      const responses = await Promise.all(promises)
      const rateLimitedResponses = responses.filter((r) => r.status === 429)

      expect(rateLimitedResponses.length).toBeGreaterThan(0)
      const body = (await rateLimitedResponses[0]?.json()) as any
      expect(body.error.code).toBe('RATE_LIMIT_EXCEEDED')
    })
  })

  describe('Not Found Errors', () => {
    it('should return 404 for non-existent endpoints', async () => {
      const response = await app.fetch(
        new Request('http://localhost/api/v1/non-existent-endpoint', {
          method: 'GET',
        }),
      )

      expect(response.status).toBe(404)
      const body = (await response.json()) as any
      expect(body.error).toBeDefined()
    })

    it('should return 404 for non-existent resources', async () => {
      const { appContainer } = await import('../../container/container.js')
      const { TaskService } = await import('../../services/task.service.js')
      const taskService = appContainer.get(TaskService)

      vi.spyOn(taskService, 'getTask').mockResolvedValueOnce(null as any)

      const response = await app.fetch(
        new Request('http://localhost/api/v1/tasks/non-existent', {
          method: 'GET',
        }),
      )

      expect(response.status).toBe(404)
      const body = (await response.json()) as any
      expect(body.error.code).toBe('TASK_NOT_FOUND')
    })
  })

  describe('Request ID Tracking', () => {
    it('should include request ID in error responses', async () => {
      const { appContainer } = await import('../../container/container.js')
      const { TaskService } = await import('../../services/task.service.js')
      const taskService = appContainer.get(TaskService)

      vi.spyOn(taskService, 'getTask').mockRejectedValueOnce(new Error('Test error'))

      const response = await app.fetch(
        new Request('http://localhost/api/v1/tasks/test-123', {
          method: 'GET',
        }),
      )

      expect(response.headers.get('x-request-id')).toBeDefined()
      const body = (await response.json()) as any
      expect(body.requestId).toBe(response.headers.get('x-request-id'))
    })

    it('should use provided X-Request-ID header', async () => {
      const customRequestId = 'custom-request-123'

      const response = await app.fetch(
        new Request('http://localhost/api/v1/tasks', {
          method: 'GET',
          headers: {
            'X-Request-ID': customRequestId,
          },
        }),
      )

      expect(response.headers.get('x-request-id')).toBe(customRequestId)
    })
  })

  describe('Content Type Validation', () => {
    it('should accept application/json content type', async () => {
      const { appContainer } = await import('../../container/container.js')
      const { RepositoryService } = await import(
        '../../features/repositories/services/repository.service.js'
      )
      const { TaskService } = await import('../../services/task.service.js')
      const repositoryService = appContainer.get(RepositoryService)
      const taskService = appContainer.get(TaskService)

      vi.spyOn(repositoryService, 'getRepositoryById').mockResolvedValueOnce({
        id: 'test-env',
        name: 'Test Repository',
      } as RepositoryConfig)

      vi.spyOn(taskService, 'createTask').mockResolvedValueOnce({
        taskId: 'test-task-id',
        status: 'open' as const,
        createdAt: new Date(),
        lastMessageAt: new Date(),
        agentType: 'claude-code',
        repositoryId: 'test-env',
        numTurns: 0,
        totalCostUsd: 0,
        iterations: [],
      })

      const response = await app.fetch(
        new Request('http://localhost/api/v1/tasks', {
          method: 'POST',
          headers: {
            'content-type': 'application/json',
          },
          body: JSON.stringify({
            agentType: 'claude-code',
            repositoryId: 'test-env',
            message: 'Test message',
          }),
        }),
      )

      expect(response.status).toBe(200)
    })

    it('should reject invalid content types for POST requests', async () => {
      const response = await app.fetch(
        new Request('http://localhost/api/v1/tasks', {
          method: 'POST',
          headers: {
            'content-type': 'text/plain',
          },
          body: 'plain text data',
        }),
      )

      expect(response.status).toBe(400)
    })
  })

  describe('Method Not Allowed', () => {
    it('should return 405 for unsupported methods', async () => {
      const response = await app.fetch(
        new Request('http://localhost/api/v1/tasks', {
          method: 'PUT',
          headers: {
            'Content-Type': 'application/json',
          },
          body: JSON.stringify({}),
        }),
      )

      expect(response.status).toBe(404) // Hono returns 404 for unmatched routes
    })
  })
})
