import {
  type SettingsResponse,
  SettingsResponseSchema,
  SystemStatusCheckSchema,
  type UpdateSettingsRequest,
  updateSettingsSchema,
} from '@moatless/types'
import type { Hono } from 'hono'
import { describeRoute } from 'hono-openapi'
import { resolver } from 'hono-openapi/zod'
import type { AppBindings } from '../app.js'
import { getSettingsService } from '../services/index.js'
import { handleServiceError } from '../utils/error-handler.js'
import { validateRequestBody } from '../utils/route-helpers.js'

export async function registerSettingsRoutes(app: Hono<AppBindings>, prefix: string) {
  // GET settings - Get current settings and defaults
  app.get(
    prefix,
    describeRoute({
      description: 'Get current application settings and defaults',
      responses: {
        200: {
          description: 'Current settings and defaults',
          content: {
            'application/json': {
              schema: resolver(SettingsResponseSchema) as any,
            },
          },
        },
        500: {
          description: 'Failed to get settings',
        },
      },
    }),
    async (c) => {
      try {
        const settingsService = getSettingsService()
        const settings: SettingsResponse = await settingsService.getSettings()
        return c.json(settings)
      } catch (error) {
        handleServiceError(error, 'Get settings')
      }
    },
  )

  // PUT settings - Update settings
  app.put(
    prefix,
    describeRoute({
      description: 'Update application settings',
      requestBody: {
        description: 'Settings update parameters',
        content: {
          'application/json': {
            schema: resolver(updateSettingsSchema) as any,
          },
        },
      },
      responses: {
        200: {
          description: 'Updated settings',
          content: {
            'application/json': {
              schema: resolver(SettingsResponseSchema) as any,
            },
          },
        },
        400: {
          description: 'Invalid request body',
        },
        500: {
          description: 'Failed to update settings',
        },
      },
    }),
    async (c) => {
      try {
        const settingsService = getSettingsService()
        const update = await validateRequestBody(c, updateSettingsSchema)
        const settings: SettingsResponse = await settingsService.updateSettings(
          update as UpdateSettingsRequest,
        )
        return c.json(settings)
      } catch (error) {
        handleServiceError(error, 'Update settings')
      }
    },
  )

  // DELETE settings - Reset settings to defaults
  app.delete(
    prefix,
    describeRoute({
      description: 'Reset application settings to defaults',
      responses: {
        200: {
          description: 'Reset settings',
          content: {
            'application/json': {
              schema: resolver(SettingsResponseSchema) as any,
            },
          },
        },
        500: {
          description: 'Failed to reset settings',
        },
      },
    }),
    async (c) => {
      try {
        const settingsService = getSettingsService()
        const settings: SettingsResponse = await settingsService.resetSettings()
        return c.json(settings)
      } catch (error) {
        handleServiceError(error, 'Reset settings')
      }
    },
  )

  // GET settings/status - Get system status
  app.get(
    `${prefix}/status`,
    describeRoute({
      description: 'Get system status including Claude and Docker availability',
      responses: {
        200: {
          description: 'System status information',
          content: {
            'application/json': {
              schema: resolver(SystemStatusCheckSchema) as any,
            },
          },
        },
        500: {
          description: 'Failed to get system status',
        },
      },
    }),
    async (c) => {
      try {
        const settingsService = getSettingsService()
        const status = await settingsService.getSystemStatus()
        return c.json(status)
      } catch (error) {
        handleServiceError(error, 'Get system status')
      }
    },
  )
}
