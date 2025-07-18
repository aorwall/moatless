import {
  type GeneralSettingsResponse,
  GeneralSettingsResponseSchema,
  type UpdateGeneralSettingsRequest,
  updateGeneralSettingsSchema,
} from '@moatless/types'
import type { Hono } from 'hono'
import { describeRoute } from 'hono-openapi'
import { resolver } from 'hono-openapi/zod'
import type { AppBindings } from '../../app.js'
import { handleServiceError } from '../../utils/error-handler.js'
import { validateRequestBody } from '../../utils/route-helpers.js'

export function registerGeneralSettingsRoutes(app: Hono<AppBindings>) {
  // GET settings/general - Get general settings
  app.get(
    '/',
    describeRoute({
      description: 'Get general application settings',
      responses: {
        200: {
          description: 'General settings and defaults',
          content: {
            'application/json': {
              schema: resolver(GeneralSettingsResponseSchema) as any,
            },
          },
        },
        500: {
          description: 'Failed to get general settings',
        },
      },
    }),
    async (c) => {
      try {
        const services = c.get('services')
        const { settings: settingsService } = services
        const settings: GeneralSettingsResponse = await settingsService.getGeneralSettings()
        return c.json(settings)
      } catch (error) {
        handleServiceError(error, 'Get general settings')
      }
    },
  )

  // PUT settings/general - Update general settings
  app.put(
    '/',
    describeRoute({
      description: 'Update general application settings',
      requestBody: {
        description: 'General settings update parameters',
        content: {
          'application/json': {
            schema: resolver(updateGeneralSettingsSchema) as any,
          },
        },
      },
      responses: {
        200: {
          description: 'Updated general settings',
          content: {
            'application/json': {
              schema: resolver(GeneralSettingsResponseSchema) as any,
            },
          },
        },
        400: {
          description: 'Invalid request body',
        },
        500: {
          description: 'Failed to update general settings',
        },
      },
    }),
    async (c) => {
      try {
        const services = c.get('services')
        const { settings: settingsService } = services
        const update = await validateRequestBody(c, updateGeneralSettingsSchema)
        const settings: GeneralSettingsResponse = await settingsService.updateGeneralSettings(
          update as UpdateGeneralSettingsRequest,
        )
        return c.json(settings)
      } catch (error) {
        handleServiceError(error, 'Update general settings')
      }
    },
  )
}
