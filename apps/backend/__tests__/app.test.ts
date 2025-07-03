import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { createApp } from '../app.js'
import type { Hono } from 'hono'
import type { AppBindings } from '../app.js'

describe('App Setup', () => {
  let app: Hono<AppBindings>

  beforeEach(async () => {
    app = await createApp()
  })

  afterEach(async () => {
    // No cleanup needed
  })

  describe('Health check', () => {
    it('should respond to health check endpoint', async () => {
      const req = new Request('http://localhost/health', { method: 'GET' })
      const response = await app.fetch(req)

      expect(response.status).toBe(200)

      const body = await response.json()
      expect(body).toHaveProperty('status', 'ok')
      expect(body).toHaveProperty('timestamp')
      expect(body).toHaveProperty('version')
    })
  })

  describe('API endpoint', () => {
    it('should respond to API endpoint', async () => {
      const req = new Request('http://localhost/api/v1', { method: 'GET' })
      const response = await app.fetch(req)

      expect(response.status).toBe(200)

      const body = await response.json() as any
      expect(body).toHaveProperty('message', 'Moatless API')
      expect(body).toHaveProperty('version', '1.0.0')
      expect(body).toHaveProperty('endpoints')
      expect(body.endpoints).toHaveProperty('tasks', '/api/v1/tasks')
      expect(body.endpoints).toHaveProperty('repositories', '/api/v1/repositories')
    })
  })

  describe('Error handling', () => {
    it('should handle 404 errors', async () => {
      const req = new Request('http://localhost/api/v1/non-existent-api', { method: 'GET' })
      const response = await app.fetch(req)

      expect(response.status).toBe(404)
    })
  })

  describe('CORS', () => {
    it('should have CORS enabled', async () => {
      const req = new Request('http://localhost/health', {
        method: 'OPTIONS',
        headers: {
          Origin: 'http://localhost:3000',
          'Access-Control-Request-Method': 'GET',
        },
      })
      const response = await app.fetch(req)

      expect(response.status).toBe(204)
      expect(response.headers.get('access-control-allow-origin')).toBeDefined()
      expect(response.headers.get('access-control-allow-credentials')).toBe('true')
    })
  })
})
