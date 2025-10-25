import type { OpenAPIHono } from '@hono/zod-openapi'
import { getMcpSettingsRoute, updateMcpSettingsRoute } from '@moatless/api-contract/settings'
import type { AppBindings } from '../../app.js'
import { appContainer } from '../../container/container.js'
import { SettingsService } from '../../services/settings.service.js'
import { handleServiceError } from '../../utils/error-handler.js'

export function registerMcpSettingsRoutes(app: OpenAPIHono<AppBindings>) {
  // GET /api/v1/settings/mcp
  app.openapi(getMcpSettingsRoute, async (c) => {
    try {
      const settingsService = appContainer.get(SettingsService)
      const settings = await settingsService.getMcpSettingsResponse()
      return c.json(settings)
    } catch (error) {
      handleServiceError(error, 'Get MCP settings')
    }
  })

  // PUT /api/v1/settings/mcp
  app.openapi(updateMcpSettingsRoute, async (c) => {
    try {
      const settingsService = appContainer.get(SettingsService)
      const update = c.req.valid('json')
      const settings = await settingsService.updateMcpSettingsSection(update)
      return c.json(settings)
    } catch (error) {
      handleServiceError(error, 'Update MCP settings')
    }
  })
}
