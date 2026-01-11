import { createRoute } from '@hono/zod-openapi'
import { CommandRequestSchema, CommandResponseSchema } from '@moatless/sandbox-schemas'
import { SANDBOX_ENDPOINTS } from '../endpoints'

export const executeCommandRoute = createRoute({
  method: 'post',
  path: SANDBOX_ENDPOINTS.COMMAND,
  tags: ['Command'],
  description: 'Execute a shell command in the sandbox workspace',
  request: {
    body: {
      content: {
        'application/json': {
          schema: CommandRequestSchema,
        },
      },
    },
  },
  responses: {
    200: {
      description: 'Command executed',
      content: {
        'application/json': {
          schema: CommandResponseSchema,
        },
      },
    },
    500: {
      description: 'Command execution failed',
      content: {
        'application/json': {
          schema: CommandResponseSchema,
        },
      },
    },
  },
})
