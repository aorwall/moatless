import type { OpenAPIHono } from '@hono/zod-openapi'
import { getSystemStatusRoute } from '@moatless/api-contract/settings'
import type { AppBindings } from '../app'
import { appContainer } from '../container/container'
import { registerMcpSettingsRoutes } from '../features/agents/index'
import { registerGitHubSettingsRoutes, registerGitLabSettingsRoutes } from '../features/auth/index'
import { registerClaudeSettingsRoutes } from '../features/claude/index'
import { SettingsService } from '../services/settings.service'
import { handleServiceError } from '../utils/error-handler'
import { registerGeneralSettingsRoutes } from './settings/general'
import { registerPublishSettingsRoutes } from './settings/publish'

export function registerSettingsRoutes(app: OpenAPIHono<AppBindings>) {
  // Register section-specific routes directly on the main app
  registerGeneralSettingsRoutes(app)
  registerClaudeSettingsRoutes(app)
  registerGitHubSettingsRoutes(app)
  registerGitLabSettingsRoutes(app)
  registerMcpSettingsRoutes(app)
  registerPublishSettingsRoutes(app)

  /**
   * GET /api/v1/settings/status
   * Get system status including Claude and GitHub configuration validation
   */
  app.openapi(getSystemStatusRoute, async (c) => {
    try {
      const settingsService = appContainer.get(SettingsService)
      const status = await settingsService.getSystemStatus()
      return c.json(status)
    } catch (error) {
      handleServiceError(error, 'Get system status')
    }
  })
}
