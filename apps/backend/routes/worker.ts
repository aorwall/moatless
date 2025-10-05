import type { OpenAPIHono } from '@hono/zod-openapi'
import { schemas } from '@moatless/types'

const { EventSchema } = schemas

import { HTTPException } from 'hono/http-exception'
import type { AppBindings } from '../app.js'
import { createLogger } from '../utils/index.js'

const logger = createLogger()

export async function registerWorkerRoutes(app: OpenAPIHono<AppBindings>, prefix: string) {
  // Emit event from worker to main API
  app.post(`${prefix}/emit-event`, async (c) => {
    const services = c.get('services')
    const { event: eventService, task: taskService } = services

    try {
      const body = await c.req.json()
      const event = EventSchema.parse(body)
      const { taskId } = event

      // Verify task exists
      const task = await taskService.getTask(taskId)
      if (!task) {
        logger.error(`Task ${taskId} not found`)
        throw new HTTPException(404, { message: `Task ${taskId} not found` })
      }

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
  })
}
