import type { OpenAPIHono } from '@hono/zod-openapi'
import {
  getServerStatusRoute,
  restartServerRoute,
  SERVER_LOGS_PATH,
  startServerRoute,
  stopServerRoute,
} from '@moatless/api-contract/preview'
import type { AppBindings } from '../../../app.js'
import { appContainer } from '../../../container/container.js'
import { streamSSE } from '../../../utils/sse-stream.js'
import { PreviewOrchestratorService } from '../services/preview-orchestrator.service.js'

/**
 * Register preview server routes
 *
 * These routes use PreviewOrchestratorService which:
 * - Coordinates with workspace feature for provisioning
 * - Delegates to PreviewRuntimeService for process management
 * - Updates workspace snapshots with preview state
 */
export function registerPreviewRoutes(app: OpenAPIHono<AppBindings>) {
  // POST /api/v1/tasks/:taskId/server/start
  app.openapi(startServerRoute, async (c) => {
    const orchestrator = appContainer.get(PreviewOrchestratorService)
    const { taskId } = c.req.valid('param')

    // Start server in background
    await orchestrator.start(taskId)

    // Return 202 Accepted - server is starting in background
    return c.json({ success: true }, 202)
  })

  // POST /api/v1/tasks/:taskId/server/stop
  app.openapi(stopServerRoute, async (c) => {
    const orchestrator = appContainer.get(PreviewOrchestratorService)
    const { taskId } = c.req.valid('param')

    await orchestrator.stop(taskId)
    return c.json({
      stopped: true,
      runtime: 0, // TODO: Calculate actual runtime from snapshot timestamps
    })
  })

  // POST /api/v1/tasks/:taskId/server/restart
  app.openapi(restartServerRoute, async (c) => {
    const orchestrator = appContainer.get(PreviewOrchestratorService)
    const { taskId } = c.req.valid('param')

    await orchestrator.restart(taskId)
    return c.json({ success: true }, 202)
  })

  // GET /api/v1/tasks/:taskId/server/status
  app.openapi(getServerStatusRoute, async (c) => {
    const orchestrator = appContainer.get(PreviewOrchestratorService)
    const { taskId } = c.req.valid('param')

    const status = await orchestrator.getStatus(taskId)
    return c.json(status)
  })

  // GET /api/v1/tasks/:taskId/server/logs
  // Note: This is a non-OpenAPI endpoint for NDJSON streaming
  app.get(SERVER_LOGS_PATH, (c) => {
    const orchestrator = appContainer.get(PreviewOrchestratorService)
    const taskId = c.req.param('taskId')

    if (!taskId) {
      return c.json({ error: 'Task ID is required' }, 400)
    }

    // Stream logs via orchestrator
    return streamSSE(c, orchestrator.streamLogs(taskId))
  })
}
