import {
  type GeneralSettingsResponse,
  type UpdateGeneralSettingsRequest,
  updateGeneralSettingsSchema,
} from '@moatless/types'
import type { Hono } from 'hono'
import type { AppBindings } from '../../app.js'
import { handleServiceError } from '../../utils/error-handler.js'
import { validateRequestBody } from '../../utils/route-helpers.js'

export function registerGeneralSettingsRoutes(app: Hono<AppBindings>) {
  // GET settings/general - Get general settings
  app.get('/', async (c) => {
    try {
      const services = c.get('services')
      const { settings: settingsService } = services
      const settings: GeneralSettingsResponse = await settingsService.getGeneralSettings()
      return c.json(settings)
    } catch (error) {
      handleServiceError(error, 'Get general settings')
    }
  })

  // PUT settings/general - Update general settings
  app.put('/', async (c) => {
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
  })
}
