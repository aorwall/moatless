import type { Hono } from 'hono'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import type { AppBindings, AppWithServices } from '../app.js'
import { createApp } from '../app.js'

describe('App Setup', () => {
  let app: Hono<AppBindings>
  let appWithServices: AppWithServices

  beforeEach(async () => {
    appWithServices = await createApp()
    app = appWithServices.app
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

      const body = (await response.json()) as any
      expect(body).toHaveProperty('message', 'Moatless API')
      expect(body).toHaveProperty('version', '1.0.0')
      expect(body).toHaveProperty('endpoints')
      expect(body.endpoints).toHaveProperty('tasks', '/api/v1/tasks')
      expect(body.endpoints).toHaveProperty('repositories', '/api/v1/repositories')
    })
  })

  describe('Error handling', () => {
    it('should handle 404 errors', async () => {
      const req = new Request('http://localhost/api/v1/non-existent-api', {
        method: 'GET',
      })
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

  describe('Service Container', () => {
    it('should resolve all registered services', () => {
      const { services } = appWithServices

      // List of all expected services
      const expectedServices = [
        'agent',
        'auth',
        'claude',
        'command',
        'docker',
        'event',
        'figma',
        'github',
        'mcp',
        'message',
        'netlify',
        'previewRegistry',
        'repository',
        'screenshot',
        'screenshotOrchestration',
        'settings',
        'storage',
        'task',
        'tool',
        'upload',
        'workspace',
      ]

      for (const serviceName of expectedServices) {
        expect(services).toHaveProperty(serviceName)
        expect(services[serviceName as keyof typeof services]).toBeDefined()
        expect(services[serviceName as keyof typeof services]).not.toBeNull()
      }
    })

    it('should have correct number of services', () => {
      const { services } = appWithServices
      const serviceCount = Object.keys(services).length

      // Update this number if services are added/removed
      expect(serviceCount).toBe(21)
    })
  })
})
