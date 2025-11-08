import type { OpenAPIHono } from '@hono/zod-openapi'
import { getClaudeSettingsRoute, updateClaudeSettingsRoute } from '@moatless/api-contract/settings'
import type { AppBindings } from '../../../app'
import { appContainer } from '../../../container/container'
import { handleServiceError } from '../../../utils/error-handler'
import { ClaudeSettingsService } from '../services/claude-settings.service'

export function registerClaudeSettingsRoutes(app: OpenAPIHono<AppBindings>) {
  // GET /api/v1/settings/claude
  app.openapi(getClaudeSettingsRoute, async (c) => {
    try {
      const claudeSettingsService = appContainer.get(ClaudeSettingsService)
      const settings = await claudeSettingsService.getClaudeSettingsResponse()
      return c.json(settings)
    } catch (error) {
      handleServiceError(error, 'Get Claude settings')
    }
  })

  // PUT /api/v1/settings/claude
  app.openapi(updateClaudeSettingsRoute, async (c) => {
    try {
      const claudeSettingsService = appContainer.get(ClaudeSettingsService)
      const update = c.req.valid('json')
      const settings = await claudeSettingsService.updateClaudeSettingsSection(update)
      return c.json(settings)
    } catch (error) {
      handleServiceError(error, 'Update Claude settings')
    }
  })
}
