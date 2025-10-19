import { createRoute } from '@hono/zod-openapi'
import { TemplateListResponseSchema } from '@moatless/types'

// ============================================================================
// Template Routes
// ============================================================================

/**
 * GET /api/v1/templates
 * List all available repository templates
 */
export const listTemplatesRoute = createRoute({
  method: 'get',
  path: '/api/v1/templates',
  tags: ['Templates'],
  description: 'List all available repository templates for creating vibe tasks',
  responses: {
    200: {
      description: 'Success',
      content: {
        'application/json': {
          schema: TemplateListResponseSchema,
        },
      },
    },
  },
})
