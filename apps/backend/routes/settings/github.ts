import type { OpenAPIHono } from '@hono/zod-openapi'
import { getGitHubSettingsRoute, updateGitHubSettingsRoute } from '@moatless/api-contract/settings'
import type { AppBindings } from '../../app.js'
import { handleServiceError } from '../../utils/error-handler.js'

export function registerGitHubSettingsRoutes(app: OpenAPIHono<AppBindings>) {
  // GET /api/v1/settings/github
  app.openapi(getGitHubSettingsRoute, async (c) => {
    try {
      const services = c.get('services')
      const { settings: settingsService } = services
      const settings = await settingsService.getGitHubSettingsResponse()
      return c.json(settings)
    } catch (error) {
      handleServiceError(error, 'Get GitHub settings')
    }
  })

  // PUT /api/v1/settings/github
  app.openapi(updateGitHubSettingsRoute, async (c) => {
    try {
      const services = c.get('services')
      const { settings: settingsService } = services
      const update = c.req.valid('json')
      const settings = await settingsService.updateGitHubSettingsSection(update)
      return c.json(settings)
    } catch (error) {
      handleServiceError(error, 'Update GitHub settings')
    }
  })
}
