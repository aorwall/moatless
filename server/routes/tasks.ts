import type { FastifyInstance } from 'fastify'
import { z } from 'zod'
import { TaskNotFoundError } from '../../shared/constants/errors.js'
import {
  CreateTaskRequestSchema,
  DeleteTaskResponseSchema,
  EventListQuerySchema,
  type EventListResponse,
  SendMessageRequestSchema,
  SendMessageResponseSchema,
  SimpleTaskResponseSchema,
  TaskListQuerySchema,
  type TaskListResponse,
  UpdateTaskRequestSchema,
  UpdateTaskResponseSchema,
  type WebSocketConnectionEvent,
} from '../../shared/types/api.js'
import type { EventType } from '../../shared/types/event.js'
import { sseManager } from '../utils/sse-manager.js'

// Schema for URL params
const TaskIdParamsSchema = z.object({
  taskId: z.string(),
})

export async function registerTaskRoutes(app: FastifyInstance) {
  const { task: taskService, event: eventService } = app.services

  // Create a new task
  app.post(
    '/',
    {
      schema: {
        body: CreateTaskRequestSchema,
        response: {
          200: SimpleTaskResponseSchema,
        },
      },
    },
    async (request, _reply) => {
      const taskId = await taskService.createTask(request.body as any)
      return { taskId, status: 'active' }
    },
  )

  // Get task by ID
  app.get(
    '/:taskId',
    {
      schema: {
        params: TaskIdParamsSchema,
      },
    },
    async (request, _reply) => {
      const { taskId } = request.params as any
      const task = await taskService.getTask(taskId)

      if (!task) {
        throw new TaskNotFoundError(taskId)
      }

      return task
    },
  )

  // List all tasks with pagination
  app.get(
    '/',
    {
      schema: {
        querystring: TaskListQuerySchema,
      },
    },
    async (request, _reply) => {
      const { limit, offset, status } = request.query as any
      let tasks = await taskService.listTasks(limit, offset)

      // Filter by status if provided
      if (status) {
        tasks = tasks.filter((task) => task.status === status)
      }

      const response: TaskListResponse = {
        tasks: tasks,
        items: tasks,
        pagination: {
          limit,
          offset,
          total: tasks.length, // Note: This is simplified, real implementation would need total count
        },
      }
      return response
    },
  )

  // Send message to task
  app.post(
    '/:taskId/messages',
    {
      schema: {
        params: TaskIdParamsSchema,
        body: SendMessageRequestSchema,
        response: {
          200: SendMessageResponseSchema,
        },
      },
    },
    async (request, _reply) => {
      const { taskId } = request.params as any
      const { message } = request.body as any

      await taskService.sendMessage(taskId, message)
      return { status: 'sent', timestamp: new Date().toISOString() }
    },
  )

  // Get task events with pagination
  app.get(
    '/:taskId/events',
    {
      schema: {
        params: TaskIdParamsSchema,
        querystring: EventListQuerySchema,
      },
    },
    async (request, _reply) => {
      const { taskId } = request.params as any
      const { limit, offset, type } = request.query as any

      // Verify task exists
      const task = await taskService.getTask(taskId)
      if (!task) {
        throw new TaskNotFoundError(taskId)
      }

      const events = await eventService.getEvents(
        taskId,
        limit,
        offset,
        type as EventType | undefined,
      )

      const response: EventListResponse = {
        events,
        items: events,
        pagination: {
          limit,
          offset,
          hasMore: events.length === limit,
        },
      }
      return response
    },
  )

  // Update task
  app.patch(
    '/:taskId',
    {
      schema: {
        params: TaskIdParamsSchema,
        body: UpdateTaskRequestSchema,
        response: {
          200: UpdateTaskResponseSchema,
        },
      },
    },
    async (request, _reply) => {
      const { taskId } = request.params as any
      const updates = request.body as any

      await taskService.updateTask(taskId, updates)
      return { status: 'updated' }
    },
  )

  // Delete task
  app.delete(
    '/:taskId',
    {
      schema: {
        params: TaskIdParamsSchema,
        response: {
          200: DeleteTaskResponseSchema,
        },
      },
    },
    async (request, _reply) => {
      const { taskId } = request.params as any

      await taskService.deleteTask(taskId)
      return { status: 'deleted' }
    },
  )

  // Get workspace status
  app.get(
    '/:taskId/workspace-status',
    {
      schema: {
        params: TaskIdParamsSchema,
      },
    },
    async (request, _reply) => {
      const { taskId } = request.params as any

      // Verify task exists
      const task = await taskService.getTask(taskId)
      if (!task) {
        throw new TaskNotFoundError(taskId)
      }

      // Get workspace status from workspace service
      const workspaceStatus = await app.services.workspace.getWorkspaceStatus(taskId)
      return workspaceStatus
    },
  )

  // SSE endpoint for real-time events
  app.get(
    '/:taskId/stream',
    {
      schema: {
        params: TaskIdParamsSchema,
        querystring: z.object({
          lastEventId: z.string().optional(),
        }),
      },
    },
    async (request, reply) => {
      const { taskId } = request.params as any
      const { lastEventId } = request.query as any

      // Verify task exists
      const task = await taskService.getTask(taskId)
      if (!task) {
        throw new TaskNotFoundError(taskId)
      }

      // Set SSE headers
      reply.raw.writeHead(200, {
        'Content-Type': 'application/x-ndjson',
        'Cache-Control': 'no-cache',
        Connection: 'keep-alive',
        'X-Accel-Buffering': 'no', // Disable Nginx buffering
      })

      // Register connection with manager
      const connectionId = sseManager.connect(taskId, request, reply, lastEventId)

      let offset = 0

      // If lastEventId is provided, find the offset
      if (lastEventId) {
        const allEvents = await eventService.getEvents(taskId)
        for (let i = 0; i < allEvents.length; i++) {
          if (allEvents[i].eventId === lastEventId) {
            offset = i + 1
            break
          }
        }
      }

      // Send existing events from offset
      const existingEvents = await eventService.getEvents(taskId, 1000, offset)
      for (const event of existingEvents) {
        reply.raw.write(JSON.stringify(event) + '\n')
      }

      // Subscribe to new events
      const unsubscribe = eventService.subscribe(taskId, (event) => {
        try {
          reply.raw.write(JSON.stringify(event) + '\n')
        } catch (error) {
          request.log.error('Failed to send event over SSE', error)
        }
      })

      // Handle client disconnect
      request.raw.on('close', () => {
        unsubscribe()
        sseManager.disconnect(taskId, connectionId)
        reply.raw.end()
      })

      // Keep connection alive with periodic pings
      const pingInterval = setInterval(() => {
        try {
          reply.raw.write(':\n') // SSE comment for keep-alive
        } catch (error) {
          clearInterval(pingInterval)
        }
      }, 30000)

      request.raw.on('close', () => {
        clearInterval(pingInterval)
      })
    },
  )

  // Get SSE stream status
  app.get(
    '/:taskId/stream/status',
    {
      schema: {
        params: TaskIdParamsSchema,
      },
    },
    async (request, _reply) => {
      const { taskId } = request.params as any

      // Verify task exists
      const task = await taskService.getTask(taskId)
      if (!task) {
        throw new TaskNotFoundError(taskId)
      }

      return {
        activeConnections: sseManager.getConnectionCount(taskId),
        taskActive: task.status === 'active',
      }
    },
  )

  // WebSocket for real-time events (keep for backward compatibility)
  app.get(
    '/:taskId/events/stream',
    {
      websocket: true,
      schema: {
        params: TaskIdParamsSchema,
      },
    },
    async (socket, request) => {
      const { taskId } = request.params as any

      try {
        // Verify task exists before setting up subscription
        const task = await taskService.getTask(taskId)

        if (!task) {
          socket.close(1008, 'Task not found')
          return
        }

        const unsubscribe = eventService.subscribe(taskId, (event) => {
          try {
            if (socket.readyState === 1) {
              // OPEN
              socket.send(JSON.stringify(event))
            }
          } catch (error) {
            request.log.error('Failed to send event over websocket', error)
          }
        })

        // Send initial connection event
        const connectionEvent: WebSocketConnectionEvent = {
          type: 'connection',
          status: 'connected',
          taskId,
          timestamp: new Date().toISOString(),
        }
        socket.send(JSON.stringify(connectionEvent))

        socket.on('close', () => {
          unsubscribe()
        })

        socket.on('error', (error) => {
          request.log.error('WebSocket error', error)
          unsubscribe()
        })
      } catch (error) {
        request.log.error('Error verifying task for WebSocket', error)
        socket.close(1011, 'Internal server error')
      }
    },
  )
}
