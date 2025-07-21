import type { Event } from '@moatless/types'
import { schemas } from '@moatless/types'

const { EventSchema, WorkerEventResponseSchema } = schemas

import type { Hono } from 'hono'
import { HTTPException } from 'hono/http-exception'
import { describeRoute } from 'hono-openapi'
import { resolver } from 'hono-openapi/zod'
import type { AppBindings } from '../app.js'
import { createLogger } from '../utils/index.js'

const logger = createLogger()

export async function registerWorkerRoutes(app: Hono<AppBindings>, prefix: string) {
  // Emit event from worker to main API
  app.post(
    `${prefix}/emit-event`,
    describeRoute({
      description: 'Emit an event from a worker to the main API event service',
      requestBody: {
        description: 'Event to emit (contains taskId)',
        content: {
          'application/json': {
            schema: resolver(EventSchema) as any,
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
        const event = EventSchema.parse(body) as Event
        const { taskId } = event

        // Verify task exists
        const task = await taskService.getTask(taskId)
        if (!task) {
          logger.error(`Task ${taskId} not found`)
          throw new HTTPException(404, { message: `Task ${taskId} not found` })
        }

        logger.info(`Emitting event ${JSON.stringify(event)}`)

        // Emit the event through the event service
        await eventService.emitEvent(event)

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
