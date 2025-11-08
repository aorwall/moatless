import type { OpenAPIHono } from '@hono/zod-openapi'
import { getMcpSettingsRoute, updateMcpSettingsRoute } from '@moatless/api-contract/settings'
import type { AppBindings } from '../../../app'
import { appContainer } from '../../../container/container'
import { handleServiceError } from '../../../utils/error-handler'
import { McpSettingsService } from '../services/mcp-settings.service'

export function registerMcpSettingsRoutes(app: OpenAPIHono<AppBindings>) {
  // GET /api/v1/settings/mcp
  app.openapi(getMcpSettingsRoute, async (c) => {
    try {
      const mcpSettingsService = appContainer.get(McpSettingsService)
      const settings = await mcpSettingsService.getMcpSettingsResponse()
      return c.json(settings)
    } catch (error) {
      handleServiceError(error, 'Get MCP settings')
    }
  })

  // PUT /api/v1/settings/mcp
  app.openapi(updateMcpSettingsRoute, async (c) => {
    try {
      const mcpSettingsService = appContainer.get(McpSettingsService)
      const update = c.req.valid('json')
      const settings = await mcpSettingsService.updateMcpSettingsSection(update)
      return c.json(settings)
    } catch (error) {
      handleServiceError(error, 'Update MCP settings')
    }
  })
}
