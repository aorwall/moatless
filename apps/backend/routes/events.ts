import { randomUUID } from 'node:crypto'
import type { FastifyInstance } from 'fastify'
import { z } from 'zod'
import type { EventListResponse } from '@moatless/shared/types/api'
import type { Event, EventType } from '@moatless/shared/types/event'
import { sseManager } from '../utils/sse-manager.js'

// Schema for global stream query params
const GlobalStreamQuerySchema = z.object({
  types: z.array(z.string()).optional(),
  activeTasksOnly: z.boolean().optional().default(true),
  lastEventId: z.string().optional(),
})

export async function registerEventRoutes(app: FastifyInstance) {
  const { event: eventService, task: taskService } = app.services

  // Global event stream endpoint
  app.get(
    '/stream',
    {
      schema: {
        querystring: GlobalStreamQuerySchema,
      },
    },
    async (request, reply) => {
      const { types, activeTasksOnly, lastEventId } = request.query as z.infer<
        typeof GlobalStreamQuerySchema
      >

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
        'Content-Type': 'application/x-ndjson',
        'Cache-Control': 'no-cache',
        Connection: 'keep-alive',
        'X-Accel-Buffering': 'no', // Disable Nginx buffering
        ...corsHeaders,
      })

      // Create global connection
      const connectionId = randomUUID()
      const filters = {
        types: types as EventType[] | undefined,
        activeTasksOnly,
      }

      // Register connection with manager
      sseManager.connectGlobal(connectionId, request, reply, filters)

      // Send existing events if reconnecting
      if (lastEventId) {
        // Get all active tasks
        const tasks = await taskService.listTasks(
          undefined,
          0,
          activeTasksOnly ? 'open' : undefined,
        )

        // Find the offset based on lastEventId across all tasks
        let foundLastEvent = false

        for (const task of tasks) {
          if (foundLastEvent) break

          const events = await eventService.getEvents(task.taskId, undefined, 0, undefined, task)
          for (let i = 0; i < events.length; i++) {
            if (events[i].eventId === lastEventId) {
              foundLastEvent = true

              // Send remaining events from this task
              for (let j = i + 1; j < events.length; j++) {
                if (!filters.types || filters.types.includes(events[j].type)) {
                  reply.raw.write(`${JSON.stringify(events[j])}\n`)
                }
              }
              break
            }
          }
        }

        // Send events from tasks after the one containing lastEventId
        if (foundLastEvent) {
          const taskIndex = tasks.findIndex((t) => {
            const events = eventService.getEvents(t.taskId, undefined, 0, undefined, t)
            return events.then((evts) => evts.some((e) => e.eventId === lastEventId))
          })

          if (taskIndex !== -1 && taskIndex < tasks.length - 1) {
            for (let i = taskIndex + 1; i < tasks.length; i++) {
              const events = await eventService.getEvents(
                tasks[i].taskId,
                undefined,
                0,
                undefined,
                tasks[i],
              )
              for (const event of events) {
                if (!filters.types || filters.types.includes(event.type)) {
                  reply.raw.write(`${JSON.stringify(event)}\n`)
                }
              }
            }
          }
        }
      }

      // Flush the initial data
      reply.raw.flushHeaders()

      // Subscribe to all task events
      const unsubscribe = eventService.subscribeAll((event) => {
        // Apply filters
        if (filters.types && !filters.types.includes(event.type)) {
          return
        }

        if (filters.activeTasksOnly) {
          // Check if task is active
          taskService
            .getTask(event.taskId)
            .then((task) => {
              if (task && (task.status === 'open' || task.status === 'running')) {
                try {
                  reply.raw.write(`${JSON.stringify(event)}\n`)
                } catch (error) {
                  request.log.error('Failed to send global event over SSE', error)
                }
              }
            })
            .catch(() => {
              // Task not found, skip
            })
        } else {
          try {
            reply.raw.write(`${JSON.stringify(event)}\n`)
          } catch (error) {
            request.log.error('Failed to send global event over SSE', error)
          }
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

  // Get all events (non-streaming)
  app.get(
    '/',
    {
      schema: {
        querystring: z.object({
          limit: z.number().optional().default(100),
          offset: z.number().optional().default(0),
          type: z.string().optional(),
          taskId: z.string().optional(),
        }),
      },
    },
    async (request, _reply) => {
      const { limit, offset, type, taskId } = request.query as {
        limit: number
        offset: number
        type?: string
        taskId?: string
      }

      let allEvents: Event[] = []

      if (taskId) {
        // Get events for specific task
        const task = await taskService.getTask(taskId)
        if (task) {
          allEvents = await eventService.getEvents(
            taskId,
            undefined,
            0,
            type as EventType | undefined,
            task,
          )
        }
      } else {
        // Get events from all tasks
        const tasks = await taskService.listTasks()

        for (const task of tasks) {
          const taskEvents = await eventService.getEvents(
            task.taskId,
            undefined,
            0,
            type as EventType | undefined,
            task,
          )
          allEvents.push(...taskEvents)
        }

        // Sort by timestamp (newest first)
        allEvents.sort((a, b) => {
          const dateA = a.timestamp ? new Date(a.timestamp).getTime() : 0
          const dateB = b.timestamp ? new Date(b.timestamp).getTime() : 0
          return dateB - dateA
        })
      }

      // Apply pagination
      const paginatedEvents = allEvents.slice(offset, offset + limit)

      const response: EventListResponse = {
        events: paginatedEvents,
        items: paginatedEvents,
      }

      return response
    },
  )

  // Get global stream status
  app.get('/stream/status', async (_request, _reply) => {
    const globalConnections = sseManager.getGlobalConnectionCount()
    const taskConnections = sseManager.getAllTaskConnectionCounts()

    return {
      globalConnections,
      taskConnections,
      totalConnections:
        globalConnections + Object.values(taskConnections).reduce((a, b) => a + b, 0),
    }
  })
}
