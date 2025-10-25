import { randomUUID } from 'node:crypto'
import type { OpenAPIHono } from '@hono/zod-openapi'
import { z } from 'zod'
import type { AppBindings } from '../../../app'
import { appContainer } from '../../../container/container'
import { EventService } from '../services/event.service'
import {
  buildCorsHeaders,
  buildStreamResponse,
  registerSseConnection,
  setupDisconnectHandler,
  startPingInterval,
  subscribeToEvents,
} from '../utils/sse-helpers'
import { sseManager } from '../utils/sse-manager'

// Schema for global stream query params (currently empty)
const GlobalStreamQuerySchema = z.object({})

// Note: SSE endpoints are kept as plain Hono routes because Server-Sent Events
// don't integrate well with OpenAPI specifications (streaming responses)
export function registerEventRoutes(app: OpenAPIHono<AppBindings>, prefix: string) {
  // Global event stream endpoint
  app.get(`${prefix}/stream`, (c) => {
    const eventService = appContainer.get(EventService)

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
