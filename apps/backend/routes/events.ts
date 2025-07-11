import { randomUUID } from 'node:crypto'
import type { Hono } from 'hono'
import { describeRoute } from 'hono-openapi'
import { resolver } from 'hono-openapi/zod'
import { z } from 'zod'
import type { AppBindings } from '../app.js'
import { logger } from '../utils/index.js'
import { sseManager } from '../utils/sse-manager.js'

// Schema for global stream query params (currently empty)
const GlobalStreamQuerySchema = z.object({})

// Response schemas
const streamStatusResponseSchema = z.object({
  globalConnections: z.number(),
  globalEventCount: z.number(),
  taskConnections: z.record(z.number()),
  taskEventCounts: z.record(z.number()),
  totalConnections: z.number(),
  totalEventCount: z.number(),
})

export async function registerEventRoutes(app: Hono<AppBindings>, prefix: string) {
  // Global event stream endpoint
  app.get(
    `${prefix}/stream`,
    describeRoute({
      description: 'Subscribe to global event stream via Server-Sent Events',
      responses: {
        200: {
          description: 'Event stream connection established',
          content: {
            'text/event-stream': {
              schema: {
                type: 'string',
                description: 'Newline-delimited JSON events',
              },
            },
          },
        },
      },
    }),
    async (c) => {
      const services = c.get('services')
      const { event: eventService } = services

      // No query parameters needed for simple streaming
      const queryParams = c.req.query()
      GlobalStreamQuerySchema.parse(queryParams)

      // Set SSE headers with CORS
      const origin = c.req.header('origin')
      const corsHeaders: Record<string, string> = {}

      // Add CORS headers based on the same logic as the CORS plugin
      if (origin) {
        // In development, allow all localhost origins
        if (process.env.NODE_ENV !== 'production') {
          try {
            const url = new URL(origin)
            if (url.hostname === 'localhost' || url.hostname === '127.0.0.1') {
              corsHeaders['Access-Control-Allow-Origin'] = origin
              corsHeaders['Access-Control-Allow-Credentials'] = 'true'
            }
          } catch {
            // Invalid origin URL, skip CORS headers
          }
        } else {
          // In production, use configured origin
          const allowedOrigin = process.env.CORS_ORIGIN
          if (allowedOrigin === 'true' || allowedOrigin === origin) {
            corsHeaders['Access-Control-Allow-Origin'] = origin
            corsHeaders['Access-Control-Allow-Credentials'] = 'true'
          }
        }
      } else if (process.env.CORS_ORIGIN === 'true' || process.env.NODE_ENV !== 'production') {
        // No origin header but CORS allows all origins
        corsHeaders['Access-Control-Allow-Origin'] = '*'
      }

      // Create a streaming response
      const { readable, writable } = new TransformStream()
      const writer = writable.getWriter()

      // Create global connection
      const connectionId = randomUUID()

      // Register connection with manager (adapted for Hono)
      const controller = {
        enqueue: (data: string) => writer.write(new TextEncoder().encode(data)),
        close: () => writer.close(),
        error: (error: any) => writer.abort(error),
        desiredSize: 1,
      } as ReadableStreamDefaultController
      sseManager.connectGlobal(connectionId, controller)

      // Subscribe to all task events
      const unsubscribe = eventService.subscribeAll((event) => {
        // Send all events without any filtering
        try {
          writer.write(new TextEncoder().encode(`${JSON.stringify(event)}\n`))
          // Increment event count for this connection
          sseManager.incrementGlobalEventCount()
        } catch (error) {
          console.error('Failed to send global event over SSE', error)
        }
      })

      // Keep connection alive with periodic pings
      const pingInterval = setInterval(() => {
        try {
          // Send a ping event for NDJSON format
          const pingEvent = {
            type: 'ping',
            timestamp: new Date().toISOString(),
          }
          logger.debug('Sending ping event', { pingEvent })
          writer.write(new TextEncoder().encode(`${JSON.stringify(pingEvent)}\n`))
        } catch (_error) {
          clearInterval(pingInterval)
        }
      }, 30000)

      // Handle client disconnect (this is tricky in Hono, we'll rely on the stream closing)
      const abortController = new AbortController()
      c.req.raw.signal?.addEventListener('abort', () => {
        unsubscribe()
        sseManager.disconnectGlobal(connectionId)
        clearInterval(pingInterval)
        writer.close()
        abortController.abort()
      })

      return new Response(readable, {
        status: 200,
        headers: {
          'Content-Type': 'text/event-stream',
          'Cache-Control': 'no-cache',
          Connection: 'keep-alive',
          'X-Accel-Buffering': 'no', // Disable Nginx buffering
          ...corsHeaders,
        },
      })
    },
  )

  // Get global stream status with event counts
  app.get(
    `${prefix}/stream/status`,
    describeRoute({
      description: 'Get statistics about event stream connections and event counts',
      responses: {
        200: {
          description: 'Event stream statistics',
          content: {
            'application/json': {
              schema: resolver(streamStatusResponseSchema),
            },
          },
        },
      },
    }),
    async (c) => {
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
    },
  )
}
