import { createRoute } from '@hono/zod-openapi'
import {
  AgentRunRequestSchema,
  AgentStatusResponseSchema,
  ExecuteAgentResponseSchema,
  StopAgentResponseSchema,
} from '@moatless/sandbox-schemas'
import { SANDBOX_ENDPOINTS } from '../endpoints'

export const executeAgentRoute = createRoute({
  method: 'post',
  path: SANDBOX_ENDPOINTS.EXECUTE_AGENT,
  tags: ['Agent'],
  description:
    'Start agent execution in the sandbox. Accepts AgentRunRequest with content array and options. Returns immediately and runs agent in background.',
  request: {
    body: {
      content: {
        'application/json': {
          schema: AgentRunRequestSchema,
        },
      },
    },
  },
  responses: {
    200: {
      description: 'Agent execution started',
      content: {
        'application/json': {
          schema: ExecuteAgentResponseSchema,
        },
      },
    },
    500: {
      description: 'Failed to start agent',
      content: {
        'application/json': {
          schema: ExecuteAgentResponseSchema,
        },
      },
    },
  },
})

export const stopAgentRoute = createRoute({
  method: 'post',
  path: SANDBOX_ENDPOINTS.STOP_AGENT,
  tags: ['Agent'],
  description: 'Stop the currently running agent',
  responses: {
    200: {
      description: 'Agent stopped',
      content: {
        'application/json': {
          schema: StopAgentResponseSchema,
        },
      },
    },
    500: {
      description: 'Failed to stop agent',
      content: {
        'application/json': {
          schema: StopAgentResponseSchema,
        },
      },
    },
  },
})

export const agentStatusRoute = createRoute({
  method: 'get',
  path: SANDBOX_ENDPOINTS.AGENT_STATUS,
  tags: ['Agent'],
  description: 'Get the current agent execution status',
  responses: {
    200: {
      description: 'Agent status',
      content: {
        'application/json': {
          schema: AgentStatusResponseSchema,
        },
      },
    },
  },
})
