import { randomUUID } from 'node:crypto'
import { promises as fs } from 'node:fs'
import * as os from 'node:os'
import * as path from 'node:path'
import { afterAll, beforeAll, describe, expect, it } from 'vitest'
import { PreviewServer } from '../services/preview-server.js'

describe('PreviewServer Simple Test', () => {
  // Skip this test if running in IDE to prevent crashes
  const isInIDE = process.env.WEBSTORM_ENVIRONMENT || process.env.INTELLIJ_ENVIRONMENT
  const testFn = isInIDE ? it.skip : it
  let tempDir: string
  let server: PreviewServer
  let taskId: string

  beforeAll(async () => {
    // Setup temp directory for logs
    tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'preview-server-test-'))
    taskId = randomUUID()
    console.log(`Test environment created at: ${tempDir}`)
  }, 30000)

  afterAll(async () => {
    // Cleanup server if still running
    if (server) {
      try {
        console.log('Cleaning up server...')
        await server.stop()
      } catch (error) {
        console.warn('Error during server cleanup:', error)
      }
    }

    try {
      console.log(`Removing temp directory: ${tempDir}`)
      await fs.rm(tempDir, { recursive: true, force: true })
    } catch (error) {
      console.warn('Error removing temp directory:', error)
    }
  }, 30000)

  testFn(
    'should start frontend dev server and detect port',
    async () => {
      console.log('Creating PreviewServer instance...')

      // Get the frontend directory path relative to repo root
      // Go up two directories from __tests__ to get to repo root
      const repoRoot = path.resolve(__dirname, '../../..')
      const frontendPath = path.join(repoRoot, 'apps', 'frontend')

      console.log(`Frontend path: ${frontendPath}`)

      // Create PreviewServer instance directly
      // Use a unique task ID to avoid conflicts with existing metadata
      const uniqueTaskId = `test-${taskId}`
      server = new PreviewServer(
        uniqueTaskId,
        frontendPath, // Point to frontend directory
        'bun run dev', // The command to run (just 'dev', not 'dev:frontend')
        {}, // No additional env vars needed
        undefined, // No event service
        undefined, // No repository config
      )

      // Start the server
      console.log('Starting frontend dev server...')
      const startResult = await server.start()
      console.log('Start result:', startResult)

      // Verify we get a starting result with PID
      expect(startResult.starting).toBe(true)
      expect(startResult.log_file).toBeTruthy()

      console.log(`Server starting with PID: ${startResult.pid}`)
      console.log(`Log file: ${startResult.log_file}`)

      // Wait for server to start and detect port (up to 45 seconds)
      let serverStatus = server.getStatus()
      console.log('Server status while starting:', serverStatus)
      const startTime = Date.now()
      const timeout = 45000 // 45 seconds for Vite to start

      console.log('Waiting for port detection...')
      while (!serverStatus.running && !serverStatus.error && Date.now() - startTime < timeout) {
        await new Promise((resolve) => setTimeout(resolve, 1000)) // Check every second
        serverStatus = server.getStatus()

        // Log current status for debugging
        if (serverStatus.status === 'initializing') {
          console.log(
            `Server still initializing... (${Math.floor((Date.now() - startTime) / 1000)}s)`,
          )
        }

        if (serverStatus.running) {
          console.log(`✅ Server is now running on ${serverStatus.url}`)
          break
        }

        if (serverStatus.error) {
          console.log(`❌ Server failed: ${serverStatus.error}`)
          break
        }
      }

      const lalal = await server.getLogs(50)
      console.log('Server logs:', lalal)

      // Verify server is running successfully
      expect(serverStatus.running).toBe(true)
      expect(serverStatus.url).toBeDefined()
      expect(serverStatus.url).toMatch(/^http:\/\/localhost:\d+$/)
      expect(serverStatus.status).toBe('running')

      // Extract and verify port
      const portMatch = serverStatus.url!.match(/:(\d+)$/)
      expect(portMatch).toBeTruthy()
      const port = Number.parseInt(portMatch![1], 10)
      console.log(`✅ Port successfully detected: ${port}`)

      // Verify port is in reasonable range
      expect(port).toBeGreaterThan(1000)
      expect(port).toBeLessThan(65536)

      // Test HTTP connectivity
      console.log(`Testing HTTP connectivity to ${serverStatus.url}...`)
      const response = await fetch(serverStatus.url!, {
        signal: AbortSignal.timeout(5000), // 5 second timeout
      })
      expect(response.ok).toBe(true)
      console.log('✅ HTTP connectivity test passed')

      // Verify we can get logs
      const logs = await server.getLogs(10)
      expect(logs).toBeTruthy()
      expect(logs).toContain('localhost')
      console.log('✅ Server logs retrieved successfully')

      // Stop server
      console.log('Stopping server...')
      const stopped = await server.stop()

      // The stop method might return false due to timing issues with process groups,
      // but what matters is that the server is no longer running
      console.log(`Stop method returned: ${stopped}`)

      // Verify server is stopped (give it a moment)
      await new Promise((resolve) => setTimeout(resolve, 1000))
      const finalStatus = server.getStatus()
      expect(finalStatus.running).toBe(false)
      console.log('✅ Server stopped successfully')
    },
    60000,
  ) // 1 minute timeout for full test
})
