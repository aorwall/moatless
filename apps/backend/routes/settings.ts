import { Hono } from 'hono'
import type { AppBindings } from '../app.js'
import { handleServiceError } from '../utils/error-handler.js'
import { registerClaudeSettingsRoutes } from './settings/claude.js'
import { registerGeneralSettingsRoutes } from './settings/general.js'
import { registerGitHubSettingsRoutes } from './settings/github.js'
import { registerPublishSettingsRoutes } from './settings/publish.js'

export async function registerSettingsRoutes(app: Hono<AppBindings>, prefix: string) {
  // Create sub-routers for each settings section
  const generalRouter = new Hono<AppBindings>()
  const claudeRouter = new Hono<AppBindings>()
  const githubRouter = new Hono<AppBindings>()
  const publishRouter = new Hono<AppBindings>()

  // Register section-specific routes
  registerGeneralSettingsRoutes(generalRouter)
  registerClaudeSettingsRoutes(claudeRouter)
  registerGitHubSettingsRoutes(githubRouter)
  registerPublishSettingsRoutes(publishRouter)

  // Mount sub-routers
  app.route(`${prefix}/general`, generalRouter)
  app.route(`${prefix}/claude`, claudeRouter)
  app.route(`${prefix}/github`, githubRouter)
  app.route(`${prefix}/publish`, publishRouter)

  // GET settings/status - Get system status
  app.get(`${prefix}/status`, async (c) => {
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
