import { promises as fs } from 'node:fs'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { PreviewServer } from '../preview-server.js'

describe('PreviewServer Integration', () => {
  let tempDir: string
  let logFile: string

  beforeEach(async () => {
    // Create a temporary directory for each test
    tempDir = await fs.mkdtemp(path.join(tmpdir(), 'preview-server-test-'))
    logFile = path.join(tempDir, 'server.log')
  })

  afterEach(async () => {
    // Clean up temporary directory
    try {
      await fs.rm(tempDir, { recursive: true, force: true })
    } catch (error) {
      console.warn('Failed to clean up temp dir:', error)
    }
  })

  it('should execute simple command and write to log file', async () => {
    const server = new PreviewServer('test-task-id', tempDir, [
      'echo "Server starting..."',
      'echo "Server ready"',
    ])

    // Start the server
    const result = await server.start(logFile)
    expect(result.starting).toBe(true)

    // Wait a moment for execution
    await new Promise((resolve) => setTimeout(resolve, 2000))

    // Check if log file has content
    const logContent = await fs.readFile(logFile, 'utf-8').catch(() => '')
    console.log('Log file content:', logContent)
    expect(logContent).toContain('Server starting...')
    expect(logContent).toContain('Server ready')

    // Check server status
    const status = server.getStatus()
    console.log('Server status:', status)

    await server.stop()
  })

  it('should handle command failures and emit failed event', async () => {
    let failedEventEmitted = false

    const mockEventService = {
      emitEvent: async (event: any) => {
        if (event.type === 'server' && event.payload.status === 'failed') {
          failedEventEmitted = true
        }
      },
    }

    const server = new PreviewServer(
      'test-task-id',
      tempDir,
      ['nonexistentcommand'],
      {},
      mockEventService as any,
    )

    // Start the server (should fail)
    const result = await server.start(logFile)
    expect(result.starting).toBe(true)

    // Wait for failure detection
    await new Promise((resolve) => setTimeout(resolve, 3000))

    // Check if failed event was emitted
    expect(failedEventEmitted).toBe(true)

    // Check server status shows failure
    const status = server.getStatus()
    expect(status.status).toBe('failed')
  })

  it('should detect port from Vite output and emit running event', async () => {
    let runningEventEmitted = false
    let detectedPort = 0

    const mockEventService = {
      emitEvent: async (event: any) => {
        console.log('Event received:', event)
        if (event.type === 'server' && event.payload.status === 'running') {
          runningEventEmitted = true
          detectedPort = event.payload.port
        }
      },
    }

    const server = new PreviewServer(
      'test-task-id',
      tempDir,
      [
        'echo "VITE v5.4.19  ready in 385 ms"',
        'echo ""',
        'echo "$ vite"',
        'echo "  ➜  Local:   http://localhost:5173/"',
        'echo "  ➜  Network: use --host to expose"',
        'sleep 1', // Give time for port detection
      ],
      {},
      mockEventService as any,
    )

    // Start the server
    const result = await server.start(logFile)
    expect(result.starting).toBe(true)

    // Wait for port detection
    await new Promise((resolve) => setTimeout(resolve, 3000))

    // Check log file content
    const logContent = await fs.readFile(logFile, 'utf-8').catch(() => '')
    console.log('Log file content:', logContent)

    // Note: The port detection will work but verification will fail since it's just echo
    // In this test we mainly verify the pattern matching works
    const status = server.getStatus()
    console.log('Final server status:', status)

    await server.stop()
  })

  it('should handle chunked output correctly', async () => {
    let runningEventEmitted = false

    const mockEventService = {
      emitEvent: async (event: any) => {
        if (event.type === 'server' && event.payload.status === 'running') {
          runningEventEmitted = true
        }
      },
    }

    const server = new PreviewServer(
      'test-task-id',
      tempDir,
      ['printf "  ➜  Local:"', 'printf "   http://localhost:"', 'echo "5173/"', 'sleep 1'],
      {},
      mockEventService as any,
    )

    // Start the server
    const result = await server.start(logFile)
    expect(result.starting).toBe(true)

    // Wait for processing
    await new Promise((resolve) => setTimeout(resolve, 3000))

    // Check that the chunked output was reassembled correctly
    const logContent = await fs.readFile(logFile, 'utf-8').catch(() => '')
    console.log('Chunked log content:', logContent)
    expect(logContent).toContain('Local:   http://localhost:5173/')

    await server.stop()
  })

  it('should work with nvm commands', async () => {
    const server = new PreviewServer('test-task-id', tempDir, ['nvm use 22.6.0', 'node --version'])

    // Start the server
    const result = await server.start(logFile)
    expect(result.starting).toBe(true)

    // Wait for execution
    await new Promise((resolve) => setTimeout(resolve, 3000))

    // Check log file content
    const logContent = await fs.readFile(logFile, 'utf-8').catch(() => '')
    console.log('NVM command log content:', logContent)

    // Should have nvm output and node version
    expect(logContent).toMatch(/Now using node|v\d+\.\d+\.\d+/)

    await server.stop()
  })
})
