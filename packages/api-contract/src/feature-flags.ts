import { createRoute } from '@hono/zod-openapi'
import { FeatureFlagsResponseSchema } from '@moatless/types'

// GET /api/v1/feature-flags - Get all feature flags
export const getFeatureFlagsRoute = createRoute({
  method: 'get',
  path: '/api/v1/feature-flags',
  tags: ['Feature Flags'],
  description: 'Get all feature flags with their current enabled/disabled state',
  responses: {
    200: {
      description: 'Success',
      content: {
        'application/json': {
          schema: FeatureFlagsResponseSchema,
        },
      },
    },
  },
})
