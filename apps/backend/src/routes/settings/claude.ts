import type { OpenAPIHono } from '@hono/zod-openapi'
import { getClaudeSettingsRoute, updateClaudeSettingsRoute } from '@moatless/api-contract/settings'
import type { AppBindings } from '../../app.js'
import { appContainer } from '../../container/container.js'
import { SettingsService } from '../../services/settings.service.js'
import { handleServiceError } from '../../utils/error-handler.js'

export function registerClaudeSettingsRoutes(app: OpenAPIHono<AppBindings>) {
  // GET /api/v1/settings/claude
  app.openapi(getClaudeSettingsRoute, async (c) => {
    try {
      const settingsService = appContainer.get(SettingsService)
      const settings = await settingsService.getClaudeSettingsResponse()
      return c.json(settings)
    } catch (error) {
      handleServiceError(error, 'Get Claude settings')
    }
  })

  // PUT /api/v1/settings/claude
  app.openapi(updateClaudeSettingsRoute, async (c) => {
    try {
      const settingsService = appContainer.get(SettingsService)
      const update = c.req.valid('json')
      const settings = await settingsService.updateClaudeSettingsSection(update)
      return c.json(settings)
    } catch (error) {
      handleServiceError(error, 'Update Claude settings')
    }
  })
}
