import type { OpenAPIHono } from '@hono/zod-openapi'
import { type AppBindings, createApp } from '../app'
import type { EventService } from '../features/events/services/event.service'
import type { WorkspaceRuntimeManager } from '../features/workspace/services/workspace-runtime-manager.service'

/**
 * Enhanced app type for testing with HTTP request simulation
 */
export interface TestApp extends OpenAPIHono<AppBindings> {
  inject: (options: RequestOptions) => Promise<TestResponse>
  close: () => Promise<void>
  listen: (options: { port: number; host?: string }) => Promise<void>
  server?: { address: () => { port: number } } | undefined
  _httpServer?: { close?: () => void } | undefined
  services: {
    event: EventService
    workspaceCoordinator: WorkspaceRuntimeManager
  }
}

/**
 * Request options for test HTTP requests
 */
interface RequestOptions {
  method: string
  url: string
  payload?: unknown
  headers?: Record<string, string>
}

/**
 * Test response interface
 */
interface TestResponse {
  statusCode: number
  body: string
  headers: Record<string, string>
}

/**
 * Creates a test app with enhanced functionality
 */
export async function build(): Promise<TestApp> {
  const { app } = await createApp()

  // Import services and container
  const { appContainer } = await import('../container/container.js')
  const { EventService } = await import('../features/events/services/event.service.js')
  const { WorkspaceRuntimeManager } = await import(
    '../features/workspace/services/workspace-runtime-manager.service.js'
  )

  // Create enhanced app with test utilities
  const testApp = app as TestApp

  // Expose services for testing
  testApp.services = {
    event: appContainer.get(EventService),
    workspaceCoordinator: appContainer.get(WorkspaceRuntimeManager),
  }

  // Add inject method that maps to app.fetch()
  testApp.inject = async (options: RequestOptions): Promise<TestResponse> => {
    const url = new URL(options.url, 'http://localhost:3000')

    const requestInit: RequestInit = {
      method: options.method,
      headers: {
        'Content-Type': 'application/json',
        ...options.headers,
      },
    }

    if (
      options.payload &&
      (options.method === 'POST' || options.method === 'PUT' || options.method === 'PATCH')
    ) {
      requestInit.body = JSON.stringify(options.payload)
    }

    const request = new Request(url.toString(), requestInit)
    const response = await app.fetch(request)

    // Convert Headers to plain object
    const headers: Record<string, string> = {}
    for (const [key, value] of response.headers.entries()) {
      headers[key] = value
    }

    return {
      statusCode: response.status,
      body: await response.text(),
      headers,
    }
  }

  // Add listen method for tests that need a real server
  testApp.listen = async (options: { port: number; host?: string }): Promise<void> => {
    const { serve } = await import('@hono/node-server')

    // Start a real HTTP server for integration tests
    const server = serve({
      fetch: app.fetch,
      port: options.port,
      hostname: options.host || 'localhost',
    })

    // Wait a bit for server to start and get the actual assigned port
    await new Promise((resolve) => setTimeout(resolve, 100))

    // Get the actual address from the server
    const address = server.address()
    const actualPort = typeof address === 'object' && address !== null ? address.port : options.port

    // Store server reference for cleanup and address access
    testApp.server = {
      address: () => ({ port: actualPort }),
    }

    // Store the server instance for cleanup
    testApp._httpServer = server
  }

  // Add close method for test cleanup
  testApp.close = (): Promise<void> => {
    // Close HTTP server if it exists
    if (testApp._httpServer) {
      testApp._httpServer.close?.()
      testApp._httpServer = undefined
      testApp.server = undefined
    }

    // Cleanup services if they have cleanup methods
    // Services should clean up automatically on process exit

    // Note: This is mainly for test compatibility
    return Promise.resolve()
  }

  return testApp
}
