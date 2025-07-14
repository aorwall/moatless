import type { ClaudeUsageResponse } from '@moatless/types'
import { ClaudeUsageQuerySchema, ClaudeUsageResponseSchema } from '@moatless/types'
import type { Hono } from 'hono'
import { describeRoute } from 'hono-openapi'
import { resolver } from 'hono-openapi/zod'
import type { AppBindings } from '../app.js'
import { getClaudeUsageService } from '../services/claude-usage.service.js'
import { handleServiceError } from '../utils/error-handler.js'

export async function registerUsageRoutes(app: Hono<AppBindings>, prefix: string) {
  // GET /api/v1/usage - Get Claude usage data
  app.get(
    prefix,
    describeRoute({
      description: 'Get Claude usage data including cost, tokens, and limits',
      parameters: [
        {
          name: 'plan',
          in: 'query',
          description: 'The user plan for determining limits',
          required: false,
          schema: resolver(ClaudeUsageQuerySchema.shape.plan),
        },
      ],
      responses: {
        200: {
          description: 'Claude usage data',
          content: {
            'application/json': {
              schema: resolver(ClaudeUsageResponseSchema) as any,
            },
          },
        },
        500: {
          description: 'Failed to get usage data',
        },
      },
    }),
    async (c) => {
      try {
        const query = c.req.query()
        const validatedQuery = ClaudeUsageQuerySchema.parse(query)

        const usageService = getClaudeUsageService()
        const usageData: ClaudeUsageResponse = await usageService.getUsage(validatedQuery.plan)

        return c.json(usageData)
      } catch (error) {
        handleServiceError(error, 'Get Claude usage data')
      }
    },
  )
}
