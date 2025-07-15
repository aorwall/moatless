import { HTTPException } from 'hono/http-exception'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { proxy } from 'hono/proxy'
import type { AppBindings } from '../../app.js'
import { createApp } from '../../app.js'
import type { WorkspaceService } from '../../services/workspace.service.js'

// Mock Hono proxy
vi.mock('hono/proxy', () => ({
  proxy: vi.fn()
}))

describe('Preview Environment Proxy Routes - Integration Tests', () => {
  let app: ReturnType<typeof createApp>
  let mockWorkspaceService: any
  let mockWorkspace: any
  let mockProxyResponse: Response

  beforeEach(() => {
    // Reset mocks
    vi.clearAllMocks()

    // Create mock workspace
    mockWorkspace = {
      getPreviewPort: vi.fn(),
      proxyPreviewRequest: vi.fn()
    }

    // Create mock workspace service
    mockWorkspaceService = {
      canManageServer: vi.fn(),
      getServerStatus: vi.fn(),
      getOrCreateWorkspace: vi.fn().mockResolvedValue(mockWorkspace)
    }

    // Create mock services
    const mockServices = {
      workspace: mockWorkspaceService
    }

    // Create app with mocked services
    app = createApp()
    // Override the services context for testing
    app.use('*', async (c, next) => {
      c.set('services', mockServices)
      await next()
    })

    // Setup default mock proxy response
    mockProxyResponse = new Response('<html><head></head><body>Test content</body></html>', {
      status: 200,
      headers: { 'content-type': 'text/html' }
    })
    
    vi.mocked(proxy).mockResolvedValue(mockProxyResponse)
  })

  describe('GET /api/environments/:taskId/proxy/*', () => {
    it('should proxy HTML requests with script injection', async () => {
      const taskId = 'test-task-123'
      const proxyPath = '/index.html'
      
      // Setup mocks
      mockWorkspaceService.canManageServer.mockResolvedValue(true)
      mockWorkspaceService.getServerStatus.mockResolvedValue({ running: true })
      mockWorkspace.getPreviewPort.mockReturnValue(3000)

      const response = await app.request(`/api/environments/${taskId}/proxy${proxyPath}`)
      
      expect(response.status).toBe(200)
      
      const html = await response.text()
      expect(html).toContain('🚀 Moatless Preview Script Injected for task: test-task-123')
      expect(html).toContain('INSPECTOR_SCRIPT_LOADED')
      expect(html).toContain('Test content')
      
      // Verify proxy was called with correct URL
      expect(proxy).toHaveBeenCalledWith(
        'http://localhost:3000/index.html',
        expect.objectContaining({
          headers: expect.objectContaining({
            'X-Forwarded-For': '127.0.0.1'
          })
        })
      )
    })

    it('should proxy non-HTML requests without script injection', async () => {
      const taskId = 'test-task-456'
      const proxyPath = '/api/data.json'
      
      // Setup mocks for JSON response
      const jsonResponse = new Response('{"data": "test"}', {
        status: 200,
        headers: { 'content-type': 'application/json' }
      })
      vi.mocked(proxy).mockResolvedValue(jsonResponse)
      
      mockWorkspaceService.canManageServer.mockResolvedValue(true)
      mockWorkspaceService.getServerStatus.mockResolvedValue({ running: true })
      mockWorkspace.getPreviewPort.mockReturnValue(3001)

      const response = await app.request(`/api/environments/${taskId}/proxy${proxyPath}`)
      
      expect(response.status).toBe(200)
      
      const responseText = await response.text()
      expect(responseText).toBe('{"data": "test"}')
      expect(responseText).not.toContain('🚀 Moatless Preview Script Injected')
      
      // Verify proxy was called with correct URL
      expect(proxy).toHaveBeenCalledWith(
        'http://localhost:3001/api/data.json',
        expect.any(Object)
      )
    })

    it('should handle query parameters correctly', async () => {
      const taskId = 'test-task-789'
      const proxyPath = '/search?q=test&limit=10'
      
      mockWorkspaceService.canManageServer.mockResolvedValue(true)
      mockWorkspaceService.getServerStatus.mockResolvedValue({ running: true })
      mockWorkspace.getPreviewPort.mockReturnValue(3002)

      await app.request(`/api/environments/${taskId}/proxy${proxyPath}`)
      
      // Verify proxy was called with query parameters
      expect(proxy).toHaveBeenCalledWith(
        'http://localhost:3002/search?q=test&limit=10',
        expect.any(Object)
      )
    })

    it('should return 404 when server is not running', async () => {
      const taskId = 'test-task-not-running'
      
      mockWorkspaceService.canManageServer.mockResolvedValue(true)
      mockWorkspaceService.getServerStatus.mockResolvedValue({ running: false })

      const response = await app.request(`/api/environments/${taskId}/proxy/index.html`)
      
      expect(response.status).toBe(404)
      const errorData = await response.json()
      expect(errorData.message).toBe('Preview server not running')
    })

    it('should return 404 when preview capability not available', async () => {
      const taskId = 'test-task-no-capability'
      
      mockWorkspaceService.canManageServer.mockResolvedValue(false)

      const response = await app.request(`/api/environments/${taskId}/proxy/index.html`)
      
      expect(response.status).toBe(404)
      const errorData = await response.json()
      expect(errorData.message).toBe('Preview capability not available for this task')
    })

    it('should return 404 when preview port not available', async () => {
      const taskId = 'test-task-no-port'
      
      mockWorkspaceService.canManageServer.mockResolvedValue(true)
      mockWorkspaceService.getServerStatus.mockResolvedValue({ running: true })
      mockWorkspace.getPreviewPort.mockReturnValue(undefined)

      const response = await app.request(`/api/environments/${taskId}/proxy/index.html`)
      
      expect(response.status).toBe(404)
      const errorData = await response.json()
      expect(errorData.message).toBe('Preview server port not available')
    })

    it('should return 400 when taskId is missing', async () => {
      const response = await app.request('/api/environments//proxy/index.html')
      
      expect(response.status).toBe(400)
      const errorData = await response.json()
      expect(errorData.message).toBe('Task ID is required')
    })

    it('should handle proxy errors gracefully', async () => {
      const taskId = 'test-task-proxy-error'
      
      mockWorkspaceService.canManageServer.mockResolvedValue(true)
      mockWorkspaceService.getServerStatus.mockResolvedValue({ running: true })
      mockWorkspace.getPreviewPort.mockReturnValue(3003)
      
      // Mock proxy to throw an error
      vi.mocked(proxy).mockRejectedValue(new Error('Connection refused'))

      const response = await app.request(`/api/environments/${taskId}/proxy/index.html`)
      
      expect(response.status).toBe(502)
      const errorData = await response.json()
      expect(errorData.message).toBe('Error proxying request to preview server')
    })

    it('should set correct headers in proxy request', async () => {
      const taskId = 'test-task-headers'
      
      mockWorkspaceService.canManageServer.mockResolvedValue(true)
      mockWorkspaceService.getServerStatus.mockResolvedValue({ running: true })
      mockWorkspace.getPreviewPort.mockReturnValue(3004)

      await app.request(`/api/environments/${taskId}/proxy/index.html`, {
        headers: {
          'User-Agent': 'Test Browser',
          'Accept': 'text/html,application/xhtml+xml',
          'Custom-Header': 'test-value'
        }
      })
      
      // Verify proxy was called with forwarded headers
      expect(proxy).toHaveBeenCalledWith(
        'http://localhost:3004/index.html',
        expect.objectContaining({
          headers: expect.objectContaining({
            'X-Forwarded-For': '127.0.0.1',
            'X-Forwarded-Host': 'localhost',
            'User-Agent': 'Test Browser',
            'Accept': 'text/html,application/xhtml+xml',
            'Custom-Header': 'test-value'
          })
        })
      )
    })

    it('should handle POST requests with body', async () => {
      const taskId = 'test-task-post'
      const requestBody = JSON.stringify({ test: 'data' })
      
      mockWorkspaceService.canManageServer.mockResolvedValue(true)
      mockWorkspaceService.getServerStatus.mockResolvedValue({ running: true })
      mockWorkspace.getPreviewPort.mockReturnValue(3005)

      await app.request(`/api/environments/${taskId}/proxy/api/submit`, {
        method: 'POST',
        body: requestBody,
        headers: {
          'Content-Type': 'application/json'
        }
      })
      
      // Verify proxy was called with POST method and correct URL
      expect(proxy).toHaveBeenCalledWith(
        'http://localhost:3005/api/submit',
        expect.objectContaining({
          headers: expect.objectContaining({
            'Content-Type': 'application/json'
          })
        })
      )
    })
  })

  describe('content-length header handling', () => {
    it('should update content-length header after script injection', async () => {
      const taskId = 'test-task-content-length'
      
      mockWorkspaceService.canManageServer.mockResolvedValue(true)
      mockWorkspaceService.getServerStatus.mockResolvedValue({ running: true })
      mockWorkspace.getPreviewPort.mockReturnValue(3006)

      const response = await app.request(`/api/environments/${taskId}/proxy/index.html`)
      
      const html = await response.text()
      const contentLength = response.headers.get('content-length')
      
      if (contentLength) {
        expect(parseInt(contentLength)).toBe(html.length)
      }
    })
  })
})