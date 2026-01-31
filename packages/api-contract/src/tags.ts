import { createRoute } from '@hono/zod-openapi'
import { TagListResponseSchema } from '@moatless/types'

/**
 * GET /api/v1/tags
 * List all tags
 */
export const listTagsRoute = createRoute({
  method: 'get',
  path: '/api/v1/tags',
  tags: ['Tags'],
  description: 'List all tags for task categorization',
  responses: {
    200: {
      description: 'Success',
      content: {
        'application/json': {
          schema: TagListResponseSchema,
        },
      },
    },
  },
})
