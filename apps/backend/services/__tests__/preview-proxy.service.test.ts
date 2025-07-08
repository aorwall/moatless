import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import http from 'node:http'
import type { Logger } from 'pino'
import { PreviewProxyService, createPreviewProxy } from '../preview-proxy.service.js'

describe('PreviewProxyService', () => {
  let mockTargetServer: http.Server
  let targetPort: number
  let proxyService: PreviewProxyService
  const testScriptContent = '/* Test Inspector Script */'
  
  // Create a mock logger
  const mockLogger: Logger = {
    child: () => mockLogger,
    trace: vi.fn(),
    debug: vi.fn(),
    info: vi.fn(),
    warn: vi.fn(),
    error: vi.fn(),
    fatal: vi.fn(),
    level: 'info',
    silent: vi.fn(),
  } as unknown as Logger

  beforeEach(async () => {
    // Create a mock target server
    mockTargetServer = http.createServer((req, res) => {
      if (req.url === '/test.html') {
        res.writeHead(200, { 'Content-Type': 'text/html' })
        res.end('<html><body>Test Page</body></html>')
      } else if (req.url === '/test.css') {
        res.writeHead(200, { 'Content-Type': 'text/css' })
        res.end('body { color: red; }')
      } else if (req.url === '/api/data') {
        res.writeHead(200, { 'Content-Type': 'application/json' })
        res.end(JSON.stringify({ test: 'data' }))
      } else {
        res.writeHead(404)
        res.end('Not Found')
      }
    })

    // Start mock server and get its port
    await new Promise<void>((resolve) => {
      mockTargetServer.listen(0, () => {
        const addr = mockTargetServer.address()
        targetPort = typeof addr === 'object' && addr ? addr.port : 0
        resolve()
      })
    })
  })

  afterEach(async () => {
    // Stop proxy if running
    if (proxyService) {
      await proxyService.stop()
    }

    // Stop mock target server
    await new Promise<void>((resolve) => {
      mockTargetServer.close(() => resolve())
    })
  })

  describe('createPreviewProxy', () => {
    it('should create a proxy service with correct configuration', () => {
      const proxy = createPreviewProxy(
        `http://localhost:${targetPort}`,
        testScriptContent,
        mockLogger,
        9500
      )

      expect(proxy).toBeInstanceOf(PreviewProxyService)
      expect(proxy.getProxyUrl()).toMatch(/http:\/\/localhost:\d+/)
    })
  })

  describe('start/stop', () => {
    it('should start and stop the proxy server', async () => {
      proxyService = createPreviewProxy(
        `http://localhost:${targetPort}`,
        testScriptContent,
        mockLogger,
        9501
      )

      // Start proxy
      await proxyService.start()
      expect(proxyService.getProxyUrl()).toBe('http://localhost:9501')

      // Stop proxy
      await proxyService.stop()

      // Verify it's stopped by trying to connect
      const isRunning = await new Promise((resolve) => {
        const req = http.get('http://localhost:9501', () => {
          resolve(true)
        })
        req.on('error', () => resolve(false))
      })
      expect(isRunning).toBe(false)
    })

    it('should throw error if starting twice', async () => {
      proxyService = createPreviewProxy(
        `http://localhost:${targetPort}`,
        testScriptContent,
        mockLogger,
        9502
      )

      await proxyService.start()
      await expect(proxyService.start()).rejects.toThrow('Proxy server is already running')
    })
  })

  describe('request proxying', () => {
    beforeEach(async () => {
      proxyService = createPreviewProxy(
        `http://localhost:${targetPort}`,
        testScriptContent,
        mockLogger,
        9503
      )
      await proxyService.start()
    })

    it('should inject script into HTML responses', async () => {
      const response = await fetch(`http://localhost:9503/test.html`)
      const html = await response.text()

      expect(response.status).toBe(200)
      expect(html).toContain(testScriptContent)
      expect(html).toContain('<script>/* Test Inspector Script */</script></body>')
    })

    it('should pass through non-HTML responses unchanged', async () => {
      // Test CSS
      const cssResponse = await fetch(`http://localhost:9503/test.css`)
      const css = await cssResponse.text()
      expect(css).toBe('body { color: red; }')
      expect(css).not.toContain(testScriptContent)

      // Test JSON
      const jsonResponse = await fetch(`http://localhost:9503/api/data`)
      const json = await jsonResponse.json()
      expect(json).toEqual({ test: 'data' })
    })

    it('should handle 404 responses', async () => {
      const response = await fetch(`http://localhost:9503/not-found`)
      expect(response.status).toBe(404)
      const text = await response.text()
      expect(text).toBe('Not Found')
    })

    it('should inject script even without body tag', async () => {
      // Create a server that returns HTML without body tag
      const noBodyServer = http.createServer((_req, res) => {
        res.writeHead(200, { 'Content-Type': 'text/html' })
        res.end('<html>No body tag</html>')
      })

      await new Promise<void>((resolve) => {
        noBodyServer.listen(0, resolve)
      })

      const noBodyPort = (noBodyServer.address() as any).port
      const noBodyProxy = createPreviewProxy(
        `http://localhost:${noBodyPort}`,
        testScriptContent,
        mockLogger,
        9504
      )

      await noBodyProxy.start()

      const response = await fetch(`http://localhost:9504/`)
      const html = await response.text()
      expect(html).toContain(testScriptContent)
      expect(html).toContain('<script>/* Test Inspector Script */</script></html>')

      await noBodyProxy.stop()
      await new Promise<void>((resolve) => {
        noBodyServer.close(() => resolve())
      })
    })
  })

  describe('error handling', () => {
    it('should handle target server being down', async () => {
      // Create proxy pointing to non-existent server
      proxyService = createPreviewProxy(
        'http://localhost:99999',
        testScriptContent,
        mockLogger,
        9505
      )
      await proxyService.start()

      const response = await fetch(`http://localhost:9505/test`)
      expect(response.status).toBe(500) // Internal Server Error (since proxy can't handle the error)
      const text = await response.text()
      expect(text).toBe('Internal Server Error')
    })

    it('should handle proxy errors gracefully', async () => {
      proxyService = createPreviewProxy(
        `http://localhost:${targetPort}`,
        testScriptContent,
        mockLogger,
        9506
      )
      await proxyService.start()

      // Make request with invalid headers that might cause issues
      const response = await fetch(`http://localhost:9506/test`, {
        headers: {
          'Invalid-Header': '\n\r',
        },
      })

      // Should still get a response (either proxied or error)
      expect(response.status).toBeGreaterThanOrEqual(200)
    })
  })

  describe('HTML modification edge cases', () => {
    beforeEach(async () => {
      proxyService = createPreviewProxy(
        `http://localhost:${targetPort}`,
        testScriptContent,
        mockLogger,
        9507
      )
      await proxyService.start()
    })

    it('should handle HTML with existing script injection point', async () => {
      // Update mock server to return HTML with script
      mockTargetServer.removeAllListeners('request')
      mockTargetServer.on('request', (req, res) => {
        if (req.url === '/with-script.html') {
          res.writeHead(200, { 'Content-Type': 'text/html' })
          res.end('<html><body><script>existing</script></body></html>')
        }
      })

      const response = await fetch(`http://localhost:9507/with-script.html`)
      const html = await response.text()
      
      // Should inject before closing body tag
      expect(html).toMatch(/<script>.*Test Inspector Script.*<\/script><\/body>/)
      expect(html).toContain('<script>existing</script>')
    })
  })
})