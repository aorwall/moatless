import { createRoute } from '@hono/zod-openapi'
import { HealthResponseSchema } from '@moatless/sandbox-schemas'
import { SANDBOX_ENDPOINTS } from '../endpoints'

export const healthRoute = createRoute({
  method: 'get',
  path: SANDBOX_ENDPOINTS.HEALTH,
  tags: ['Health'],
  description: 'Health check endpoint for sandbox status',
  responses: {
    200: {
      description: 'Sandbox is healthy',
      content: {
        'application/json': {
          schema: HealthResponseSchema,
        },
      },
    },
  },
})
