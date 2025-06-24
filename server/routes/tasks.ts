import type { FastifyInstance } from 'fastify'
import { z } from 'zod'
import { TaskNotFoundError } from '../../shared/constants/errors.js'
import type { EventType } from '../../shared/types/event.js'
import { CreateTaskRequestSchema } from '../../shared/types/task.js'
import { SendMessageRequestSchema } from '../../shared/types/api.js'

// Schema for URL params
const TaskIdParamsSchema = z.object({
  taskId: z.string(),
})

// Schema for pagination query
const PaginationQuerySchema = z.object({
  limit: z.coerce.number().min(1).max(100).default(20),
  offset: z.coerce.number().min(0).default(0),
})

// Schema for task list query
const TaskListQuerySchema = PaginationQuerySchema.extend({
  status: z.enum(['active', 'completed', 'error', 'closed']).optional(),
})

// Schema for event list query
const EventListQuerySchema = PaginationQuerySchema.extend({
  type: z.string().optional(),
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
          200: z.object({
            taskId: z.string(),
            status: z.string(),
          }),
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

      return {
        tasks,
        pagination: {
          limit,
          offset,
          total: tasks.length, // Note: This is simplified, real implementation would need total count
        },
      }
    },
  )

  // Send message to task
  app.post(
    '/:taskId/messages',
    {
      schema: {
        params: TaskIdParamsSchema,
        body: SendMessageRequestSchema,
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

      return {
        events,
        pagination: {
          limit,
          offset,
          hasMore: events.length === limit,
        },
      }
    },
  )

  // Update task
  app.patch(
    '/:taskId',
    {
      schema: {
        params: TaskIdParamsSchema,
        body: z.object({
          status: z.enum(['active', 'completed', 'error', 'closed']).optional(),
        }),
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
      },
    },
    async (request, _reply) => {
      const { taskId } = request.params as any

      await taskService.deleteTask(taskId)
      return { status: 'deleted' }
    },
  )

  // WebSocket for real-time events
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
        socket.send(
          JSON.stringify({
            type: 'connection',
            status: 'connected',
            taskId,
            timestamp: new Date().toISOString(),
          }),
        )

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
