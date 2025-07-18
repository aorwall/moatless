import {
  type GitHubSettingsResponse,
  GitHubSettingsResponseSchema,
  type UpdateGitHubSettingsRequest,
  updateGitHubSettingsSchema,
} from '@moatless/types'
import type { Hono } from 'hono'
import { describeRoute } from 'hono-openapi'
import { resolver } from 'hono-openapi/zod'
import type { AppBindings } from '../../app.js'
import { handleServiceError } from '../../utils/error-handler.js'
import { validateRequestBody } from '../../utils/route-helpers.js'

export function registerGitHubSettingsRoutes(app: Hono<AppBindings>) {
  // GET settings/github - Get GitHub settings
  app.get(
    '/',
    describeRoute({
      description: 'Get GitHub configuration settings',
      responses: {
        200: {
          description: 'GitHub settings and defaults',
          content: {
            'application/json': {
              schema: resolver(GitHubSettingsResponseSchema) as any,
            },
          },
        },
        500: {
          description: 'Failed to get GitHub settings',
        },
      },
    }),
    async (c) => {
      try {
        const services = c.get('services')
        const { settings: settingsService } = services
        const settings: GitHubSettingsResponse = await settingsService.getGitHubSettingsResponse()
        return c.json(settings)
      } catch (error) {
        handleServiceError(error, 'Get GitHub settings')
      }
    },
  )

  // PUT settings/github - Update GitHub settings
  app.put(
    '/',
    describeRoute({
      description: 'Update GitHub configuration settings',
      requestBody: {
        description: 'GitHub settings update parameters',
        content: {
          'application/json': {
            schema: resolver(updateGitHubSettingsSchema) as any,
          },
        },
      },
      responses: {
        200: {
          description: 'Updated GitHub settings',
          content: {
            'application/json': {
              schema: resolver(GitHubSettingsResponseSchema) as any,
            },
          },
        },
        400: {
          description: 'Invalid request body',
        },
        500: {
          description: 'Failed to update GitHub settings',
        },
      },
    }),
    async (c) => {
      try {
        const services = c.get('services')
        const { settings: settingsService } = services
        const update = await validateRequestBody(c, updateGitHubSettingsSchema)
        const settings: GitHubSettingsResponse = await settingsService.updateGitHubSettingsSection(
          update as UpdateGitHubSettingsRequest,
        )
        return c.json(settings)
      } catch (error) {
        handleServiceError(error, 'Update GitHub settings')
      }
    },
  )
}
