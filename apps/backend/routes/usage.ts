import type {
  ClaudeSessionUsageResponse,
  ClaudeUsageResponse,
  RepositoryConfig,
} from '@moatless/types'
import {
  ClaudeSessionUsageResponseSchema,
  ClaudeUsageQuerySchema,
  ClaudeUsageResponseSchema,
} from '@moatless/types'
import type { Hono } from 'hono'
import { describeRoute } from 'hono-openapi'
import { resolver } from 'hono-openapi/zod'
import { z } from 'zod'
import type { AppBindings } from '../app.js'
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
        {
          name: 'repositoryId',
          in: 'query',
          description: 'Repository ID for docker environment path resolution',
          required: false,
          schema: resolver(ClaudeUsageQuerySchema.shape.repositoryId),
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

        // Get repository context if repositoryId is provided
        let repository: RepositoryConfig | undefined
        if (validatedQuery.repositoryId) {
          const services = c.get('services')
          const { repository: repositoryService } = services
          const foundRepository = await repositoryService.getRepositoryById(
            validatedQuery.repositoryId,
          )

          if (!foundRepository) {
            return c.json({ error: 'Repository not found' }, 404)
          }

          repository = foundRepository
        }

        const services = c.get('services')
        const { claude } = services
        const usageData: ClaudeUsageResponse = await claude.getUsage(
          validatedQuery.plan,
          repository,
        )

        return c.json(usageData)
      } catch (error) {
        handleServiceError(error, 'Get Claude usage data')
      }
    },
  )

  // GET /api/v1/usage/session/:sessionId - Get usage data for a specific session
  app.get(
    `${prefix}/session/:sessionId`,
    describeRoute({
      description: 'Get Claude usage data for a specific session',
      parameters: [
        {
          name: 'sessionId',
          in: 'path',
          description: 'The session ID to get usage data for',
          required: true,
          schema: resolver(z.string()),
        },
        {
          name: 'repositoryId',
          in: 'query',
          description: 'Repository ID for docker environment path resolution',
          required: false,
          schema: resolver(z.string()),
        },
      ],
      responses: {
        200: {
          description: 'Session usage data',
          content: {
            'application/json': {
              schema: resolver(ClaudeSessionUsageResponseSchema) as any,
            },
          },
        },
        404: {
          description: 'Session not found',
        },
        500: {
          description: 'Failed to get session usage data',
        },
      },
    }),
    async (c) => {
      try {
        const sessionId = c.req.param('sessionId')

        if (!sessionId) {
          return c.json({ error: 'Session ID is required' }, 400)
        }

        // Note: repositoryId parameter is no longer used since we search for session files globally

        const services = c.get('services')
        const { claude } = services
        const sessionUsage: ClaudeSessionUsageResponse = await claude.getSessionUsage(sessionId)

        // Check if session was found (has entries)
        if (sessionUsage.messageCount === 0 && !sessionUsage.startTime) {
          return c.json({ error: 'Session not found' }, 404)
        }

        return c.json(sessionUsage)
      } catch (error) {
        handleServiceError(error, 'Get session usage data')
      }
    },
  )
}
