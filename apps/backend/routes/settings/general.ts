import type { OpenAPIHono } from '@hono/zod-openapi'
import {
  getGeneralSettingsRoute,
  updateGeneralSettingsRoute,
} from '@moatless/api-contract/settings'
import type { AppBindings } from '../../app.js'
import { handleServiceError } from '../../utils/error-handler.js'

export function registerGeneralSettingsRoutes(app: OpenAPIHono<AppBindings>) {
  // GET /api/v1/settings/general
  app.openapi(getGeneralSettingsRoute, async (c) => {
    try {
      const services = c.get('services')
      const { settings: settingsService } = services
      const settings = await settingsService.getGeneralSettings()
      return c.json(settings)
    } catch (error) {
      handleServiceError(error, 'Get general settings')
    }
  })

  // PUT /api/v1/settings/general
  app.openapi(updateGeneralSettingsRoute, async (c) => {
    try {
      const services = c.get('services')
      const { settings: settingsService } = services
      const update = c.req.valid('json')
      const settings = await settingsService.updateGeneralSettings(update)
      return c.json(settings)
    } catch (error) {
      handleServiceError(error, 'Update general settings')
    }
  })
}
