import type { OpenAPIHono } from '@hono/zod-openapi'
import {
  getPublishSettingsRoute,
  updatePublishSettingsRoute,
} from '@moatless/api-contract/settings'
import type { AppBindings } from '../../app.js'
import { appContainer } from '../../container/container.js'
import { SettingsService } from '../../services/settings.service.js'
import { handleServiceError } from '../../utils/error-handler.js'

export function registerPublishSettingsRoutes(app: OpenAPIHono<AppBindings>) {
  // GET /api/v1/settings/publish
  app.openapi(getPublishSettingsRoute, async (c) => {
    try {
      const settingsService = appContainer.get(SettingsService)
      const settings = await settingsService.getPublishSettingsResponse()
      return c.json(settings)
    } catch (error) {
      handleServiceError(error, 'Get Publish settings')
    }
  })

  // PUT /api/v1/settings/publish
  app.openapi(updatePublishSettingsRoute, async (c) => {
    try {
      const settingsService = appContainer.get(SettingsService)
      const update = c.req.valid('json')
      const settings = await settingsService.updatePublishSettingsSection(update)
      return c.json(settings)
    } catch (error) {
      handleServiceError(error, 'Update Publish settings')
    }
  })
}
