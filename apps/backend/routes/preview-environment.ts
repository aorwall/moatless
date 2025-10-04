import type { ServerStatus } from '@moatless/types'
import type { Hono } from 'hono'
import { HTTPException } from 'hono/http-exception'
import type { AppBindings } from '../app.js'

export async function registerPreviewEnvironmentRoutes(app: Hono<AppBindings>, prefix: string) {
  // Start development server
  app.post(`${prefix}/:taskId/server/start`, async (c) => {
    const services = c.get('services')
    const taskId = c.req.param('taskId')
    if (!taskId) {
      throw new HTTPException(400, { message: 'Task ID is required' })
    }

    // Task validation and capability check now handled in workspace service
    const workspace = await services.workspace.getOrCreateWorkspace(taskId)
    const result: ServerStatus = await workspace.startServer()
    return c.json(result)
  })

  // Stop development server
  app.post(`${prefix}/:taskId/server/stop`, async (c) => {
    const services = c.get('services')
    const taskId = c.req.param('taskId')
    if (!taskId) {
      throw new HTTPException(400, { message: 'Task ID is required' })
    }

    // Task validation and capability check now handled in workspace service
    const workspace = await services.workspace.getOrCreateWorkspace(taskId)
    const stopped = await workspace.stopServer()
    return c.json({
      stopped,
      runtime: 0, // TODO: Calculate actual runtime
    })
  })

  // Restart development server
  app.post(`${prefix}/:taskId/server/restart`, async (c) => {
    const services = c.get('services')
    const taskId = c.req.param('taskId')
    if (!taskId) {
      throw new HTTPException(400, { message: 'Task ID is required' })
    }

    // Task validation and capability check now handled in workspace service
    const workspace = await services.workspace.getOrCreateWorkspace(taskId)
    await workspace.stopServer()
    await new Promise((resolve) => setTimeout(resolve, 1000))
    const result = await workspace.startServer()
    return c.json(result)
  })

  // Get server status
  app.get(`${prefix}/:taskId/server/status`, async (c) => {
    const services = c.get('services')
    const taskId = c.req.param('taskId')
    if (!taskId) {
      throw new HTTPException(400, { message: 'Task ID is required' })
    }

    // Task validation and capability check now handled in workspace service
    const workspace = await services.workspace.getOrCreateWorkspace(taskId)
    const status = await workspace.getServerStatus()
    return c.json(status)
  })
}
