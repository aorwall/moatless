import http from 'node:http'
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it } from 'vitest'
import { createApp } from '../../app.js'
import { createTestServices } from '../../test/helpers.js'
import type { TestServices } from '../../test/helpers.js'

describe('Preview Environment Proxy - E2E Tests', () => {
  let app: ReturnType<typeof createApp>
  let testServices: TestServices
  let mockPreviewServer: http.Server
  let previewPort: number
  let serverPort: number
  let server: http.Server

  beforeAll(async () => {
    // Create test services with real implementations
    testServices = await createTestServices()
    
    // Create mock preview server
    mockPreviewServer = http.createServer((req, res) => {
      if (req.url === '/') {
        res.writeHead(200, { 'Content-Type': 'text/html' })
        res.end(`
          <!DOCTYPE html>
          <html>
            <head>
              <title>Test Preview App</title>
            </head>
            <body>
              <h1>Welcome to Preview</h1>
              <script>console.log('Original script');</script>
            </body>
          </html>
        `)
      } else if (req.url === '/api/data') {
        res.writeHead(200, { 'Content-Type': 'application/json' })
        res.end(JSON.stringify({ message: 'Hello from preview server' }))
      } else if (req.url === '/styles.css') {
        res.writeHead(200, { 'Content-Type': 'text/css' })
        res.end('body { background-color: #f0f0f0; }')
      } else if (req.url?.startsWith('/echo')) {
        // Echo endpoint for testing request forwarding
        const method = req.method
        const headers = req.headers
        const url = req.url
        
        let body = ''
        req.on('data', chunk => body += chunk)
        req.on('end', () => {
          res.writeHead(200, { 'Content-Type': 'application/json' })
          res.end(JSON.stringify({ method, headers, url, body }))
        })
      } else {
        res.writeHead(404, { 'Content-Type': 'text/plain' })
        res.end('Not Found')
      }
    })

    // Start mock preview server
    await new Promise<void>((resolve) => {
      mockPreviewServer.listen(0, () => {
        const addr = mockPreviewServer.address()
        previewPort = typeof addr === 'object' && addr ? addr.port : 0
        resolve()
      })
    })

    // Create and start the main server
    app = createApp()
    // Override services for testing
    app.use('*', async (c, next) => {
      c.set('services', testServices)
      await next()
    })

    server = http.createServer(app.fetch)
    await new Promise<void>((resolve) => {
      server.listen(0, () => {
        const addr = server.address()
        serverPort = typeof addr === 'object' && addr ? addr.port : 0
        resolve()
      })
    })
  })

  afterAll(async () => {
    // Cleanup servers
    await new Promise<void>((resolve) => {
      mockPreviewServer.close(() => resolve())
    })
    
    await new Promise<void>((resolve) => {
      server.close(() => resolve())
    })

    // Cleanup test services
    await testServices.cleanup()
  })

  beforeEach(async () => {
    // Setup test environment and task
    await testServices.setupTestRepository()
    const task = await testServices.createTestTask()
    
    // Mock the workspace to return our preview port
    const workspace = await testServices.workspace.getOrCreateWorkspace(task.id)
    
    // Mock the environment to return our test preview port
    if ('getPreviewPort' in workspace.environment && workspace.environment.getPreviewPort) {
      workspace.environment.getPreviewPort = () => previewPort
    }
    
    // Mock server status to indicate running
    if ('getServerStatus' in workspace.environment && workspace.environment.getServerStatus) {
      workspace.environment.getServerStatus = async () => ({
        running: true,
        configured: true,
        uptimeSeconds: 60,
        pid: 12345
      })
    }
  })

  afterEach(async () => {
    await testServices.cleanup()
  })

  describe('HTML script injection', () => {
    it('should inject script into HTML responses from real server', async () => {
      const taskId = await testServices.getTestTaskId()
      
      const response = await fetch(`http://localhost:${serverPort}/api/environments/${taskId}/proxy/`)
      
      expect(response.status).toBe(200)
      expect(response.headers.get('content-type')).toContain('text/html')
      
      const html = await response.text()
      
      // Should contain original content
      expect(html).toContain('<h1>Welcome to Preview</h1>')
      expect(html).toContain("console.log('Original script');")
      
      // Should contain injected script
      expect(html).toContain(`🚀 Moatless Preview Script Injected for task: ${taskId}`)
      expect(html).toContain('INSPECTOR_SCRIPT_LOADED')
      expect(html).toContain(`taskId: '${taskId}'`)
      expect(html).toContain("source: 'preview-iframe'")
      
      // Should have proper script placement (before </head>)
      expect(html).toMatch(/<script>.*🚀 Moatless Preview Script.*<\/script><\/head>/)
    })

    it('should not inject script into non-HTML responses', async () => {
      const taskId = await testServices.getTestTaskId()
      
      // Test JSON response
      const jsonResponse = await fetch(`http://localhost:${serverPort}/api/environments/${taskId}/proxy/api/data`)
      expect(jsonResponse.status).toBe(200)
      expect(jsonResponse.headers.get('content-type')).toContain('application/json')
      
      const jsonData = await jsonResponse.json()
      expect(jsonData).toEqual({ message: 'Hello from preview server' })
      
      // Test CSS response
      const cssResponse = await fetch(`http://localhost:${serverPort}/api/environments/${taskId}/proxy/styles.css`)
      expect(cssResponse.status).toBe(200)
      expect(cssResponse.headers.get('content-type')).toContain('text/css')
      
      const cssText = await cssResponse.text()
      expect(cssText).toBe('body { background-color: #f0f0f0; }')
      expect(cssText).not.toContain('🚀 Moatless Preview Script Injected')
    })
  })

  describe('request forwarding', () => {
    it('should forward GET requests with headers correctly', async () => {
      const taskId = await testServices.getTestTaskId()
      
      const response = await fetch(`http://localhost:${serverPort}/api/environments/${taskId}/proxy/echo`, {
        headers: {
          'User-Agent': 'Test E2E Browser',
          'Accept': 'application/json',
          'Custom-Header': 'test-value'
        }
      })
      
      expect(response.status).toBe(200)
      const data = await response.json()
      
      expect(data.method).toBe('GET')
      expect(data.url).toBe('/echo')
      expect(data.headers['user-agent']).toBe('Test E2E Browser')
      expect(data.headers['accept']).toBe('application/json')
      expect(data.headers['custom-header']).toBe('test-value')
      expect(data.headers['x-forwarded-for']).toBe('127.0.0.1')
      expect(data.headers['x-forwarded-host']).toBe('localhost')
    })

    it('should forward POST requests with body correctly', async () => {
      const taskId = await testServices.getTestTaskId()
      const postData = { message: 'Hello from E2E test' }
      
      const response = await fetch(`http://localhost:${serverPort}/api/environments/${taskId}/proxy/echo`, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json'
        },
        body: JSON.stringify(postData)
      })
      
      expect(response.status).toBe(200)
      const data = await response.json()
      
      expect(data.method).toBe('POST')
      expect(data.headers['content-type']).toBe('application/json')
      expect(JSON.parse(data.body)).toEqual(postData)
    })

    it('should handle query parameters correctly', async () => {
      const taskId = await testServices.getTestTaskId()
      
      const response = await fetch(
        `http://localhost:${serverPort}/api/environments/${taskId}/proxy/echo?param1=value1&param2=value2`
      )
      
      expect(response.status).toBe(200)
      const data = await response.json()
      
      expect(data.url).toBe('/echo?param1=value1&param2=value2')
    })
  })

  describe('error handling', () => {
    it('should handle 404 responses from preview server', async () => {
      const taskId = await testServices.getTestTaskId()
      
      const response = await fetch(`http://localhost:${serverPort}/api/environments/${taskId}/proxy/nonexistent`)
      
      expect(response.status).toBe(404)
      const text = await response.text()
      expect(text).toBe('Not Found')
    })

    it('should return 404 when task does not exist', async () => {
      const fakeTaskId = 'nonexistent-task-123'
      
      const response = await fetch(`http://localhost:${serverPort}/api/environments/${fakeTaskId}/proxy/`)
      
      expect(response.status).toBe(404)
    })

    it('should handle server being unavailable', async () => {
      const taskId = await testServices.getTestTaskId()
      
      // Stop the mock preview server to simulate unavailability
      await new Promise<void>((resolve) => {
        mockPreviewServer.close(() => resolve())
      })
      
      const response = await fetch(`http://localhost:${serverPort}/api/environments/${taskId}/proxy/`)
      
      expect(response.status).toBe(502)
      
      // Restart mock preview server for other tests
      mockPreviewServer = http.createServer((req, res) => {
        res.writeHead(200, { 'Content-Type': 'text/html' })
        res.end('<html><head></head><body>Restarted</body></html>')
      })
      
      await new Promise<void>((resolve) => {
        mockPreviewServer.listen(previewPort, () => resolve())
      })
    })
  })

  describe('content-length handling', () => {
    it('should update content-length header after script injection', async () => {
      const taskId = await testServices.getTestTaskId()
      
      const response = await fetch(`http://localhost:${serverPort}/api/environments/${taskId}/proxy/`)
      
      expect(response.status).toBe(200)
      
      const html = await response.text()
      const contentLength = response.headers.get('content-length')
      
      // Content-length should match the actual response body length
      if (contentLength) {
        expect(parseInt(contentLength)).toBe(html.length)
      }
      
      // Response should be larger than original due to script injection
      expect(html.length).toBeGreaterThan(200) // Original content is much smaller
    })
  })

  describe('concurrent requests', () => {
    it('should handle multiple simultaneous requests', async () => {
      const taskId = await testServices.getTestTaskId()
      
      // Make 5 concurrent requests
      const promises = Array.from({ length: 5 }, (_, i) =>
        fetch(`http://localhost:${serverPort}/api/environments/${taskId}/proxy/echo?request=${i}`)
      )
      
      const responses = await Promise.all(promises)
      
      // All requests should succeed
      responses.forEach((response, i) => {
        expect(response.status).toBe(200)
      })
      
      // Verify each response has correct data
      for (let i = 0; i < responses.length; i++) {
        const data = await responses[i].json()
        expect(data.url).toBe(`/echo?request=${i}`)
      }
    }, { timeout: 15000 })
  })
})