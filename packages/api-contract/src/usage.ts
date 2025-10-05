import { createRoute } from '@hono/zod-openapi'
import {
  ClaudeSessionUsageResponseSchema,
  ClaudeUsageQuerySchema,
  ClaudeUsageResponseSchema,
} from '@moatless/types'
import { pathParams } from './params.js'

// GET /api/v1/usage - Get Claude usage data
export const getUsageRoute = createRoute({
  method: 'get',
  path: '/api/v1/usage',
  tags: ['Usage'],
  description: 'Retrieve Claude API usage statistics including costs, tokens, and message counts',
  request: {
    query: ClaudeUsageQuerySchema,
  },
  responses: {
    200: {
      description: 'Success',
      content: {
        'application/json': {
          schema: ClaudeUsageResponseSchema,
        },
      },
    },
  },
})

// GET /api/v1/usage/session/{sessionId} - Get session usage data
export const getSessionUsageRoute = createRoute({
  method: 'get',
  path: '/api/v1/usage/session/{sessionId}',
  tags: ['Usage'],
  description: 'Retrieve detailed usage statistics for a specific Claude API session',
  request: {
    params: pathParams.sessionId,
  },
  responses: {
    200: {
      description: 'Success',
      content: {
        'application/json': {
          schema: ClaudeSessionUsageResponseSchema,
        },
      },
    },
  },
})
