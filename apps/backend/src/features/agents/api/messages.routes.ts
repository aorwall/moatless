import type { OpenAPIHono } from '@hono/zod-openapi'
import { getMessagesRoute } from '@moatless/api-contract/tasks'
import { TaskNotFoundError } from '@moatless/types'
import type { AppBindings } from '../../../app'
import { appContainer } from '../../../container/container'
import { MessageService } from '../services/message.service'

/**
 * Register message routes
 *
 * Messages are part of the agent feature since agents own the message lifecycle.
 * Routes are kept under /api/v1/tasks/:taskId/messages for backward compatibility.
 */
export function registerMessagesRoutes(app: OpenAPIHono<AppBindings>) {
  // Get task messages with merged tool calls
  app.openapi(getMessagesRoute, async (c) => {
    const messageService = appContainer.get(MessageService)

    const { taskId } = c.req.valid('param')

    try {
      const messages = await messageService.getTaskMessages(taskId)

      return c.json({
        messages,
        totalCount: messages.length,
      })
    } catch (error) {
      if (error instanceof Error) {
        if (error.message.includes('Task not found')) {
          throw new TaskNotFoundError(taskId)
        }
        // Re-throw other errors for proper handling
        throw error
      }
      throw error
    }
  })
}
