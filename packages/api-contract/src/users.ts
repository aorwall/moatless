import { createRoute } from '@hono/zod-openapi'
import { UserListResponseSchema } from '@moatless/types'

/**
 * GET /api/v1/users
 * List all users (summary only)
 */
export const listUsersRoute = createRoute({
  method: 'get',
  path: '/api/v1/users',
  tags: ['Users'],
  description: 'List all users with summary information',
  responses: {
    200: {
      description: 'Success',
      content: {
        'application/json': {
          schema: UserListResponseSchema,
        },
      },
    },
  },
})
