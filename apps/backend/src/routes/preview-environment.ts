import type { OpenAPIHono } from '@hono/zod-openapi'
import {
  getServerStatusRoute,
  restartServerRoute,
  startServerRoute,
  stopServerRoute,
} from '@moatless/api-contract/preview'
import type { ServerStatus } from '@moatless/types'
import type { AppBindings } from '../app.js'

export async function registerPreviewEnvironmentRoutes(app: OpenAPIHono<AppBindings>) {
  // Start development server

  app.openapi(startServerRoute, async (c) => {
    const services = c.get('services')

    const { taskId } = c.req.valid('param')

    // Task validation and capability check now handled in workspace service
    const workspace = await services.workspace.getOrCreateWorkspace(taskId)
    const result: ServerStatus = await workspace.startServer()
    return c.json(result)
  })

  // Stop development server

  app.openapi(stopServerRoute, async (c) => {
    const services = c.get('services')

    const { taskId } = c.req.valid('param')

    // Task validation and capability check now handled in workspace service
    const workspace = await services.workspace.getOrCreateWorkspace(taskId)
    const stopped = await workspace.stopServer()
    return c.json({
      stopped,
      runtime: 0, // TODO: Calculate actual runtime
    })
  })

  // Restart development server

  app.openapi(restartServerRoute, async (c) => {
    const services = c.get('services')

    const { taskId } = c.req.valid('param')

    // Task validation and capability check now handled in workspace service
    const workspace = await services.workspace.getOrCreateWorkspace(taskId)
    await workspace.stopServer()
    await new Promise((resolve) => setTimeout(resolve, 1000))
    const result = await workspace.startServer()
    return c.json(result)
  })

  // Get server status

  app.openapi(getServerStatusRoute, async (c) => {
    const services = c.get('services')

    const { taskId } = c.req.valid('param')

    // Task validation and capability check now handled in workspace service
    const workspace = await services.workspace.getOrCreateWorkspace(taskId)
    const status = await workspace.getServerStatus()
    return c.json(status)
  })
}
