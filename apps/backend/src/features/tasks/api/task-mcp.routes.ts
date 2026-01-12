import type { OpenAPIHono } from '@hono/zod-openapi'
import { updateTaskMetadataRoute } from '@moatless/types/schemas'
import type { AppBindings } from '../../../app'
import { appContainer } from '../../../container/container'
import { validateTaskToken } from '../../../middleware/task-auth.middleware'
import { handleServiceError } from '../../../utils/error-handler'
import { logger } from '../../../utils/logger'
import { TaskService } from '../services/task.service'

/**
 * MCP-related task routes
 * These endpoints are called by the sandbox via task-scoped JWT authentication
 */
export function registerTaskMcpRoutes(app: OpenAPIHono<AppBindings>) {
  /**
   * Update task metadata (name and description)
   * POST /api/v1/tasks/:taskId/metadata
   *
   * Called by MCP tools in the sandbox to update task metadata
   * Requires X-Task-Token header with task-scoped JWT
   */
  app.openapi(updateTaskMetadataRoute, async (c) => {
    // Manually validate task token
    let tokenValid = false
    await validateTaskToken(c, () => {
      tokenValid = true
    })

    if (!tokenValid) {
      // Middleware will throw if invalid, this is just a safety check
      return c.json({ success: false, error: 'Unauthorized' }, 401)
    }

    const taskService = appContainer.get(TaskService)
    const taskId = c.req.param('taskId')
    const body = c.req.valid('json')

    logger.info(
      {
        taskId,
        updates: body,
      },
      'MCP: Updating task metadata',
    )

    try {
      // Build updates object
      const updates: { name?: string; description?: string } = {}
      if (body.name !== undefined) {
        updates.name = body.name
      }
      if (body.description !== undefined) {
        updates.description = body.description
      }

      // Validate at least one field is provided
      if (Object.keys(updates).length === 0) {
        logger.warn({ taskId }, 'MCP: No fields provided for update')
        return c.json({ success: false }, 400)
      }

      // Update task via TaskService (handles vibe repo renaming and event emission)
      await taskService.updateTaskWithEvent(taskId, updates)

      logger.info(
        {
          taskId,
          updatedFields: Object.keys(updates),
        },
        'MCP: Task metadata updated successfully',
      )

      return c.json({ success: true }, 200)
    } catch (error) {
      logger.error(
        {
          err: error as Error,
          taskId,
          updates: body,
        },
        'MCP: Failed to update task metadata',
      )
      return handleServiceError(error, 'Update task metadata')
    }
  })
}
