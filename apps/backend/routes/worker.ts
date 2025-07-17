import type { Event } from '@moatless/types'
import type { Hono } from 'hono'
import { HTTPException } from 'hono/http-exception'
import { describeRoute } from 'hono-openapi'
import { resolver } from 'hono-openapi/zod'
import { z } from 'zod'
import type { AppBindings } from '../app.js'

// Schema for worker event emission
const WorkerEventSchema = z.object({
  taskId: z.string(),
  event: z.any(), // Event type from @moatless/types
})

const WorkerEventResponseSchema = z.object({
  success: z.boolean(),
  message: z.string().optional(),
})

export async function registerWorkerRoutes(app: Hono<AppBindings>, prefix: string) {
  // Emit event from worker to main API
  app.post(
    `${prefix}/events`,
    describeRoute({
      description: 'Emit an event from a worker to the main API event service',
      requestBody: {
        description: 'Event data to emit',
        content: {
          'application/json': {
            schema: resolver(WorkerEventSchema) as any,
          },
        },
      },
      responses: {
        200: {
          description: 'Event emitted successfully',
          content: {
            'application/json': {
              schema: resolver(WorkerEventResponseSchema) as any,
            },
          },
        },
        400: {
          description: 'Invalid request body',
        },
        401: {
          description: 'Unauthorized - invalid or missing API key',
        },
        404: {
          description: 'Task not found',
        },
        500: {
          description: 'Internal server error',
        },
      },
    }),
    async (c) => {
      const services = c.get('services')
      const { event: eventService, task: taskService } = services

      try {
        const body = await c.req.json()
        const validatedBody = WorkerEventSchema.parse(body)
        const { taskId, event } = validatedBody as { taskId: string; event: Event }

        // Verify task exists
        const task = await taskService.getTask(taskId)
        if (!task) {
          throw new HTTPException(404, { message: `Task ${taskId} not found` })
        }

        // Add taskId to the event if not present
        const eventWithTaskId = {
          ...event,
          taskId,
        }

        // Emit the event through the event service
        await eventService.emitEvent(eventWithTaskId)

        return c.json({
          success: true,
          message: 'Event emitted successfully',
        })
      } catch (error) {
        if (error instanceof HTTPException) {
          throw error
        }

        const errorMessage = error instanceof Error ? error.message : 'Unknown error'
        throw new HTTPException(500, { message: `Failed to emit event: ${errorMessage}` })
      }
    },
  )
}
