import type { OpenAPIHono } from '@hono/zod-openapi'
import { getSystemStatusRoute } from '@moatless/api-contract/settings'
import type { AppBindings } from '../app.js'
import { handleServiceError } from '../utils/error-handler.js'
import { registerClaudeSettingsRoutes } from './settings/claude.js'
import { registerGeneralSettingsRoutes } from './settings/general.js'
import { registerGitHubSettingsRoutes } from './settings/github.js'
import { registerMcpSettingsRoutes } from './settings/mcp.js'
import { registerPublishSettingsRoutes } from './settings/publish.js'

export function registerSettingsRoutes(app: OpenAPIHono<AppBindings>) {
  // Register section-specific routes directly on the main app
  registerGeneralSettingsRoutes(app)
  registerClaudeSettingsRoutes(app)
  registerGitHubSettingsRoutes(app)
  registerMcpSettingsRoutes(app)
  registerPublishSettingsRoutes(app)

  /**
   * GET /api/v1/settings/status
   * Get system status including Claude and GitHub configuration validation
   */
  app.openapi(getSystemStatusRoute, async (c) => {
    try {
      const services = c.get('services')
      const { settings: settingsService } = services
      const status = await settingsService.getSystemStatus()
      return c.json(status)
    } catch (error) {
      handleServiceError(error, 'Get system status')
    }
  })
}
