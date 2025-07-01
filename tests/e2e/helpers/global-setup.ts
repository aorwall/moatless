import { promises as fs } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import type { FullConfig } from '@playwright/test'
import { spawn, type ChildProcess } from 'node:child_process'

let serverProcess: ChildProcess | null = null
let tempDir: string

async function globalSetup(config: FullConfig) {
  // Create a temporary directory for test repositories
  tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'moatless-e2e-test-'))
  console.log('Created temp directory:', tempDir)

  // Set variables for test
  process.env.SKIP_CLAUDE_DISK_EVENTS = 'true'
  process.env.USE_CONSOLE_LOGGING = 'true'
  process.env.NODE_ENV = 'test'
  process.env.E2E_TEMP_DIR = tempDir

  // Build the application first
  console.log('Building application for E2E tests...')
  const buildProcess = spawn('pnpm', ['build'], {
    stdio: 'inherit',
    env: process.env,
  })

  await new Promise<void>((resolve, reject) => {
    buildProcess.on('close', (code) => {
      if (code === 0) {
        resolve()
      } else {
        reject(new Error(`Build failed with code ${code}`))
      }
    })
    buildProcess.on('error', reject)
  })

  // Start the server
  console.log('Starting server for E2E tests...')
  serverProcess = spawn('node', ['apps/backend/dist/index.js'], {
    env: {
      ...process.env,
      PORT: '3001',
      NODE_ENV: 'production', // Use production mode to enable static file serving
      SKIP_CLAUDE_DISK_EVENTS: 'true',
      USE_CONSOLE_LOGGING: 'true',
    },
    stdio: 'pipe',
  })

  // Wait for server to be ready
  await new Promise<void>((resolve, reject) => {
    let serverReady = false
    const timeout = setTimeout(() => {
      if (!serverReady) {
        reject(new Error('Server startup timeout'))
      }
    }, 30000) // 30 second timeout

    if (serverProcess?.stdout) {
      serverProcess.stdout.on('data', (data) => {
        const output = data.toString()
        console.log('Server:', output.trim())
        
        // Look for server ready indicators
        if (output.includes('Server listening on') || output.includes('listening on port')) {
          serverReady = true
          clearTimeout(timeout)
          resolve()
        }
      })
    }

    if (serverProcess?.stderr) {
      serverProcess.stderr.on('data', (data) => {
        console.error('Server error:', data.toString().trim())
      })
    }

    serverProcess?.on('error', (error) => {
      clearTimeout(timeout)
      reject(error)
    })

    serverProcess?.on('exit', (code) => {
      clearTimeout(timeout)
      if (!serverReady) {
        reject(new Error(`Server exited with code ${code} before becoming ready`))
      }
    })
  })

  console.log('E2E setup complete. Server running on port 3001')
  
  // Give the server a moment to fully initialize
  await new Promise(resolve => setTimeout(resolve, 2000))
}

export default globalSetup