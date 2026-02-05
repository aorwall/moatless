import type { OpenAPIHono } from '@hono/zod-openapi'
import {
  getWorkspaceStatusRoute,
  restartWorkspaceRoute,
  retryWorkspaceRoute,
} from '@moatless/api-contract/workspace'
import type { AppBindings } from '../../../app'
import { appContainer } from '../../../container/container'
import { handleServiceError } from '../../../utils/error-handler'
import { WorkspaceRuntimeManager } from '../services/workspace-runtime-manager.service'

/**
 * Register workspace management routes using new workspace feature architecture
 *
 * Uses WorkspaceRuntimeManager to get workspace status from runtime snapshots
 */
export function registerWorkspaceRoutes(app: OpenAPIHono<AppBindings>) {
  // GET /api/v1/tasks/:taskId/workspace-status
  app.openapi(getWorkspaceStatusRoute, async (c) => {
    const runtimeManager = appContainer.get(WorkspaceRuntimeManager)
    const { taskId } = c.req.valid('param')

    try {
      const workspaceStatus = await runtimeManager.getWorkspaceStatus(taskId)
      return c.json(workspaceStatus)
    } catch (error) {
      handleServiceError(error, 'Get workspace status')
    }
  })

  // POST /api/v1/tasks/:taskId/workspace-retry
  app.openapi(retryWorkspaceRoute, async (c) => {
    const runtimeManager = appContainer.get(WorkspaceRuntimeManager)
    const { taskId } = c.req.valid('param')

    try {
      const result = await runtimeManager.retryWorkspace(taskId)
      return c.json(result)
    } catch (error) {
      handleServiceError(error, 'Retry workspace')
    }
  })

  // POST /api/v1/tasks/:taskId/workspace-restart
  app.openapi(restartWorkspaceRoute, async (c) => {
    const runtimeManager = appContainer.get(WorkspaceRuntimeManager)
    const { taskId } = c.req.valid('param')

    try {
      const result = await runtimeManager.restartWorkspace(taskId)
      return c.json(result)
    } catch (error) {
      handleServiceError(error, 'Restart workspace')
    }
  })
}
