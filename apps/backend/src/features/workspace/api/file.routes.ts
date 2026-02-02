import type { OpenAPIHono } from '@hono/zod-openapi'
import { searchWorkspaceFilesRoute } from '@moatless/api-contract/workspace'
import type { AppBindings } from '../../../app'
import { appContainer } from '../../../container/container'
import { handleServiceError } from '../../../utils/error-handler'
import { WorkspaceRuntimeManager } from '../services/workspace-runtime-manager.service'

/**
 * Register file operation routes for workspace feature
 *
 * These routes use WorkspaceRuntimeManager to get workspace handles with drivers:
 * - WorkspaceRuntimeManager.ensureWorkspace() provisions workspace if needed
 * - WorkspaceHandle.driver provides file operations via remote HTTP calls
 * - Routes no longer need to manage workspace provisioning or path resolution
 */
export function registerFileRoutes(app: OpenAPIHono<AppBindings>) {
  // POST /api/v1/tasks/:taskId/search-files
  app.openapi(searchWorkspaceFilesRoute, async (c) => {
    const runtimeManager = appContainer.get(WorkspaceRuntimeManager)
    const { taskId } = c.req.valid('param')
    const { query, limit } = c.req.valid('json')

    try {
      const handle = await runtimeManager.ensureWorkspace(taskId)
      const result = await handle.driver.searchFiles({
        query,
        ...(limit !== undefined && { limit }),
      })
      return c.json(result)
    } catch (error) {
      handleServiceError(error, 'Search files')
    }
  })
}
