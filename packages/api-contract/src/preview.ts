import { createRoute } from '@hono/zod-openapi'
import { ServerStatusSchema, ServerStopResponseSchema } from '@moatless/types'
import { z } from 'zod'
import { pathParams } from './params.js'

export const startServerRoute = createRoute({
  method: 'post',
  path: '/api/v1/tasks/{taskId}/server/start',
  tags: ['Preview Server'],
  description:
    'Start the development server for a task workspace. Returns 202 (Accepted) immediately. The server starts in the background and emits SSE events for status updates.',
  request: { params: pathParams.taskId },
  responses: {
    202: {
      description: 'Accepted - Server starting in background',
      content: {
        'application/json': {
          schema: z.object({ success: z.literal(true) }),
        },
      },
    },
  },
})

export const stopServerRoute = createRoute({
  method: 'post',
  path: '/api/v1/tasks/{taskId}/server/stop',
  tags: ['Preview Server'],
  description: 'Stop the running development server for a task workspace',
  request: { params: pathParams.taskId },
  responses: {
    200: {
      description: 'Success',
      content: {
        'application/json': {
          schema: ServerStopResponseSchema,
        },
      },
    },
  },
})

export const restartServerRoute = createRoute({
  method: 'post',
  path: '/api/v1/tasks/{taskId}/server/restart',
  tags: ['Preview Server'],
  description:
    'Restart the development server for a task workspace. Returns 202 (Accepted) immediately. The server restarts in the background and emits SSE events for status updates.',
  request: { params: pathParams.taskId },
  responses: {
    202: {
      description: 'Accepted - Server restarting in background',
      content: {
        'application/json': {
          schema: z.object({ success: z.literal(true) }),
        },
      },
    },
  },
})

export const getServerStatusRoute = createRoute({
  method: 'get',
  path: '/api/v1/tasks/{taskId}/server/status',
  tags: ['Preview Server'],
  description: 'Get current status of the development server for a task workspace',
  request: { params: pathParams.taskId },
  responses: {
    200: {
      description: 'Success',
      content: {
        'application/json': {
          schema: ServerStatusSchema,
        },
      },
    },
  },
})

/**
 * Server logs streaming endpoint path
 * Note: This is a non-OpenAPI endpoint for NDJSON streaming
 */
export const SERVER_LOGS_PATH = '/api/v1/tasks/:taskId/server/logs'
