import { randomUUID } from 'node:crypto'
import type { OpenAPIHono } from '@hono/zod-openapi'
import { z } from 'zod'
import type { AppBindings } from '../app.js'
import { logger } from '../utils/index.js'
import { sseManager } from '../utils/sse-manager.js'

// Schema for global stream query params (currently empty)
const GlobalStreamQuerySchema = z.object({})

// Note: SSE endpoints are kept as plain Hono routes because Server-Sent Events
// don't integrate well with OpenAPI specifications (streaming responses)
export function registerEventRoutes(app: OpenAPIHono<AppBindings>, prefix: string) {
  // Global event stream endpoint
  app.get(`${prefix}/stream`, (c) => {
    const services = c.get('services')
    const { event: eventService } = services

    // No query parameters needed for simple streaming
    const queryParams = c.req.query()
    GlobalStreamQuerySchema.parse(queryParams)

    const origin = c.req.header('origin')
    const corsHeaders = buildCorsHeaders(origin)

    const { readable, writable } = new TransformStream()
    const writer = writable.getWriter()
    const connectionId = randomUUID()
    const abortController = new AbortController()

    registerSseConnection(connectionId, writer)

    const unsubscribe = subscribeToEvents(eventService, writer, abortController)
    const pingInterval = startPingInterval(writer, abortController)

    setupDisconnectHandler(
      c.req.raw.signal,
      unsubscribe,
      connectionId,
      pingInterval,
      writer,
      abortController,
    )

    return buildStreamResponse(readable, corsHeaders)
  })

  // Get global stream status with event counts
  app.get(`${prefix}/stream/status`, (c) => {
    const stats = sseManager.getAllConnectionStats()

    return c.json({
      globalConnections: stats.global.connectionCount,
      globalEventCount: stats.global.totalEvents,
      taskConnections: Object.fromEntries(
        Object.entries(stats.tasks).map(([taskId, details]) => [taskId, details.connectionCount]),
      ),
      taskEventCounts: Object.fromEntries(
        Object.entries(stats.tasks).map(([taskId, details]) => [taskId, details.totalEvents]),
      ),
      totalConnections: stats.total.connectionCount,
      totalEventCount: stats.total.totalEvents,
    })
  })
}

// Helper functions for SSE stream handler

function buildCorsHeaders(origin: string | undefined): Record<string, string> {
  const corsHeaders: Record<string, string> = {}

  if (origin) {
    return buildCorsHeadersWithOrigin(origin, corsHeaders)
  }

  return buildCorsHeadersWithoutOrigin(corsHeaders)
}

function buildCorsHeadersWithOrigin(
  origin: string,
  corsHeaders: Record<string, string>,
): Record<string, string> {
  if (process.env['NODE_ENV'] !== 'production') {
    return buildDevelopmentCorsHeaders(origin, corsHeaders)
  }

  return buildProductionCorsHeaders(origin, corsHeaders)
}

function buildDevelopmentCorsHeaders(
  origin: string,
  corsHeaders: Record<string, string>,
): Record<string, string> {
  try {
    const url = new URL(origin)
    if (url.hostname === 'localhost' || url.hostname === '127.0.0.1') {
      corsHeaders['Access-Control-Allow-Origin'] = origin
      corsHeaders['Access-Control-Allow-Credentials'] = 'true'
    }
  } catch {
    // Invalid origin URL, skip CORS headers
  }
  return corsHeaders
}

function buildProductionCorsHeaders(
  origin: string,
  corsHeaders: Record<string, string>,
): Record<string, string> {
  const allowedOrigin = process.env['CORS_ORIGIN']
  if (allowedOrigin === 'true' || allowedOrigin === origin) {
    corsHeaders['Access-Control-Allow-Origin'] = origin
    corsHeaders['Access-Control-Allow-Credentials'] = 'true'
  }
  return corsHeaders
}

function buildCorsHeadersWithoutOrigin(
  corsHeaders: Record<string, string>,
): Record<string, string> {
  if (process.env['CORS_ORIGIN'] === 'true' || process.env['NODE_ENV'] !== 'production') {
    corsHeaders['Access-Control-Allow-Origin'] = '*'
  }
  return corsHeaders
}

function registerSseConnection(
  connectionId: string,
  writer: WritableStreamDefaultWriter<Uint8Array>,
): void {
  const controller = {
    enqueue: (data: string) => writer.write(new TextEncoder().encode(data)),
    close: () => writer.close(),
    error: (error: Error) => writer.abort(error),
    desiredSize: 1,
  } as ReadableStreamDefaultController
  sseManager.connectGlobal(connectionId, controller)
}

function subscribeToEvents(
  eventService: { subscribeAll: (callback: (event: unknown) => void) => () => void },
  writer: WritableStreamDefaultWriter<Uint8Array>,
  abortController: AbortController,
): () => void {
  return eventService.subscribeAll((event) => {
    try {
      if (abortController.signal.aborted) {
        return
      }
      writer.write(new TextEncoder().encode(`${JSON.stringify(event)}\n`))
      sseManager.incrementGlobalEventCount()
    } catch (error) {
      if (error instanceof TypeError && error.message.includes('WritableStream')) {
        logger.debug('SSE connection closed, ignoring write error')
      } else {
        logger.error('Failed to send global event over SSE', error)
      }
    }
  })
}

function startPingInterval(
  writer: WritableStreamDefaultWriter<Uint8Array>,
  abortController: AbortController,
): NodeJS.Timeout {
  return setInterval(() => {
    try {
      if (abortController.signal.aborted) {
        return
      }
      const pingEvent = {
        type: 'ping',
        timestamp: new Date().toISOString(),
      }
      logger.debug('Sending ping event', { pingEvent })
      writer.write(new TextEncoder().encode(`${JSON.stringify(pingEvent)}\n`))
    } catch (error) {
      if (error instanceof TypeError && error.message.includes('WritableStream')) {
        logger.debug('SSE connection closed during ping, stopping pings')
      }
    }
  }, 30000)
}

function setupDisconnectHandler(
  signal: AbortSignal | undefined | null,
  unsubscribe: () => void,
  connectionId: string,
  pingInterval: NodeJS.Timeout,
  writer: WritableStreamDefaultWriter<Uint8Array>,
  abortController: AbortController,
): void {
  signal?.addEventListener('abort', () => {
    unsubscribe()
    sseManager.disconnectGlobal(connectionId)
    clearInterval(pingInterval)
    writer.close()
    abortController.abort()
  })
}

function buildStreamResponse(
  readable: ReadableStream,
  corsHeaders: Record<string, string>,
): Response {
  return new Response(readable, {
    status: 200,
    headers: {
      'Content-Type': 'text/event-stream',
      'Cache-Control': 'no-cache',
      Connection: 'keep-alive',
      'X-Accel-Buffering': 'no',
      ...corsHeaders,
    },
  })
}
