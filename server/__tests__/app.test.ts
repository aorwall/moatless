import { describe, expect, it, beforeEach, afterEach } from 'vitest'
import fastify, { FastifyInstance } from 'fastify'
import { setupApp } from '../app.js'

describe('App Setup', () => {
  let app: FastifyInstance

  beforeEach(async () => {
    app = fastify({ logger: false })
    await setupApp(app)
  })

  afterEach(async () => {
    await app.close()
  })

  describe('Health check', () => {
    it('should respond to health check endpoint', async () => {
      const response = await app.inject({
        method: 'GET',
        url: '/health',
      })

      expect(response.statusCode).toBe(200)
      
      const body = JSON.parse(response.body)
      expect(body).toHaveProperty('status', 'ok')
      expect(body).toHaveProperty('timestamp')
      expect(body).toHaveProperty('version')
    })
  })

  describe('API endpoint', () => {
    it('should respond to API endpoint', async () => {
      const response = await app.inject({
        method: 'GET',
        url: '/api',
      })

      expect(response.statusCode).toBe(200)
      
      const body = JSON.parse(response.body)
      expect(body).toHaveProperty('message', 'Moatless API')
      expect(body).toHaveProperty('version', '1.0.0')
      expect(body).toHaveProperty('endpoints')
      expect(body.endpoints).toHaveProperty('tasks', '/api/tasks')
      expect(body.endpoints).toHaveProperty('environments', '/api/environments')
    })
  })

  describe('Services', () => {
    it('should have all services available', () => {
      expect(app.services).toBeDefined()
      expect(app.services.storage).toBeDefined()
      expect(app.services.event).toBeDefined()
      expect(app.services.environment).toBeDefined()
      expect(app.services.workspace).toBeDefined()
      expect(app.services.task).toBeDefined()
    })
  })

  describe('Error handling', () => {
    it('should handle 404 errors', async () => {
      const response = await app.inject({
        method: 'GET',
        url: '/non-existent',
      })

      expect(response.statusCode).toBe(404)
    })
  })

  describe('CORS', () => {
    it('should have CORS enabled', async () => {
      const response = await app.inject({
        method: 'OPTIONS',
        url: '/health',
        headers: {
          'Origin': 'http://localhost:3000',
          'Access-Control-Request-Method': 'GET',
        },
      })

      expect(response.statusCode).toBe(204)
      expect(response.headers).toHaveProperty('access-control-allow-origin')
      expect(response.headers).toHaveProperty('access-control-allow-credentials', 'true')
    })
  })

  describe('WebSocket support', () => {
    it('should have WebSocket support enabled', () => {
      // Check if websocket plugin is registered
      expect(app.websocketServer).toBeDefined()
    })
  })
})