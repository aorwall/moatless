import type { OpenAPIHono } from '@hono/zod-openapi'
import {
  deleteMessageRoute,
  getMessagesRoute,
  getPendingMessagesRoute,
} from '@moatless/api-contract/tasks'
import { TaskNotFoundError } from '@moatless/types'
import { HTTPException } from 'hono/http-exception'
import type { AppBindings } from '../../../app'
import { appContainer } from '../../../container/container'
import { TaskMessageRepository } from '../../tasks/db/task-message.repository'
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

  // Delete a queued message (not yet sent to agent)
  app.openapi(deleteMessageRoute, async (c) => {
    const messageRepository = appContainer.get(TaskMessageRepository)

    const { taskId, messageId } = c.req.valid('param')
    const deleted = await messageRepository.deleteQueuedMessage(taskId, messageId)

    if (!deleted) {
      throw new HTTPException(404, { message: 'Message not found or already sent to agent' })
    }

    return c.json({ status: 'deleted' as const })
  })

  // Get pending user messages (not yet sent to agent) + last result message
  app.openapi(getPendingMessagesRoute, async (c) => {
    const messageRepository = appContainer.get(TaskMessageRepository)

    const { taskId } = c.req.valid('param')

    const [pendingMessages, lastResult] = await Promise.all([
      messageRepository.getPendingUserMessages(taskId),
      messageRepository.getLastResultMessage(taskId),
    ])

    // Include last result in the response for resume logic
    const messages = lastResult ? [...pendingMessages, lastResult] : pendingMessages

    return c.json({
      messages,
      totalCount: messages.length,
    })
  })
}
