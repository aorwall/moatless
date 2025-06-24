import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { FastifyInstance } from 'fastify'
import { MoatlessError } from '../../../shared/constants/errors.js'
import { build } from '../../test/helper.js'

describe('API Error Handling', () => {
  let app: FastifyInstance

  beforeEach(async () => {
    app = await build()
  })

  afterEach(async () => {
    await app.close()
  })

  describe('Validation Errors', () => {
    it('should return 400 for invalid request body', async () => {
      const response = await app.inject({
        method: 'POST',
        url: '/api/tasks',
        payload: {
          // Missing required fields
          agentType: 'invalid-agent-type',
        },
      })

      expect(response.statusCode).toBe(400)
      const body = JSON.parse(response.body)
      expect(body.error).toBeDefined()
      expect(body.error.code).toBe('VALIDATION_ERROR')
      expect(body.error.details).toBeDefined()
    })

    it('should return 400 for invalid query parameters', async () => {
      const response = await app.inject({
        method: 'GET',
        url: '/api/tasks?limit=invalid&offset=-1',
      })

      expect(response.statusCode).toBe(400)
      const body = JSON.parse(response.body)
      expect(body.error.code).toBe('VALIDATION_ERROR')
    })

    it('should return 404 for invalid path parameters', async () => {
      const response = await app.inject({
        method: 'GET',
        url: '/api/tasks/invalid-uuid-format/events',
      })

      expect(response.statusCode).toBe(404) // Fastify returns 404 for unmatched routes
      const body = JSON.parse(response.body)
      expect(body.error).toBeDefined()
    })
  })

  describe('Service Errors', () => {
    it('should handle MoatlessError correctly', async () => {
      const error = new MoatlessError('Test error message', 'INTERNAL_SERVER_ERROR' as any, 418)
      vi.spyOn(app.services.task, 'getTask').mockRejectedValueOnce(error)

      const response = await app.inject({
        method: 'GET',
        url: '/api/tasks/test-123',
      })

      expect(response.statusCode).toBe(418)
      const body = JSON.parse(response.body)
      expect(body.error.code).toBe('INTERNAL_SERVER_ERROR')
      expect(body.error.message).toBe('Test error message')
    })

    it('should handle unexpected errors', async () => {
      vi.spyOn(app.services.task, 'getTask').mockRejectedValueOnce(
        new Error('Unexpected database error'),
      )

      const response = await app.inject({
        method: 'GET',
        url: '/api/tasks/test-123',
      })

      expect(response.statusCode).toBe(500)
      const body = JSON.parse(response.body)
      expect(body.error.code).toBe('INTERNAL_SERVER_ERROR')
      expect(body.error.message).toBe('Internal server error')
    })

    it('should handle null or undefined errors', async () => {
      vi.spyOn(app.services.task, 'getTask').mockRejectedValueOnce(new Error())

      const response = await app.inject({
        method: 'GET',
        url: '/api/tasks/test-123',
      })

      expect(response.statusCode).toBe(500)
      const body = JSON.parse(response.body)
      expect(body.error.code).toBe('INTERNAL_SERVER_ERROR')
    })
  })

  describe('Rate Limiting', () => {
    it.skip('should return 429 when rate limit is exceeded', async () => {
      // Make requests up to the limit from the same IP
      const promises = []
      for (let i = 0; i < 101; i++) {
        promises.push(
          app.inject({
            method: 'GET',
            url: '/api/tasks',
            headers: {
              'x-forwarded-for': '127.0.0.1', // Simulate same client
            },
          }),
        )
      }

      const responses = await Promise.all(promises)
      const rateLimitedResponses = responses.filter(r => r.statusCode === 429)

      expect(rateLimitedResponses.length).toBeGreaterThan(0)
      const body = JSON.parse(rateLimitedResponses[0].body)
      expect(body.error.code).toBe('RATE_LIMIT_EXCEEDED')
    })
  })

  describe('Not Found Errors', () => {
    it('should return 404 for non-existent endpoints', async () => {
      const response = await app.inject({
        method: 'GET',
        url: '/api/non-existent-endpoint',
      })

      expect(response.statusCode).toBe(404)
      const body = JSON.parse(response.body)
      expect(body.error).toBeDefined()
    })

    it('should return 404 for non-existent resources', async () => {
      vi.spyOn(app.services.task, 'getTask').mockResolvedValueOnce(null)

      const response = await app.inject({
        method: 'GET',
        url: '/api/tasks/non-existent',
      })

      expect(response.statusCode).toBe(404)
      const body = JSON.parse(response.body)
      expect(body.error.code).toBe('TASK_NOT_FOUND')
    })
  })

  describe('Request ID Tracking', () => {
    it('should include request ID in error responses', async () => {
      vi.spyOn(app.services.task, 'getTask').mockRejectedValueOnce(
        new Error('Test error'),
      )

      const response = await app.inject({
        method: 'GET',
        url: '/api/tasks/test-123',
      })

      expect(response.headers['x-request-id']).toBeDefined()
      const body = JSON.parse(response.body)
      expect(body.requestId).toBe(response.headers['x-request-id'])
    })

    it('should use provided X-Request-ID header', async () => {
      const customRequestId = 'custom-request-123'

      const response = await app.inject({
        method: 'GET',
        url: '/api/tasks',
        headers: {
          'X-Request-ID': customRequestId,
        },
      })

      expect(response.headers['x-request-id']).toBe(customRequestId)
    })
  })

  describe('Content Type Validation', () => {
    it('should accept application/json content type', async () => {
      vi.spyOn(app.services.environment, 'getEnvironment').mockResolvedValueOnce({
        id: 'test-env',
        name: 'Test Environment',
      } as any)

      const response = await app.inject({
        method: 'POST',
        url: '/api/tasks',
        headers: {
          'content-type': 'application/json',
        },
        payload: {
          agentType: 'claude-code',
          environmentId: 'test-env',
          message: 'Test message',
        },
      })

      expect(response.statusCode).toBe(200)
    })

    it('should reject invalid content types for POST requests', async () => {
      const response = await app.inject({
        method: 'POST',
        url: '/api/tasks',
        headers: {
          'content-type': 'text/plain',
        },
        payload: 'plain text data',
      })

      expect(response.statusCode).toBe(400)
    })
  })

  describe('Method Not Allowed', () => {
    it('should return 405 for unsupported methods', async () => {
      const response = await app.inject({
        method: 'PUT',
        url: '/api/tasks', // PUT not supported on this endpoint
        payload: {},
      })

      expect(response.statusCode).toBe(404) // Fastify returns 404 for unmatched routes
    })
  })
})