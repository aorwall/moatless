import { createRoute } from '@hono/zod-openapi'
import {
  ServerLogsQuerySchema,
  ServerLogsResponseExtendedSchema,
  ServerStatusSchema,
  ServerStopResponseSchema,
} from '@moatless/types'
import { pathParams } from '../params.js'
import { withStandardErrors } from '../responses.js'

export const startServerRoute = createRoute({
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

export const stopServerRoute = createRoute({
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

export const restartServerRoute = createRoute({
  method: 'post',
  path: '/api/v1/tasks/{taskId}/server/restart',
  tags: ['Preview Server'],
  description: 'Restart the development server for a task workspace (stops, waits 1s, then starts)',
  request: { params: pathParams.taskId },
  responses: withStandardErrors(
    ServerStatusSchema,
    'Returns updated server status after restart including URL and running state',
  ),
})

export const getServerStatusRoute = createRoute({
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

export const getServerLogsRoute = createRoute({
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
