import { promises as fs } from 'node:fs'
import type { FullConfig } from '@playwright/test'

async function globalTeardown(config: FullConfig) {
  console.log('Starting E2E teardown...')

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

  // Clean up environment variables
  delete process.env.SKIP_CLAUDE_DISK_EVENTS
  delete process.env.E2E_TEMP_DIR

  console.log('E2E teardown complete')
}

export default globalTeardown