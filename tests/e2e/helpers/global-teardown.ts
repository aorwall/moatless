import { promises as fs } from 'node:fs'
import type { FullConfig } from '@playwright/test'

// Store server process info globally so teardown can access it
declare global {
  var __E2E_SERVER_PID__: number | undefined
}

async function globalTeardown(config: FullConfig) {
  console.log('Starting E2E teardown...')

  // Kill the server process if it's still running
  if (globalThis.__E2E_SERVER_PID__) {
    try {
      process.kill(globalThis.__E2E_SERVER_PID__, 'SIGTERM')
      console.log('Terminated server process:', globalThis.__E2E_SERVER_PID__)

      // Wait a bit for graceful shutdown
      await new Promise((resolve) => setTimeout(resolve, 1000))

      // Force kill if still running
      try {
        process.kill(globalThis.__E2E_SERVER_PID__, 'SIGKILL')
        console.log('Force killed server process:', globalThis.__E2E_SERVER_PID__)
      } catch {
        // Process already terminated
      }
    } catch (error) {
      console.warn('Failed to terminate server process:', error)
    }
    globalThis.__E2E_SERVER_PID__ = undefined
  }

  // Clean up temp directory if it exists
  const tempDir = process.env.E2E_TEMP_DIR
  if (tempDir) {
    try {
      await fs.rm(tempDir, { recursive: true, force: true })
      console.log('Cleaned up temp directory:', tempDir)
    } catch (error) {
      console.warn('Failed to clean up temp directory:', error)
    }
  }

  // Clean up test variables
  delete process.env.SKIP_CLAUDE_DISK_EVENTS
  delete process.env.E2E_TEMP_DIR

  console.log('E2E teardown complete')
}

export default globalTeardown
