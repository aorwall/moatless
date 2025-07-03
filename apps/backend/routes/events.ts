import { randomUUID } from 'node:crypto'
import type { FastifyInstance } from 'fastify'
import { z } from 'zod'
import { sseManager } from '../utils/sse-manager.js'
import {logger} from "../utils/index.js";


// Schema for global stream query params (currently empty)
const GlobalStreamQuerySchema = z.object({})

export async function registerEventRoutes(app: FastifyInstance) {
  const { event: eventService } = app.services

  // Global event stream endpoint
  app.get(
    '/stream',
    {
      schema: {
        querystring: GlobalStreamQuerySchema,
      },
    },
    async (request, reply) => {
      // No query parameters needed for simple streaming

      // Set SSE headers with CORS
      const origin = request.headers.origin
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

      reply.raw.writeHead(200, {
        'Content-Type': 'text/event-stream',
        'Cache-Control': 'no-cache',
        Connection: 'keep-alive',
        'X-Accel-Buffering': 'no', // Disable Nginx buffering
        ...corsHeaders,
      })

      // Create global connection
      const connectionId = randomUUID()

      // Register connection with manager
      sseManager.connectGlobal(connectionId, request, reply)

      // Only stream new events, no replay of existing events

      // Flush the initial data
      reply.raw.flushHeaders()

      // Subscribe to all task events
      const unsubscribe = eventService.subscribeAll((event) => {
        // Send all events without any filtering
        try {
          reply.raw.write(`${JSON.stringify(event)}\n`)
          // Increment event count for this connection
          sseManager.incrementGlobalEventCount()
        } catch (error) {
          request.log.error('Failed to send global event over SSE', error)
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
          reply.raw.write(`${JSON.stringify(pingEvent)}\n`)
        } catch (_error) {
          clearInterval(pingInterval)
        }
      }, 30000)

      // Handle client disconnect
      request.raw.on('close', () => {
        unsubscribe()
        sseManager.disconnectGlobal(connectionId)
        clearInterval(pingInterval)
        reply.raw.end()
      })
    },
  )


  // Get global stream status with event counts
  app.get('/stream/status', async (_request, _reply) => {
    const stats = sseManager.getAllConnectionStats()

    return {
      globalConnections: stats.global.connectionCount,
      globalEventCount: stats.global.totalEvents,
      taskConnections: Object.fromEntries(
        Object.entries(stats.tasks).map(([taskId, details]) => [taskId, details.connectionCount])
      ),
      taskEventCounts: Object.fromEntries(
        Object.entries(stats.tasks).map(([taskId, details]) => [taskId, details.totalEvents])
      ),
      totalConnections: stats.total.connectionCount,
      totalEventCount: stats.total.totalEvents,
    }
  })
}
