import { randomUUID } from 'node:crypto'
import { promises as fs } from 'node:fs'
import http from 'node:http'
import os from 'node:os'
import path from 'node:path'
import { afterAll, beforeAll, describe, expect, it } from 'vitest'
import { ScreenshotService } from '../screenshot.service.js'

describe('ScreenshotService Integration Tests', () => {
  let screenshotService: ScreenshotService
  let testDataDir: string
  let testServer: http.Server
  let serverUrl: string
  let serverPort: number

  beforeAll(async () => {
    // Create temporary data directory
    testDataDir = path.join(os.tmpdir(), `screenshot-integration-test-${randomUUID()}`)
    await fs.mkdir(testDataDir, { recursive: true })

    // Override DATA_DIR for this test
    process.env.DATA_DIR = testDataDir

    // Create mock services with minimal implementation
    const mockStorage = {
      getTask: async () => null,
      saveTask: async () => {
        // No-op for integration test
      },
    } as any

    const mockEventService = {
      emitEvent: async () => {
        // No-op for integration test
      },
    } as any

    const mockRepositoryService = {
      getRepositoryById: async () => null,
    } as any

    const mockWorkspaceService = {
      capturePreviewScreenshot: async () => undefined,
    } as any

    // Create screenshot service
    screenshotService = new ScreenshotService(
      mockStorage,
      mockEventService,
      mockRepositoryService,
      mockWorkspaceService,
    )

    // Create a simple HTTP server serving a basic HTML page
    testServer = http.createServer((req, res) => {
      res.writeHead(200, { 'Content-Type': 'text/html' })
      res.end(`
        <!DOCTYPE html>
        <html>
          <head>
            <title>Test Page for Screenshots</title>
            <style>
              body {
                font-family: Arial, sans-serif;
                padding: 20px;
                background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
                color: white;
                min-height: 100vh;
                display: flex;
                align-items: center;
                justify-content: center;
              }
              .container {
                text-align: center;
                max-width: 600px;
              }
              h1 {
                font-size: 3em;
                margin-bottom: 0.5em;
              }
              p {
                font-size: 1.2em;
              }
            </style>
          </head>
          <body>
            <div class="container">
              <h1>Screenshot Test Page</h1>
              <p>This page is used to test screenshot capture functionality.</p>
              <p>Timestamp: ${new Date().toISOString()}</p>
            </div>
          </body>
        </html>
      `)
    })

    // Start server on random available port
    await new Promise<void>((resolve) => {
      testServer.listen(0, () => {
        const address = testServer.address()
        serverPort = typeof address === 'object' && address !== null ? address.port : 0
        serverUrl = `http://localhost:${serverPort}`
        resolve()
      })
    })
  })

  afterAll(async () => {
    // Cleanup browser
    await screenshotService.cleanup()

    // Stop test server
    await new Promise<void>((resolve, reject) => {
      testServer.close((err) => {
        if (err) reject(err)
        else resolve()
      })
    })

    // Cleanup test data directory
    try {
      await fs.rm(testDataDir, { recursive: true, force: true })
    } catch (_error) {
      // Ignore cleanup errors
    }

    // Reset environment variable
    process.env.DATA_DIR = undefined
  })

  describe('ensureBrowser', () => {
    it('should launch a Playwright browser', async () => {
      const browser = await screenshotService.ensureBrowser()

      expect(browser).toBeDefined()
      expect(browser.isConnected()).toBe(true)
    })

    it('should reuse the same browser instance on multiple calls', async () => {
      const browser1 = await screenshotService.ensureBrowser()
      const browser2 = await screenshotService.ensureBrowser()

      expect(browser1).toBe(browser2)
      expect(browser1.isConnected()).toBe(true)
    })

    it('should maintain browser connection', async () => {
      const browser = await screenshotService.ensureBrowser()

      // Create and close a page to test browser stability
      const page = await browser.newPage()
      await page.goto(serverUrl)
      await page.close()

      // Browser should still be connected
      expect(browser.isConnected()).toBe(true)
    })
  })

  describe('captureScreenshot', () => {
    const taskId = 'test-task-123'
    const iterationNumber = 1

    it('should capture a screenshot from a URL', async () => {
      const result = await screenshotService.captureScreenshot(serverUrl, taskId, iterationNumber)

      expect(result).toBeDefined()
      expect(result.screenshotPath).toBeDefined()
      expect(result.thumbnailPath).toBeDefined()
      expect(result.screenshotPath).toContain('iteration-1.png')
      expect(result.thumbnailPath).toContain('iteration-1-thumb.png')
    })

    it('should create screenshot file on disk', async () => {
      const result = await screenshotService.captureScreenshot(serverUrl, taskId, iterationNumber)

      // Verify screenshot file exists
      const screenshotExists = await fs
        .access(result.screenshotPath)
        .then(() => true)
        .catch(() => false)
      expect(screenshotExists).toBe(true)

      // Verify file is not empty
      const stats = await fs.stat(result.screenshotPath)
      expect(stats.size).toBeGreaterThan(0)
    })

    it('should create thumbnail file on disk', async () => {
      const result = await screenshotService.captureScreenshot(serverUrl, taskId, iterationNumber)

      // Verify thumbnail file exists
      const thumbExists = await fs
        .access(result.thumbnailPath)
        .then(() => true)
        .catch(() => false)
      expect(thumbExists).toBe(true)

      // Verify file is not empty
      const stats = await fs.stat(result.thumbnailPath)
      expect(stats.size).toBeGreaterThan(0)
    })

    it('should create thumbnail smaller than full screenshot', async () => {
      const result = await screenshotService.captureScreenshot(serverUrl, taskId, iterationNumber)

      const screenshotStats = await fs.stat(result.screenshotPath)
      const thumbnailStats = await fs.stat(result.thumbnailPath)

      // Thumbnail should be smaller than full screenshot
      expect(thumbnailStats.size).toBeLessThan(screenshotStats.size)
    })

    it('should save screenshots in correct directory structure', async () => {
      const result = await screenshotService.captureScreenshot(serverUrl, taskId, iterationNumber)

      // Verify directory structure: DATA_DIR/screenshots/taskId/
      expect(result.screenshotPath).toContain(path.join('screenshots', taskId))
      expect(result.thumbnailPath).toContain(path.join('screenshots', taskId))
    })

    it('should handle multiple screenshots for different iterations', async () => {
      const iteration1 = await screenshotService.captureScreenshot(serverUrl, taskId, 1)
      const iteration2 = await screenshotService.captureScreenshot(serverUrl, taskId, 2)

      expect(iteration1.screenshotPath).not.toBe(iteration2.screenshotPath)
      expect(iteration1.screenshotPath).toContain('iteration-1.png')
      expect(iteration2.screenshotPath).toContain('iteration-2.png')

      // Both files should exist
      const file1Exists = await fs
        .access(iteration1.screenshotPath)
        .then(() => true)
        .catch(() => false)
      const file2Exists = await fs
        .access(iteration2.screenshotPath)
        .then(() => true)
        .catch(() => false)

      expect(file1Exists).toBe(true)
      expect(file2Exists).toBe(true)
    })

    it('should handle different task IDs', async () => {
      const task1 = await screenshotService.captureScreenshot(serverUrl, 'task-1', 1)
      const task2 = await screenshotService.captureScreenshot(serverUrl, 'task-2', 1)

      expect(task1.screenshotPath).not.toBe(task2.screenshotPath)
      expect(task1.screenshotPath).toContain('task-1')
      expect(task2.screenshotPath).toContain('task-2')
    })

    it('should capture page content correctly', async () => {
      const result = await screenshotService.captureScreenshot(serverUrl, taskId, iterationNumber)

      // Read the screenshot file to verify it exists and has content
      const buffer = await fs.readFile(result.screenshotPath)

      // PNG files start with specific magic bytes
      expect(buffer[0]).toBe(0x89) // PNG signature
      expect(buffer[1]).toBe(0x50) // P
      expect(buffer[2]).toBe(0x4e) // N
      expect(buffer[3]).toBe(0x47) // G
    })

    it('should throw error for invalid URL', async () => {
      await expect(
        screenshotService.captureScreenshot('http://localhost:99999', taskId, iterationNumber),
      ).rejects.toThrow()
    })

    it('should handle server errors gracefully', async () => {
      // Create a server that returns 500 error
      const errorServer = http.createServer((req, res) => {
        res.writeHead(500, { 'Content-Type': 'text/plain' })
        res.end('Internal Server Error')
      })

      await new Promise<void>((resolve) => {
        errorServer.listen(0, () => resolve())
      })

      const errorAddress = errorServer.address()
      const errorPort =
        typeof errorAddress === 'object' && errorAddress !== null ? errorAddress.port : 0
      const errorUrl = `http://localhost:${errorPort}`

      try {
        // Should still capture screenshot even with 500 error (Playwright loads the page)
        const result = await screenshotService.captureScreenshot(errorUrl, taskId, iterationNumber)
        expect(result).toBeDefined()
      } finally {
        await new Promise<void>((resolve, reject) => {
          errorServer.close((err) => {
            if (err) reject(err)
            else resolve()
          })
        })
      }
    })
  })

  describe('cleanup', () => {
    it('should close browser when called', async () => {
      // Ensure browser is created
      const browser = await screenshotService.ensureBrowser()
      expect(browser.isConnected()).toBe(true)

      // Cleanup
      await screenshotService.cleanup()

      // Browser should be closed
      expect(browser.isConnected()).toBe(false)
    })

    it('should handle cleanup when no browser exists', async () => {
      // Create new service without browser
      const cleanService = new ScreenshotService({} as any, {} as any, {} as any, {} as any)

      // Should not throw
      await expect(cleanService.cleanup()).resolves.toBeUndefined()
    })
  })
})
