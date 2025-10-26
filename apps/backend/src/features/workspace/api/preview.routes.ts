import type { OpenAPIHono } from '@hono/zod-openapi'
import {
  getServerStatusRoute,
  restartServerRoute,
  startServerRoute,
  stopServerRoute,
} from '@moatless/api-contract/preview'
import type { AppBindings } from '../../../app'
import { appContainer } from '../../../container/container'
import { streamSSE } from '../../../utils/sse-stream'
import { PreviewManager } from '../services/preview-manager.service'

/**
 * Register preview server routes using new workspace feature architecture
 *
 * These routes use PreviewManager which:
 * - Calls WorkspaceCoordinator.ensureWorkspace() internally
 * - Delegates to LocalEnvironmentDriver for execution
 * - Updates runtime snapshots with preview state
 */
export function registerPreviewRoutes(app: OpenAPIHono<AppBindings>) {
  // POST /api/v1/tasks/:taskId/server/start
  app.openapi(startServerRoute, async (c) => {
    const previewManager = appContainer.get(PreviewManager)
    const { taskId } = c.req.valid('param')

    const result = await previewManager.start(taskId)
    return c.json(result)
  })

  // POST /api/v1/tasks/:taskId/server/stop
  app.openapi(stopServerRoute, async (c) => {
    const previewManager = appContainer.get(PreviewManager)
    const { taskId } = c.req.valid('param')

    await previewManager.stop(taskId)
    return c.json({
      stopped: true,
      runtime: 0, // TODO: Calculate actual runtime from snapshot timestamps
    })
  })

  // POST /api/v1/tasks/:taskId/server/restart
  app.openapi(restartServerRoute, async (c) => {
    const previewManager = appContainer.get(PreviewManager)
    const { taskId } = c.req.valid('param')

    const result = await previewManager.restart(taskId)
    return c.json(result)
  })

  // GET /api/v1/tasks/:taskId/server/status
  app.openapi(getServerStatusRoute, async (c) => {
    const previewManager = appContainer.get(PreviewManager)
    const { taskId } = c.req.valid('param')

    const status = await previewManager.getStatus(taskId)
    return c.json(status)
  })

  // GET /api/v1/tasks/:taskId/server/logs/stream
  // Note: This is a non-OpenAPI endpoint because SSE doesn't fit well in OpenAPI specs
  app.get('/api/v1/tasks/:taskId/server/logs/stream', (c) => {
    const previewManager = appContainer.get(PreviewManager)
    const taskId = c.req.param('taskId')

    if (!taskId) {
      return c.json({ error: 'Task ID is required' }, 400)
    }

    // Stream logs via PreviewManager
    return streamSSE(c, previewManager.streamLogs(taskId))
  })
}
