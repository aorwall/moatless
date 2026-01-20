import type { OpenAPIHono } from '@hono/zod-openapi'
import {
  BROWSER_SCREENSHOT_FILE_PATH,
  deleteBrowserScreenshotsRoute,
  getBrowserScreenshotsRoute,
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

  // GET /api/v1/tasks/:taskId/browser/screenshots - List screenshots
  app.openapi(getBrowserScreenshotsRoute, async (c) => {
    const orchestrator = appContainer.get(PreviewOrchestratorService)
    const { taskId } = c.req.valid('param')

    const result = await orchestrator.getBrowserScreenshots(taskId)
    return c.json(result)
  })

  // GET /api/v1/tasks/:taskId/browser/screenshots/:filename - Get screenshot image
  // Note: Non-OpenAPI route for binary response
  app.get(BROWSER_SCREENSHOT_FILE_PATH, async (c) => {
    const orchestrator = appContainer.get(PreviewOrchestratorService)
    const taskId = c.req.param('taskId')
    const filename = c.req.param('filename')

    if (!(taskId && filename)) {
      return c.json({ error: 'Task ID and filename are required' }, 400)
    }

    const buffer = await orchestrator.getBrowserScreenshot(taskId, filename)
    if (!buffer) {
      return c.json({ error: 'Screenshot not found' }, 404)
    }

    return new Response(new Uint8Array(buffer), {
      headers: {
        'Content-Type': 'image/png',
        'Cache-Control': 'public, max-age=31536000, immutable',
      },
    })
  })

  // DELETE /api/v1/tasks/:taskId/browser/screenshots - Clear screenshots
  app.openapi(deleteBrowserScreenshotsRoute, async (c) => {
    const orchestrator = appContainer.get(PreviewOrchestratorService)
    const { taskId } = c.req.valid('param')

    const result = await orchestrator.clearBrowserScreenshots(taskId)
    return c.json(result)
  })
}
