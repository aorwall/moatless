import type { OpenAPIHono } from '@hono/zod-openapi'
import { getWorkspaceStatusRoute, retryWorkspaceRoute } from '@moatless/api-contract/workspace'
import type { AppBindings } from '../../../app'
import { appContainer } from '../../../container/container'
import { handleServiceError } from '../../../utils/error-handler'
import { WorkspaceCoordinator } from '../services/workspace-coordinator.service'

/**
 * Register workspace management routes using new workspace feature architecture
 *
 * Uses WorkspaceCoordinator to get workspace status from runtime snapshots
 */
export function registerWorkspaceRoutes(app: OpenAPIHono<AppBindings>) {
  // GET /api/v1/tasks/:taskId/workspace-status
  app.openapi(getWorkspaceStatusRoute, async (c) => {
    const coordinator = appContainer.get(WorkspaceCoordinator)
    const { taskId } = c.req.valid('param')

    try {
      const workspaceStatus = await coordinator.getWorkspaceStatus(taskId)
      return c.json(workspaceStatus)
    } catch (error) {
      handleServiceError(error, 'Get workspace status')
    }
  })

  // POST /api/v1/tasks/:taskId/workspace-retry
  app.openapi(retryWorkspaceRoute, async (c) => {
    const coordinator = appContainer.get(WorkspaceCoordinator)
    const { taskId } = c.req.valid('param')

    try {
      const result = await coordinator.retryWorkspace(taskId)
      return c.json(result)
    } catch (error) {
      handleServiceError(error, 'Retry workspace')
    }
  })
}
