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
}
