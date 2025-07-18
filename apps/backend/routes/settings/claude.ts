import {
  type ClaudeSettingsResponse,
  ClaudeSettingsResponseSchema,
  type UpdateClaudeSettingsRequest,
  updateClaudeSettingsSchema,
} from '@moatless/types'
import type { Hono } from 'hono'
import { describeRoute } from 'hono-openapi'
import { resolver } from 'hono-openapi/zod'
import type { AppBindings } from '../../app.js'
import { handleServiceError } from '../../utils/error-handler.js'
import { validateRequestBody } from '../../utils/route-helpers.js'

export function registerClaudeSettingsRoutes(app: Hono<AppBindings>) {
  // GET settings/claude - Get Claude settings
  app.get(
    '/',
    describeRoute({
      description: 'Get Claude configuration settings',
      responses: {
        200: {
          description: 'Claude settings and defaults',
          content: {
            'application/json': {
              schema: resolver(ClaudeSettingsResponseSchema) as any,
            },
          },
        },
        500: {
          description: 'Failed to get Claude settings',
        },
      },
    }),
    async (c) => {
      try {
        const services = c.get('services')
        const { settings: settingsService } = services
        const settings: ClaudeSettingsResponse = await settingsService.getClaudeSettingsResponse()
        return c.json(settings)
      } catch (error) {
        handleServiceError(error, 'Get Claude settings')
      }
    },
  )

  // PUT settings/claude - Update Claude settings
  app.put(
    '/',
    describeRoute({
      description: 'Update Claude configuration settings',
      requestBody: {
        description: 'Claude settings update parameters',
        content: {
          'application/json': {
            schema: resolver(updateClaudeSettingsSchema) as any,
          },
        },
      },
      responses: {
        200: {
          description: 'Updated Claude settings',
          content: {
            'application/json': {
              schema: resolver(ClaudeSettingsResponseSchema) as any,
            },
          },
        },
        400: {
          description: 'Invalid request body',
        },
        500: {
          description: 'Failed to update Claude settings',
        },
      },
    }),
    async (c) => {
      try {
        const services = c.get('services')
        const { settings: settingsService } = services
        const update = await validateRequestBody(c, updateClaudeSettingsSchema)
        const settings: ClaudeSettingsResponse = await settingsService.updateClaudeSettingsSection(
          update as UpdateClaudeSettingsRequest,
        )
        return c.json(settings)
      } catch (error) {
        handleServiceError(error, 'Update Claude settings')
      }
    },
  )
}
