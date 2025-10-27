import type { OpenAPIHono } from '@hono/zod-openapi'
import { getWorkspaceStatusRoute } from '@moatless/api-contract/workspace'
import type { AppBindings } from '../../../app'
import { appContainer } from '../../../container/container'
import { WorkspaceService } from '../../../services/workspace.service'
import { handleServiceError } from '../../../utils/error-handler'

/**
 * Register workspace management routes using new workspace feature architecture
 *
 * These routes use the new architecture where possible:
 * - WorkspaceCoordinator for workspace provisioning
 * - WorkspaceService for status queries (still depends on legacy Workspace class)
 *
 * TODO: Migrate getWorkspaceStatus to use WorkspaceRuntimeManager once Workspace class
 *       file operations are migrated to FileGateway
 */
export function registerWorkspaceRoutes(app: OpenAPIHono<AppBindings>) {
  // GET /api/v1/tasks/:taskId/workspace-status
  app.openapi(getWorkspaceStatusRoute, async (c) => {
    const workspaceService = appContainer.get(WorkspaceService)
    const { taskId } = c.req.valid('param')

    try {
      const workspaceStatus = await workspaceService.getWorkspaceStatus(taskId)
      return c.json(workspaceStatus)
    } catch (error) {
      handleServiceError(error, 'Get workspace status')
    }
  })
}
