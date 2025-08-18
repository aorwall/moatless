import {
  type PublishSettingsResponse,
  PublishSettingsResponseSchema,
  type UpdatePublishSettingsRequest,
  UpdatePublishSettingsRequestSchema,
} from '@moatless/types'
import type { Hono } from 'hono'
import { describeRoute } from 'hono-openapi'
import { resolver } from 'hono-openapi/zod'
import type { AppBindings } from '../../app.js'
import { handleServiceError } from '../../utils/error-handler.js'
import { validateRequestBody } from '../../utils/route-helpers.js'

export function registerPublishSettingsRoutes(app: Hono<AppBindings>) {
  // GET settings/publish - Get Publish settings
  app.get(
    '/',
    describeRoute({
      description: 'Get Publish configuration settings',
      responses: {
        200: {
          description: 'Publish settings and defaults',
          content: {
            'application/json': {
              schema: resolver(PublishSettingsResponseSchema) as any,
            },
          },
        },
        500: {
          description: 'Failed to get Publish settings',
        },
      },
    }),
    async (c) => {
      try {
        const services = c.get('services')
        const { settings: settingsService } = services
        const settings: PublishSettingsResponse = await settingsService.getPublishSettingsResponse()
        return c.json(settings)
      } catch (error) {
        handleServiceError(error, 'Get Publish settings')
      }
    },
  )

  // PUT settings/publish - Update Publish settings
  app.put(
    '/',
    describeRoute({
      description: 'Update Publish configuration settings',
      requestBody: {
        description: 'Publish settings update parameters',
        content: {
          'application/json': {
            schema: resolver(UpdatePublishSettingsRequestSchema) as any,
          },
        },
      },
      responses: {
        200: {
          description: 'Updated Publish settings',
          content: {
            'application/json': {
              schema: resolver(PublishSettingsResponseSchema) as any,
            },
          },
        },
        400: {
          description: 'Invalid request body',
        },
        500: {
          description: 'Failed to update Publish settings',
        },
      },
    }),
    async (c) => {
      try {
        const services = c.get('services')
        const { settings: settingsService } = services
        const update = await validateRequestBody(c, UpdatePublishSettingsRequestSchema)
        const settings: PublishSettingsResponse =
          await settingsService.updatePublishSettingsSection(update as UpdatePublishSettingsRequest)
        return c.json(settings)
      } catch (error) {
        handleServiceError(error, 'Update Publish settings')
      }
    },
  )
}
