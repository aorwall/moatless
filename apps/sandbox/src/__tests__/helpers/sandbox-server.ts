import type { Server } from 'node:http'
import { serve } from '@hono/node-server'
import { createApp } from '../../app'
import type { SandboxConfig } from '../../config/environment'
import { initializeServerRegistry } from '../../features/servers'

export interface TestSandboxServer {
  baseUrl: string
  port: number
  stop: () => Promise<void>
}

/**
 * Start a sandbox server for testing
 */
export async function startTestSandboxServer(
  workspaceDir: string,
  options: {
    port?: number
    taskId?: string
  } = {},
): Promise<TestSandboxServer> {
  const port = options.port || 8765
  const taskId = options.taskId || `test-task-${Date.now()}`

  const config: SandboxConfig = {
    taskId,
    workspaceDir,
    dataDir: workspaceDir,
    uploadDir: workspaceDir,
    port,
    logLevel: 'warn', // Reduce noise during tests
    agentConfig: {}, // Empty agent config for tests
  }

  // Initialize server registry (mirrors index.ts startup)
  initializeServerRegistry(config)

  const app = createApp(config)

  // Start server
  const server = serve({
    fetch: app.fetch,
    port,
  }) as Server

  // Wait for server to be ready
  await waitForServerReady(`http://localhost:${port}`)

  return {
    baseUrl: `http://localhost:${port}`,
    port,
    stop: async () => {
      return new Promise((resolve, reject) => {
        server.close((err) => {
          if (err) reject(err)
          else resolve()
        })
      })
    },
  }
}

/**
 * Wait for server to respond to health check
 */
async function waitForServerReady(baseUrl: string, timeoutMs = 5000): Promise<void> {
  const startTime = Date.now()

  while (Date.now() - startTime < timeoutMs) {
    try {
      const response = await fetch(`${baseUrl}/health`)
      if (response.ok) {
        return
      }
    } catch {
      // Server not ready yet
    }
    await new Promise((resolve) => setTimeout(resolve, 100))
  }

  throw new Error(`Server failed to start within ${timeoutMs}ms`)
}

/**
 * HTTP client helpers for testing
 */
export function createTestClient(baseUrl: string) {
  return {
    get: async (path: string) => {
      const response = await fetch(`${baseUrl}${path}`)
      return {
        status: response.status,
        json: () => response.json(),
        text: () => response.text(),
      }
    },

    post: async (path: string, body?: unknown) => {
      const response = await fetch(`${baseUrl}${path}`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: body ? JSON.stringify(body) : undefined,
      })
      return {
        status: response.status,
        json: () => response.json(),
        text: () => response.text(),
      }
    },

    /**
     * Connect to SSE endpoint and collect events
     */
    sse: async (
      path: string,
      body: unknown,
      options: { maxEvents?: number; timeoutMs?: number } = {},
    ) => {
      const { maxEvents = 100, timeoutMs = 30000 } = options
      const events: Array<{ event: string; data: unknown }> = []

      const response = await fetch(`${baseUrl}${path}`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(body),
      })

      if (!response.ok) {
        throw new Error(`SSE request failed: ${response.status}`)
      }

      const reader = response.body?.getReader()
      if (!reader) {
        throw new Error('No response body')
      }

      const decoder = new TextDecoder()
      const startTime = Date.now()
      let buffer = ''

      while (events.length < maxEvents && Date.now() - startTime < timeoutMs) {
        const { done, value } = await reader.read()
        if (done) break

        buffer += decoder.decode(value, { stream: true })

        // Parse SSE events from buffer
        const lines = buffer.split('\n')
        buffer = lines.pop() || '' // Keep incomplete line in buffer

        let currentEvent = ''
        let currentData = ''

        for (const line of lines) {
          if (line.startsWith('event:')) {
            currentEvent = line.slice(6).trim()
          } else if (line.startsWith('data:')) {
            currentData = line.slice(5).trim()
          } else if (line === '' && currentEvent && currentData) {
            try {
              events.push({ event: currentEvent, data: JSON.parse(currentData) })
            } catch {
              events.push({ event: currentEvent, data: currentData })
            }
            currentEvent = ''
            currentData = ''

            // Check for done event
            if (events[events.length - 1]?.event === 'done') {
              reader.cancel()
              return events
            }
          }
        }
      }

      reader.cancel()
      return events
    },
  }
}
