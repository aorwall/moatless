import type { OpenAPIHono } from '@hono/zod-openapi'
import { getServerStatusRoute, SERVER_LOGS_PATH } from '@moatless/api-contract/preview'
import type { AppBindings } from '../../../app.js'
import { appContainer } from '../../../container/container.js'
import { streamSSE } from '../../../utils/sse-stream.js'
import { PreviewOrchestratorService } from '../services/preview-orchestrator.service.js'

/**
 * Register preview server routes
 *
 * These routes use PreviewOrchestratorService which:
 * - Delegates status/logs to environment drivers (reads from sandbox)
 * - Provides browser screenshot proxy
 *
 * Note: Start/stop/restart are handled by the Claude Code agent via
 * /opt/moatless/bin/preview-server script directly in the sandbox.
 */
export function registerPreviewRoutes(app: OpenAPIHono<AppBindings>) {
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
