import type { Hono } from 'hono'
import { type AppBindings, createApp } from '../app.js'
import type { Services } from '../services/index.js'

/**
 * Enhanced app type for testing with HTTP request simulation
 */
export interface TestApp extends Hono<AppBindings> {
  services: Services
  inject: (options: RequestOptions) => Promise<TestResponse>
  close: () => Promise<void>
  listen: (options: { port: number; host?: string }) => Promise<void>
  server?: { address: () => { port: number } }
  _httpServer?: { close?: () => void }
}

/**
 * Request options for test HTTP requests
 */
export interface RequestOptions {
  method: string
  url: string
  payload?: unknown
  headers?: Record<string, string>
}

/**
 * Test response interface
 */
export interface TestResponse {
  statusCode: number
  body: string
  headers: Record<string, string>
}

/**
 * Creates a test app with enhanced functionality
 */
export async function build(): Promise<TestApp> {
  const { app, services } = await createApp()

  // Create enhanced app with test utilities
  const testApp = app as TestApp
  testApp.services = services

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
    response.headers.forEach((value: string, key: string) => {
      headers[key] = value
    })

    return {
      statusCode: response.status,
      body: await response.text(),
      headers,
    }
  }

  // Add listen method for tests that need a real server
  testApp.listen = async (options: { port: number; host?: string }): Promise<void> => {
    const { serve } = await import('@hono/node-server')

    // Start a real HTTP server for E2E tests
    const server = serve({
      fetch: app.fetch,
      port: options.port,
      hostname: options.host || 'localhost',
    })

    // Store server reference for cleanup and address access
    testApp.server = {
      address: () => ({ port: options.port }),
    }

    // Store the server instance for cleanup

    testApp._httpServer = server
  }

  // Add close method for test cleanup
  testApp.close = async (): Promise<void> => {
    // Close HTTP server if it exists
    if (testApp._httpServer) {
      testApp._httpServer.close?.()
      delete testApp._httpServer
      delete testApp.server
    }

    // Cleanup services if they have cleanup methods
    // Note: PreviewEnvironmentService doesn't have a cleanup method
    // Services should clean up automatically on process exit

    // Note: This is mainly for test compatibility
  }

  return testApp
}
