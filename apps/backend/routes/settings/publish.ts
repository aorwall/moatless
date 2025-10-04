import {
  type PublishSettingsResponse,
  type UpdatePublishSettingsRequest,
  UpdatePublishSettingsRequestSchema,
} from '@moatless/types'
import type { Hono } from 'hono'
import type { AppBindings } from '../../app.js'
import { handleServiceError } from '../../utils/error-handler.js'
import { validateRequestBody } from '../../utils/route-helpers.js'

export function registerPublishSettingsRoutes(app: Hono<AppBindings>) {
  // GET settings/publish - Get Publish settings
  app.get('/', async (c) => {
    try {
      const services = c.get('services')
      const { settings: settingsService } = services
      const settings: PublishSettingsResponse = await settingsService.getPublishSettingsResponse()
      return c.json(settings)
    } catch (error) {
      handleServiceError(error, 'Get Publish settings')
    }
  })

  // PUT settings/publish - Update Publish settings
  app.put('/', async (c) => {
    try {
      const services = c.get('services')
      const { settings: settingsService } = services
      const update = await validateRequestBody(c, UpdatePublishSettingsRequestSchema)
      const settings: PublishSettingsResponse = await settingsService.updatePublishSettingsSection(
        update as UpdatePublishSettingsRequest,
      )
      return c.json(settings)
    } catch (error) {
      handleServiceError(error, 'Update Publish settings')
    }
  })
}
