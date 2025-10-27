import type { Hono } from 'hono'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import type { AppBindings, AppWithServices } from '../app'
import { createApp } from '../app'
import { appContainer } from '../container/container'

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
    it('should resolve all registered services', async () => {
      // Import all service classes
      const { AuthService } = await import('../features/auth/services/auth.service.js')
      const { ClaudeService } = await import('../features/agents/index.js')
      const { CommandService } = await import('../services/command.service.js')
      const { DockerService } = await import('../services/docker.service.js')
      const { EventService } = await import('../features/events/index.js')
      const { FigmaService } = await import('../features/figma/services/figma.service.js')
      const { GitHubService } = await import('../services/github.service.js')
      const { McpService } = await import('../services/mcp.service.js')
      const { MessageService } = await import('../features/agents/services/message.service.js')
      const { NetlifyService } = await import('../services/netlify.service.js')
      const { RepositoryService } = await import(
        '../features/repositories/services/repository.service.js'
      )
      const { ScreenshotService } = await import('../services/screenshot.service.js')
      const { ScreenshotOrchestrationService } = await import(
        '../services/screenshot-orchestration.service.js'
      )
      const { SettingsService } = await import('../services/settings.service.js')
      const { TaskService } = await import('../features/tasks/services/task.service.js')
      const { ToolService } = await import('../services/tool.service.js')
      const { UploadService } = await import('../services/upload.service.js')
      const { WorkspaceService } = await import('../services/workspace.service.js')

      // List of all expected service classes
      const expectedServices = [
        AuthService,
        ClaudeService,
        CommandService,
        DockerService,
        EventService,
        FigmaService,
        GitHubService,
        McpService,
        MessageService,
        NetlifyService,
        RepositoryService,
        ScreenshotService,
        ScreenshotOrchestrationService,
        SettingsService,
        TaskService,
        ToolService,
        UploadService,
        WorkspaceService,
      ]

      for (const ServiceClass of expectedServices) {
        const service = appContainer.get(ServiceClass)
        expect(service).toBeDefined()
        expect(service).not.toBeNull()
      }
    })

    it('should have correct number of services', async () => {
      // Import all service classes
      const { AuthService } = await import('../features/auth/services/auth.service.js')
      const { ClaudeService } = await import('../features/agents/index.js')
      const { CommandService } = await import('../services/command.service.js')
      const { DockerService } = await import('../services/docker.service.js')
      const { EventService } = await import('../features/events/index.js')
      const { FigmaService } = await import('../features/figma/services/figma.service.js')
      const { GitHubService } = await import('../services/github.service.js')
      const { McpService } = await import('../services/mcp.service.js')
      const { MessageService } = await import('../features/agents/services/message.service.js')
      const { NetlifyService } = await import('../services/netlify.service.js')
      const { RepositoryService } = await import(
        '../features/repositories/services/repository.service.js'
      )
      const { ScreenshotService } = await import('../services/screenshot.service.js')
      const { ScreenshotOrchestrationService } = await import(
        '../services/screenshot-orchestration.service.js'
      )
      const { SettingsService } = await import('../services/settings.service.js')
      const { TaskService } = await import('../features/tasks/services/task.service.js')
      const { ToolService } = await import('../services/tool.service.js')
      const { UploadService } = await import('../services/upload.service.js')
      const { WorkspaceService } = await import('../services/workspace.service.js')

      // Count registered services
      const serviceCount = [
        AuthService,
        ClaudeService,
        CommandService,
        DockerService,
        EventService,
        FigmaService,
        GitHubService,
        McpService,
        MessageService,
        NetlifyService,
        RepositoryService,
        ScreenshotService,
        ScreenshotOrchestrationService,
        SettingsService,
        TaskService,
        ToolService,
        UploadService,
        WorkspaceService,
      ].length

      // Update this number if services are added/removed
      expect(serviceCount).toBe(19)
    })
  })
})
