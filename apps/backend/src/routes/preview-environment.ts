import type { OpenAPIHono } from '@hono/zod-openapi'
import {
  getServerStatusRoute,
  restartServerRoute,
  startServerRoute,
  stopServerRoute,
} from '@moatless/api-contract/preview'
import type { ServerStatus } from '@moatless/types'
import type { AppBindings } from '../app.js'
import { appContainer } from '../container/container.js'
import { RepositoryService } from '../features/repositories/services/repository.service.js'
import { WorkspaceService } from '../services/workspace.service.js'
import { streamSSE } from '../utils/sse-stream.js'

export function registerPreviewEnvironmentRoutes(app: OpenAPIHono<AppBindings>) {
  // Start development server

  app.openapi(startServerRoute, async (c) => {
    const workspaceService = appContainer.get(WorkspaceService)

    const { taskId } = c.req.valid('param')

    // Task validation and capability check now handled in workspace service
    const workspace = await workspaceService.getOrCreateWorkspace(taskId)
    const result: ServerStatus = await workspace.startServer()
    return c.json(result)
  })

  // Stop development server

  app.openapi(stopServerRoute, async (c) => {
    const workspaceService = appContainer.get(WorkspaceService)

    const { taskId } = c.req.valid('param')

    // Task validation and capability check now handled in workspace service
    const workspace = await workspaceService.getOrCreateWorkspace(taskId)
    const stopped = await workspace.stopServer()
    return c.json({
      stopped,
      runtime: 0, // TODO: Calculate actual runtime
    })
  })

  // Restart development server

  app.openapi(restartServerRoute, async (c) => {
    const workspaceService = appContainer.get(WorkspaceService)
    const repositoryService = appContainer.get(RepositoryService)

    const { taskId } = c.req.valid('param')

    // Task validation and capability check now handled in workspace service
    const workspace = await workspaceService.getOrCreateWorkspace(taskId)

    // Reload repository config to pick up any changes
    const latestRepo = await repositoryService.getRepositoryById(workspace.repositoryId)
    if (latestRepo) {
      workspace.updateRepositoryConfig(latestRepo)
    }

    await workspace.stopServer()
    await new Promise((resolve) => setTimeout(resolve, 1000))
    const result = await workspace.startServer()
    return c.json(result)
  })

  // Get server status

  app.openapi(getServerStatusRoute, async (c) => {
    const workspaceService = appContainer.get(WorkspaceService)

    const { taskId } = c.req.valid('param')

    // Task validation and capability check now handled in workspace service
    const workspace = await workspaceService.getOrCreateWorkspace(taskId)
    const status = await workspace.getServerStatus()
    return c.json(status)
  })

  // Stream server logs via SSE
  // Note: This is a non-OpenAPI endpoint because SSE doesn't fit well in OpenAPI specs
  app.get('/api/v1/tasks/:taskId/server/logs/stream', async (c) => {
    const workspaceService = appContainer.get(WorkspaceService)
    const taskId = c.req.param('taskId')

    if (!taskId) {
      return c.json({ error: 'Task ID is required' }, 400)
    }

    // Get workspace and stream logs
    const workspace = await workspaceService.getOrCreateWorkspace(taskId)
    return streamSSE(c, workspace.streamServerLogs())
  })
}
