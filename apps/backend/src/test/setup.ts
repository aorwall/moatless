import { promises as fs } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { afterAll, beforeAll, beforeEach } from 'vitest'

let tempDataDir: string
let originalDataDir: string | undefined

beforeAll(async () => {
  // Store original DATA_DIR if it exists
  originalDataDir = process.env['DATA_DIR']

  // Create a unique temporary directory for this test run
  tempDataDir = await fs.mkdtemp(path.join(os.tmpdir(), 'moatless-test-data-'))

  // Set the DATA_DIR environment variable for all tests
  process.env['DATA_DIR'] = tempDataDir

  console.log(`Test DATA_DIR set to: ${tempDataDir}`)
})

beforeEach(async () => {
  // Ensure the data directory exists and is clean for each test
  // This helps with test isolation
  try {
    await fs.rm(tempDataDir, { recursive: true, force: true })
  } catch {
    // Directory might not exist, that's fine
  }
  await fs.mkdir(tempDataDir, { recursive: true })
})

afterAll(async () => {
  // Clean up the temporary directory after all tests complete
  if (tempDataDir) {
    try {
      await fs.rm(tempDataDir, { recursive: true, force: true })
      console.log(`Cleaned up test DATA_DIR: ${tempDataDir}`)
    } catch (error) {
      console.warn(`Failed to clean up test DATA_DIR: ${error}`)
    }
  }

  // Restore original DATA_DIR environment variable
  if (originalDataDir !== undefined) {
    process.env['DATA_DIR'] = originalDataDir
  } else {
    Reflect.deleteProperty(process.env, 'DATA_DIR')
  }
})
