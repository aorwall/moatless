import { createRoute } from '@hono/zod-openapi'
import {
  ServerLogsQuerySchema,
  ServerLogsResponseExtendedSchema,
  ServerStatusSchema,
  ServerStopResponseSchema,
} from '@moatless/types'
import { pathParams } from './params.js'

export const startServerRoute = createRoute({
  method: 'post',
  path: '/api/v1/tasks/{taskId}/server/start',
  tags: ['Preview Server'],
  description: 'Start the development server for a task workspace to preview the application',
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
  description: 'Restart the development server for a task workspace (stops, waits 1s, then starts)',
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

export const getServerLogsRoute = createRoute({
  method: 'get',
  path: '/api/v1/tasks/{taskId}/server/logs',
  tags: ['Preview Server'],
  description: 'Retrieve development server logs for a task with optional line limit',
  request: {
    params: pathParams.taskId,
    query: ServerLogsQuerySchema,
  },
  responses: {
    200: {
      description: 'Success',
      content: {
        'application/json': {
          schema: ServerLogsResponseExtendedSchema,
        },
      },
    },
  },
})
