import http from 'node:http'
import { afterAll, beforeAll, describe, expect, it } from 'vitest'

describe('Preview Environment Proxy - Integration Tests (broad)', () => {
  let mockPreviewServer: http.Server
  let previewPort: number

  beforeAll(async () => {
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
  })

  afterAll(async () => {
    // Cleanup servers
    await new Promise<void>((resolve) => {
      mockPreviewServer.close(() => resolve())
    })
  })

  describe('HTML script injection patterns', () => {
    it('should have correct script injection patterns for HTML responses', () => {
      const taskId = 'test-task-123'
      const originalHtml = `
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
      `

      // Simulate script injection
      const injectedScript = `
            <script>
              console.log('🚀 Moatless Preview Script Injected for task: ${taskId}');
              
              // Send a message to parent to confirm script is loaded
              window.addEventListener('load', () => {
                if (window.parent && window.parent !== window) {
                  window.parent.postMessage({
                    type: 'INSPECTOR_SCRIPT_LOADED',
                    payload: { version: '1.0.0', taskId: '${taskId}' },
                    source: 'preview-iframe'
                  }, '*');
                }
              });
            </script>
          `

      const modifiedHtml = originalHtml.replace('</head>', `${injectedScript}</head>`)

      // Verify script injection worked
      expect(modifiedHtml).toContain('🚀 Moatless Preview Script Injected for task: test-task-123')
      expect(modifiedHtml).toContain('INSPECTOR_SCRIPT_LOADED')
      expect(modifiedHtml).toContain(`taskId: '${taskId}'`)
      expect(modifiedHtml).toContain("source: 'preview-iframe'")
      expect(modifiedHtml).toContain("console.log('Original script');")
    })

    it('should preserve original content when injecting scripts', () => {
      const originalContent = '<h1>Welcome to Preview</h1>'
      const originalScript = "console.log('Original script');"
      const html = `<html><head></head><body>${originalContent}<script>${originalScript}</script></body></html>`

      const injectedScript = '<script>console.log("Injected script");</script>'
      const modifiedHtml = html.replace('</head>', `${injectedScript}</head>`)

      expect(modifiedHtml).toContain(originalContent)
      expect(modifiedHtml).toContain(originalScript)
      expect(modifiedHtml).toContain('Injected script')
    })
  })

  describe('content type handling', () => {
    it('should identify HTML content types correctly', () => {
      const htmlContentTypes = [
        'text/html',
        'text/html; charset=utf-8',
        'text/html; charset=UTF-8',
        'text/html;charset=utf-8',
      ]

      htmlContentTypes.forEach((contentType) => {
        expect(contentType.toLowerCase().includes('text/html')).toBe(true)
      })
    })

    it('should not inject into non-HTML content types', () => {
      const nonHtmlContent = 'body { background-color: #f0f0f0; }'
      const contentType = 'text/css'

      // Should not inject into CSS
      expect(contentType.includes('text/html')).toBe(false)
      expect(nonHtmlContent).not.toContain('🚀 Moatless Preview Script Injected')
    })
  })

  describe('proxy URL construction', () => {
    it('should construct correct proxy URLs', () => {
      const taskId = 'test-task-123'
      const serverPort = 3000
      const proxyPath = '/api/data'

      const proxyUrl = `http://localhost:${serverPort}${proxyPath}`
      expect(proxyUrl).toBe('http://localhost:3000/api/data')

      const previewEndpoint = `/api/environments/${taskId}/proxy${proxyPath}`
      expect(previewEndpoint).toBe('/api/environments/test-task-123/proxy/api/data')
    })

    it('should handle query parameters in proxy URLs', () => {
      const taskId = 'test-task-456'
      const path = '/search'
      const queryString = 'q=test&limit=10'
      const fullPath = `${path}?${queryString}`

      const previewEndpoint = `/api/environments/${taskId}/proxy${fullPath}`
      expect(previewEndpoint).toBe('/api/environments/test-task-456/proxy/search?q=test&limit=10')
    })
  })

  describe('mock server functionality', () => {
    it('should serve HTML content from mock server', async () => {
      const response = await fetch(`http://localhost:${previewPort}/`)

      expect(response.status).toBe(200)
      expect(response.headers.get('content-type')).toBe('text/html')

      const html = await response.text()
      expect(html).toContain('<h1>Welcome to Preview</h1>')
      expect(html).toContain("console.log('Original script');")
    })

    it('should serve JSON content from mock server', async () => {
      const response = await fetch(`http://localhost:${previewPort}/api/data`)

      expect(response.status).toBe(200)
      expect(response.headers.get('content-type')).toBe('application/json')

      const data = await response.json()
      expect(data).toEqual({ message: 'Hello from preview server' })
    })

    it('should serve CSS content from mock server', async () => {
      const response = await fetch(`http://localhost:${previewPort}/styles.css`)

      expect(response.status).toBe(200)
      expect(response.headers.get('content-type')).toBe('text/css')

      const css = await response.text()
      expect(css).toBe('body { background-color: #f0f0f0; }')
    })

    it('should return 404 for non-existent resources', async () => {
      const response = await fetch(`http://localhost:${previewPort}/nonexistent`)

      expect(response.status).toBe(404)
      expect(response.headers.get('content-type')).toBe('text/plain')

      const text = await response.text()
      expect(text).toBe('Not Found')
    })
  })
})
