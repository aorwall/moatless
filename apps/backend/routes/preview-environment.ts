import type { OpenAPIHono } from '@hono/zod-openapi'
import { createRoute } from '@hono/zod-openapi'
import {
  ServerLogsQuerySchema,
  ServerLogsResponseExtendedSchema,
  type ServerStatus,
  ServerStatusSchema,
  ServerStopResponseSchema,
} from '@moatless/types'
import type { AppBindings } from '../app.js'
import { pathParams, withStandardErrors } from '../utils/openapi-helpers.js'

export async function registerPreviewEnvironmentRoutes(app: OpenAPIHono<AppBindings>) {
  // Start development server
  const startServerRoute = createRoute({
    method: 'post',
    path: '/api/v1/tasks/{taskId}/server/start',
    tags: ['Preview Server'],
    description: 'Start the development server for a task workspace to preview the application',
    request: { params: pathParams.taskId },
    responses: withStandardErrors(
      ServerStatusSchema,
      'Returns server status including running state, URL, proxy URL, PID, and uptime',
    ),
  })

  app.openapi(startServerRoute, async (c) => {
    const services = c.get('services')

    const { taskId } = c.req.valid('param')

    // Task validation and capability check now handled in workspace service
    const workspace = await services.workspace.getOrCreateWorkspace(taskId)
    const result: ServerStatus = await workspace.startServer()
    return c.json(result)
  })

  // Stop development server
  const stopServerRoute = createRoute({
    method: 'post',
    path: '/api/v1/tasks/{taskId}/server/stop',
    tags: ['Preview Server'],
    description: 'Stop the running development server for a task workspace',
    request: { params: pathParams.taskId },
    responses: withStandardErrors(
      ServerStopResponseSchema,
      'Returns stopped status and runtime duration',
    ),
  })

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
  const restartServerRoute = createRoute({
    method: 'post',
    path: '/api/v1/tasks/{taskId}/server/restart',
    tags: ['Preview Server'],
    description:
      'Restart the development server for a task workspace (stops, waits 1s, then starts)',
    request: { params: pathParams.taskId },
    responses: withStandardErrors(
      ServerStatusSchema,
      'Returns updated server status after restart including URL and running state',
    ),
  })

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
  const getServerStatusRoute = createRoute({
    method: 'get',
    path: '/api/v1/tasks/{taskId}/server/status',
    tags: ['Preview Server'],
    description: 'Get current status of the development server for a task workspace',
    request: { params: pathParams.taskId },
    responses: withStandardErrors(
      ServerStatusSchema,
      'Returns current server status including running state, URL, PID, uptime, and any errors',
    ),
  })

  app.openapi(getServerStatusRoute, async (c) => {
    const services = c.get('services')

    const { taskId } = c.req.valid('param')

    // Task validation and capability check now handled in workspace service
    const workspace = await services.workspace.getOrCreateWorkspace(taskId)
    const status = await workspace.getServerStatus()
    return c.json(status)
  })

  // Get server logs
  const getServerLogsRoute = createRoute({
    method: 'get',
    path: '/api/v1/tasks/{taskId}/server/logs',
    tags: ['Preview Server'],
    description: 'Retrieve development server logs for a task with optional line limit',
    request: {
      params: pathParams.taskId,
      query: ServerLogsQuerySchema,
    },
    responses: withStandardErrors(
      ServerLogsResponseExtendedSchema,
      'Returns server logs with line count and truncation status',
    ),
  })

  app.openapi(getServerLogsRoute, async (c) => {
    const services = c.get('services')

    const { taskId } = c.req.valid('param')
    const { lines } = c.req.valid('query')

    // Task validation and capability check now handled in workspace service
    const workspace = await services.workspace.getOrCreateWorkspace(taskId)
    const logs = await workspace.getServerLogs(lines)
    return c.json({
      logs,
      lines: logs.split('\n').length,
      truncated: false, // TODO: Implement truncation logic
    })
  })
}
