import { createRoute } from '@hono/zod-openapi'
import {
  ServerLogsResponseSchema,
  ServerStatusResponseSchema,
  StartServerRequestSchema,
  StartServerResponseSchema,
  StopServerResponseSchema,
} from '@moatless/sandbox-schemas'
import { z } from 'zod'
import { SANDBOX_ENDPOINTS } from '../endpoints'

export const startServerRoute = createRoute({
  method: 'post',
  path: SANDBOX_ENDPOINTS.SERVER_START,
  tags: ['Server'],
  description: 'Start the preview server in the sandbox',
  request: {
    body: {
      content: {
        'application/json': {
          schema: StartServerRequestSchema,
        },
      },
    },
  },
  responses: {
    200: {
      description: 'Server started',
      content: {
        'application/json': {
          schema: StartServerResponseSchema,
        },
      },
    },
    400: {
      description: 'Invalid configuration',
      content: {
        'application/json': {
          schema: StartServerResponseSchema,
        },
      },
    },
    500: {
      description: 'Failed to start server',
      content: {
        'application/json': {
          schema: StartServerResponseSchema,
        },
      },
    },
  },
})

export const stopServerRoute = createRoute({
  method: 'post',
  path: SANDBOX_ENDPOINTS.SERVER_STOP,
  tags: ['Server'],
  description: 'Stop the preview server',
  responses: {
    200: {
      description: 'Server stopped',
      content: {
        'application/json': {
          schema: StopServerResponseSchema,
        },
      },
    },
    500: {
      description: 'Failed to stop server',
      content: {
        'application/json': {
          schema: StopServerResponseSchema,
        },
      },
    },
  },
})

export const serverStatusRoute = createRoute({
  method: 'get',
  path: SANDBOX_ENDPOINTS.SERVER_STATUS,
  tags: ['Server'],
  description: 'Get preview server status',
  responses: {
    200: {
      description: 'Server status',
      content: {
        'application/json': {
          schema: ServerStatusResponseSchema,
        },
      },
    },
    500: {
      description: 'Failed to get server status',
      content: {
        'application/json': {
          schema: ServerStatusResponseSchema,
        },
      },
    },
  },
})

export const serverLogsRoute = createRoute({
  method: 'get',
  path: SANDBOX_ENDPOINTS.SERVER_LOGS,
  tags: ['Server'],
  description: 'Get preview server logs',
  request: {
    query: z.object({
      lines: z.coerce.number().optional(),
    }),
  },
  responses: {
    200: {
      description: 'Server logs',
      content: {
        'application/json': {
          schema: ServerLogsResponseSchema,
        },
      },
    },
    500: {
      description: 'Failed to get server logs',
      content: {
        'application/json': {
          schema: ServerLogsResponseSchema,
        },
      },
    },
  },
})
