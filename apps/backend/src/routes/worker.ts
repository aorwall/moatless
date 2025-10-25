import type { OpenAPIHono } from '@hono/zod-openapi'
import { emitWorkerEventRoute } from '@moatless/api-contract/worker'
import { HTTPException } from 'hono/http-exception'
import type { AppBindings } from '../app.js'
import { appContainer } from '../container/container.js'
import { EventService } from '../services/event.service.js'
import { TaskService } from '../services/task.service.js'
import { createLogger } from '../utils/index.js'

const logger = createLogger()

export function registerWorkerRoutes(app: OpenAPIHono<AppBindings>) {
  /**
   * POST /api/v1/worker/emit-event
   * Emit event from worker to main API
   */
  app.openapi(emitWorkerEventRoute, async (c) => {
    const eventService = appContainer.get(EventService)
    const taskService = appContainer.get(TaskService)

    try {
      const event = c.req.valid('json')
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
      throw new HTTPException(500, {
        message: `Failed to emit event: ${errorMessage}`,
      })
    }
  })
}
